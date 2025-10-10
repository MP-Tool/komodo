use std::{
  sync::{Arc, OnceLock},
  time::Duration,
};

use anyhow::anyhow;
use arc_swap::ArcSwap;
use cache::CloneCache;
use encoding::{
  CastBytes as _, Decode as _, Encode as _, EncodedChannel,
  EncodedJsonMessage,
};
use noise::key::SpkiPublicKey;
use periphery_client::transport::{EncodedTransportMessage, TransportMessage};
use resolver_api::Resolve;
use transport::{
  auth::{
    ConnectionIdentifiers, LoginFlow, LoginFlowArgs,
    PublicKeyValidator,
  },
  channel::{BufferedChannel, BufferedReceiver, Sender},
  websocket::{
    Websocket, WebsocketReceiverExt as _, WebsocketSender as _,
  },
};

use crate::{
  api::{Args, PeripheryRequest},
  config::{periphery_config, periphery_keys},
};

pub mod client;
pub mod server;

// Core Address / Host -> Channel
pub type CoreChannels =
  CloneCache<String, Arc<BufferedChannel<EncodedTransportMessage>>>;

pub fn core_channels() -> &'static CoreChannels {
  static CORE_CHANNELS: OnceLock<CoreChannels> = OnceLock::new();
  CORE_CHANNELS.get_or_init(Default::default)
}

pub fn core_public_keys() -> &'static CorePublicKeys {
  static CORE_PUBLIC_KEYS: OnceLock<CorePublicKeys> = OnceLock::new();
  CORE_PUBLIC_KEYS.get_or_init(CorePublicKeys::default)
}

pub struct CorePublicKeys(ArcSwap<Vec<SpkiPublicKey>>);

impl Default for CorePublicKeys {
  fn default() -> Self {
    let keys = CorePublicKeys(Default::default());
    keys.refresh();
    keys
  }
}

impl CorePublicKeys {
  pub fn is_valid(&self, public_key: &str) -> bool {
    let keys = self.0.load();
    keys.is_empty() || keys.iter().any(|pk| pk.as_str() == public_key)
  }

  pub fn refresh(&self) {
    let config = periphery_config();
    let Some(core_public_keys) = config.core_public_keys.as_ref()
    else {
      return;
    };
    let core_public_keys = core_public_keys
      .iter()
      .flat_map(|public_key| {
        let res = if let Some(path) = public_key.strip_prefix("file:")
        {
          SpkiPublicKey::from_file(path)
        } else {
          SpkiPublicKey::from_maybe_pem(public_key)
        };
        match (res, config.server_enabled) {
          (Ok(public_key), _) => Some(public_key),
          (Err(e), false) => {
            // If only outbound connections, only warn.
            // It will be written the next time `RotateCoreKeys` is executed.
            warn!("{e:#}");
            None
          }
          (Err(e), true) => {
            // This is too dangerous to allow if server_enabled.
            panic!("{e:#}");
          }
        }
      })
      .collect::<Vec<_>>();
    self.0.store(Arc::new(core_public_keys));
  }
}

impl PublicKeyValidator for &CorePublicKeys {
  type ValidationResult = ();
  async fn validate(&self, public_key: String) -> anyhow::Result<()> {
    let keys = self.0.load();
    if keys.is_empty()
      || keys.iter().any(|pk| pk.as_str() == public_key)
    {
      Ok(())
    } else {
      Err(
        anyhow!("{public_key} is invalid")
          .context("Ensure public key matches one of the 'core_public_keys' in periphery config (PERIPHERY_CORE_PUBLIC_KEYS)")
          .context("Periphery failed to validate Core public key"),
      )
    }
  }
}

async fn handle_login<W: Websocket, L: LoginFlow>(
  socket: &mut W,
  identifiers: ConnectionIdentifiers<'_>,
) -> anyhow::Result<()> {
  L::login(LoginFlowArgs {
    socket,
    identifiers,
    private_key: periphery_keys().load().private.as_str(),
    public_key_validator: core_public_keys(),
  })
  .await
}

async fn handle_socket<W: Websocket>(
  socket: W,
  args: &Arc<Args>,
  sender: &Sender<EncodedTransportMessage>,
  receiver: &mut BufferedReceiver<EncodedTransportMessage>,
) {
  let config = periphery_config();
  info!(
    "Logged in to Komodo Core {} websocket{}",
    args.core,
    if !config.core_addresses.is_empty()
      && !config.connect_as.is_empty()
    {
      format!(" as Server {}", config.connect_as)
    } else {
      String::new()
    }
  );

  let (mut ws_write, mut ws_read) = socket.split();

  let forward_writes = async {
    loop {
      let message = match receiver.recv().await {
        Ok(message) => message,
        Err(e) => {
          warn!("{e:#}");
          break;
        }
      };
      match ws_write.send_inner(message.into_bytes()).await {
        // Clears the stored message from receiver buffer.
        Ok(_) => receiver.clear_buffer(),
        Err(e) => {
          warn!("Failed to send response | {e:?}");
          let _ = ws_write.close().await;
          break;
        }
      }
    }
  };

  let handle_reads = async {
    loop {
      let message = match ws_read.recv().await {
        Ok(res) => res,
        Err(e) => {
          warn!("{e:#}");
          break;
        }
      };
      match message {
        TransportMessage::Request(message) => {
          handle_request(args.clone(), sender.clone(), message.0)
        }
        TransportMessage::Terminal(message) => {
          crate::terminal::handle_message(message.0).await
        }
        // Rest shouldn't be received by Periphery
        _ => {}
      }
    }
  };

  tokio::select! {
    _ = forward_writes => {},
    _ = handle_reads => {},
  }
}

fn handle_request(
  args: Arc<Args>,
  sender: Sender<EncodedTransportMessage>,
  message: EncodedChannel<EncodedJsonMessage>,
) {
  tokio::spawn(async move {
    let (channel, request): (_, PeripheryRequest) = match message
      .decode()
      .and_then(|res| Ok((res.channel, res.data.decode()?)))
    {
      Ok(res) => res,
      Err(e) => {
        // TODO: handle:
        warn!("Failed to parse Request bytes | {e:#}");
        return;
      }
    };

    let resolve_response = async {
      let response = match request.resolve(&args).await {
        Ok(res) => res,
        Err(e) => (&e).encode(),
      };
      if let Err(e) = sender.send_response(channel, response).await {
        error!("Failed to send response over channel | {e:?}");
      }
    };

    let ping_in_progress = async {
      loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        if let Err(e) = sender.send_in_progress(channel).await {
          error!("Failed to ping in progress over channel | {e:?}");
        }
      }
    };

    tokio::select! {
      _ = resolve_response => {},
      _ = ping_in_progress => {},
    }
  });
}
