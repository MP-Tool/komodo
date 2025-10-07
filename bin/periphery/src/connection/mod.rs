use std::{
  fs::read_to_string,
  sync::{Arc, OnceLock},
  time::Duration,
};

use anyhow::{Context as _, anyhow};
use arc_swap::ArcSwap;
use bytes::Bytes;
use cache::CloneCache;
use noise::key::SpkiPublicKey;
use resolver_api::Resolve;
use response::JsonBytes;
use transport::{
  auth::{
    ConnectionIdentifiers, LoginFlow, LoginFlowArgs,
    PublicKeyValidator,
  },
  channel::{BufferedChannel, BufferedReceiver, Sender},
  message::{Message, MessageState},
  websocket::{Websocket, WebsocketReceiver, WebsocketSender as _},
};
use uuid::Uuid;

use crate::{
  api::{Args, PeripheryRequest},
  config::{periphery_config, periphery_private_key},
};

pub mod client;
pub mod server;

// Core Address / Host -> Channel
pub type CoreChannels = CloneCache<String, Arc<BufferedChannel>>;

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
    let Some(core_public_keys) =
      periphery_config().core_public_keys.as_ref()
    else {
      return;
    };
    let core_public_keys = core_public_keys
      .iter()
      .flat_map(|public_key| {
        let maybe_pem =
          if let Some(path) = public_key.strip_prefix("file:") {
            read_to_string(path)
              .with_context(|| {
                format!("Failed to read public key at {path:?}")
              })
              .inspect_err(|e| warn!("{e:#}"))
              .ok()?
          } else {
            public_key.clone()
          };
        SpkiPublicKey::from_maybe_pem(&maybe_pem)
          .inspect_err(|e| warn!("{e:#}"))
          .ok()
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
    private_key: periphery_private_key().load().as_str(),
    public_key_validator: core_public_keys(),
  })
  .await
}

async fn handle_socket<W: Websocket>(
  socket: W,
  args: &Arc<Args>,
  sender: &Sender,
  receiver: &mut BufferedReceiver,
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
      match ws_write.send(message).await {
        // Clears the stored message from receiver buffer.
        Ok(_) => receiver.clear_buffer(),
        Err(e) => {
          warn!("Failed to send response | {e:?}");
          let _ = ws_write.close(None).await;
          break;
        }
      }
    }
  };

  let handle_reads = async {
    loop {
      let (data, channel, state) = match ws_read.recv_parts().await {
        Ok(res) => res,
        Err(e) => {
          warn!("{e:#}");
          break;
        }
      };
      match state {
        MessageState::Request => {
          handle_request(args.clone(), sender.clone(), channel, data)
        }
        MessageState::Terminal => {
          crate::terminal::handle_message(channel, data).await
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
  sender: Sender,
  req_id: Uuid,
  request: Bytes,
) {
  tokio::spawn(async move {
    let request =
      match serde_json::from_slice::<PeripheryRequest>(&request) {
        Ok(req) => req,
        Err(e) => {
          // TODO: handle:
          warn!("Failed to parse transport bytes | {e:#}");
          return;
        }
      };

    let resolve_response = async {
      let message: Message = match request.resolve(&args).await {
        Ok(JsonBytes::Ok(res)) => {
          (res, req_id, MessageState::Successful).into()
        }
        Ok(JsonBytes::Err(e)) => (&e.into(), req_id).into(),
        Err(e) => (&e.error, req_id).into(),
      };
      if let Err(e) = sender.send(message).await {
        error!("Failed to send response over channel | {e:?}");
      }
    };

    let ping_in_progress = async {
      loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        if let Err(e) =
          sender.send((req_id, MessageState::InProgress)).await
        {
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
