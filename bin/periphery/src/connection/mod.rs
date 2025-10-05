use std::{
  sync::{Arc, OnceLock},
  time::Duration,
};

use anyhow::anyhow;
use bytes::Bytes;
use cache::CloneCache;
use resolver_api::Resolve;
use response::JsonBytes;
use serror::serialize_error_bytes;
use tokio::sync::mpsc::Sender;
use transport::{
  MessageState,
  auth::{
    ConnectionIdentifiers, LoginFlow, LoginFlowArgs,
    PublicKeyValidator,
  },
  bytes::{
    data_from_transport_bytes, id_state_from_transport_bytes,
    to_transport_bytes,
  },
  channel::{BufferedChannel, BufferedReceiver},
  websocket::{
    Websocket, WebsocketMessage, WebsocketReceiver,
    WebsocketSender as _,
  },
};
use uuid::Uuid;

use crate::{
  api::{Args, PeripheryRequest},
  config::{
    core_public_keys, periphery_config, periphery_private_key,
  },
};

pub mod client;
pub mod server;

// Core Address / Host -> Channel
pub type CoreChannels =
  CloneCache<String, Arc<BufferedChannel<Bytes>>>;

pub fn core_channels() -> &'static CoreChannels {
  static CORE_CHANNELS: OnceLock<CoreChannels> = OnceLock::new();
  CORE_CHANNELS.get_or_init(Default::default)
}

pub struct CorePublicKeyValidator;

impl PublicKeyValidator for CorePublicKeyValidator {
  type ValidationResult = ();
  async fn validate(&self, public_key: String) -> anyhow::Result<()> {
    if let Some(public_keys) = core_public_keys()
      && public_keys
        .load()
        .iter()
        .all(|expected| public_key != expected.as_str())
    {
      Err(
        anyhow!("{public_key} is invalid")
          .context("Ensure public key matches one of the 'core_public_keys' in periphery config (PERIPHERY_CORE_PUBLIC_KEYS)")
          .context("Periphery failed to validate Core public key"),
      )
    } else {
      Ok(())
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
    public_key_validator: CorePublicKeyValidator,
  })
  .await
}

async fn handle_socket<W: Websocket>(
  socket: W,
  args: &Arc<Args>,
  sender: &Sender<Bytes>,
  receiver: &mut BufferedReceiver<Bytes>,
) {
  let config = periphery_config();
  info!(
    "Logged in to Komodo Core {} websocket{}",
    args.core,
    if config.core_addresses.is_some()
      && let Some(connect_as) = &config.connect_as
    {
      format!(" as Server {connect_as}")
    } else {
      String::new()
    }
  );

  let (mut ws_write, mut ws_read) = socket.split();

  let forward_writes = async {
    loop {
      let msg = match receiver.recv().await {
        // Sender Dropped (shouldn't happen, it is static).
        None => break,
        // This has to copy the bytes to follow ownership rules.
        Some(msg) => Bytes::copy_from_slice(msg),
      };
      match ws_write.send(msg).await {
        // Clears the stored message from receiver buffer.
        // TODO: Move after response ack.
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
      match ws_read.recv().await {
        Ok(WebsocketMessage::Binary(bytes)) => {
          handle_incoming_bytes(args, sender, bytes).await
        }
        Ok(WebsocketMessage::Close(frame)) => {
          warn!("Connection closed with frame: {frame:?}");
          break;
        }
        Ok(WebsocketMessage::Closed) => {
          warn!("Connection already closed");
          break;
        }
        Err(e) => {
          warn!("Failed to read websocket message | {e:?}");
          break;
        }
      };
    }
  };

  tokio::select! {
    _ = forward_writes => {},
    _ = handle_reads => {},
  }
}

async fn handle_incoming_bytes(
  args: &Arc<Args>,
  sender: &Sender<Bytes>,
  bytes: Bytes,
) {
  let (id, state) = match id_state_from_transport_bytes(&bytes) {
    Ok(res) => res,
    Err(e) => {
      warn!("Failed to parse transport bytes | {e:#}");
      return;
    }
  };
  match state {
    MessageState::Request => {
      handle_request(args.clone(), sender.clone(), id, bytes)
    }
    MessageState::Terminal => {
      crate::terminal::handle_incoming_message(id, bytes).await
    }
    // Shouldn't be received by Periphery
    MessageState::InProgress => {}
    MessageState::Successful => {}
    MessageState::Failed => {}
  }
}

fn handle_request(
  args: Arc<Args>,
  sender: Sender<Bytes>,
  req_id: Uuid,
  bytes: Bytes,
) {
  tokio::spawn(async move {
    let request = match data_from_transport_bytes(bytes) {
      Ok(req) if !req.is_empty() => req,
      _ => {
        return;
      }
    };

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
      let (state, data) = match request.resolve(&args).await {
        Ok(JsonBytes::Ok(res)) => (MessageState::Successful, res),
        Ok(JsonBytes::Err(e)) => (
          MessageState::Failed,
          serialize_error_bytes(
            &anyhow::Error::new(e)
              .context("Failed to serialize response body"),
          ),
        ),
        Err(e) => {
          (MessageState::Failed, serialize_error_bytes(&e.error))
        }
      };
      if let Err(e) =
        sender.send(to_transport_bytes(data, req_id, state)).await
      {
        error!("Failed to send response over channel | {e:?}");
      }
    };

    let ping_in_progress = async {
      loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        if let Err(e) = sender
          .send(to_transport_bytes(
            Vec::new(),
            req_id,
            MessageState::InProgress,
          ))
          .await
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
