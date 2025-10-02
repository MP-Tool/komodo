use std::{
  sync::{
    Arc,
    atomic::{self, AtomicBool},
  },
  time::Duration,
};

use anyhow::anyhow;
use bytes::Bytes;
use cache::CloneCache;
use komodo_client::entities::optional_str;
use serror::serror_into_anyhow_error;
use tokio::sync::{
  RwLock,
  mpsc::{Sender, error::SendError},
};
use tokio_util::sync::CancellationToken;
use transport::{
  auth::{ConnectionIdentifiers, LoginFlow, PublicKeyValidator},
  bytes::id_from_transport_bytes,
  channel::{BufferedReceiver, buffered_channel},
  websocket::{
    Websocket, WebsocketMessage, WebsocketReceiver as _,
    WebsocketSender as _,
  },
};

use crate::{
  config::{core_private_key, periphery_public_keys},
  periphery::ConnectionChannels,
};

pub mod client;
pub mod server;

pub enum PeripheryPublicKeyValidator<'a> {
  /// When Core connects to Periphery, it is mainly
  /// Periphery authenticating the connection from Core.
  /// If user doesn't pin a public key in config, the
  /// Periphery public key is not validated.
  CoreToPeriphery(Option<&'a str>),
  /// When Periphery connects to Core, its public key
  /// must be validated, otherwise any random Periphery
  /// can connect. If one isn't provided here,
  /// will fallback to the default accepted 'periphery_public_keys'
  /// in the Core config.
  PeripheryToCore(Option<&'a str>),
}

impl PeripheryPublicKeyValidator<'_> {
  pub fn new<'a>(
    address: &str,
    configured_public_key: &'a str,
  ) -> PeripheryPublicKeyValidator<'a> {
    let public_key = optional_str(configured_public_key);
    if address.is_empty() {
      PeripheryPublicKeyValidator::PeripheryToCore(public_key)
    } else {
      PeripheryPublicKeyValidator::CoreToPeriphery(public_key)
    }
  }
}

impl PublicKeyValidator for PeripheryPublicKeyValidator<'_> {
  fn validate(&self, public_key: String) -> anyhow::Result<()> {
    use PeripheryPublicKeyValidator::*;
    let invalid_error = || {
      anyhow!("Got invalid public key: {public_key}")
        .context(
          "Ensure public key matches configured Periphery Public Key",
        )
        .context("Core failed to validate Periphery public key")
    };
    match self {
      // No public key validation
      CoreToPeriphery(None) => Ok(()),
      // Core config public key authentication
      PeripheryToCore(None) => {
        let expected =
          periphery_public_keys().ok_or_else(invalid_error)?;
        if expected
          .iter()
          .any(|expected| &public_key == expected.as_str())
        {
          Ok(())
        } else {
          Err(invalid_error())
        }
      }
      // Validate keys explicitly set in Server config.
      CoreToPeriphery(Some(expected))
      | PeripheryToCore(Some(expected)) => {
        if &public_key == expected {
          Ok(())
        } else {
          Err(invalid_error())
        }
      }
    }
  }
}

#[derive(Default)]
pub struct PeripheryConnections(
  CloneCache<String, Arc<PeripheryConnection>>,
);

impl PeripheryConnections {
  pub async fn insert(
    &self,
    server_id: String,
    args: PeripheryConnectionArgs<'_>,
  ) -> (Arc<PeripheryConnection>, BufferedReceiver<Bytes>) {
    let channels = if let Some(existing_connection) =
      self.0.remove(&server_id).await
    {
      existing_connection.cancel();
      // Keep the same channels so requests
      // can handle disconnects while processing.
      existing_connection.channels.clone()
    } else {
      Default::default()
    };

    let (connection, receiver) =
      PeripheryConnection::new(args, channels);

    self.0.insert(server_id, connection.clone()).await;

    (connection, receiver)
  }

  pub async fn get(
    &self,
    server_id: &String,
  ) -> Option<Arc<PeripheryConnection>> {
    self.0.get(server_id).await
  }

  /// Remove and cancel connection
  pub async fn remove(
    &self,
    server_id: &String,
  ) -> Option<Arc<PeripheryConnection>> {
    self
      .0
      .remove(server_id)
      .await
      .inspect(|connection| connection.cancel())
  }
}

/// The configurable args of a connection
#[derive(Clone, Copy)]
pub struct PeripheryConnectionArgs<'a> {
  pub address: &'a str,
  pub core_private_key: &'a str,
  pub periphery_public_key: &'a str,
}

impl PeripheryConnectionArgs<'_> {
  pub fn matches(&self, connection: &PeripheryConnection) -> bool {
    self.address == connection.address
      && self.core_private_key == connection.core_private_key
      && self.periphery_public_key == connection.periphery_public_key
  }
}

#[derive(Debug)]
pub struct PeripheryConnection {
  /// Specify outbound connection address.
  /// Inbound connections have this as empty string
  pub address: String,
  /// The private key to use, or empty for core private key
  pub core_private_key: String,
  /// The public key to expect Periphery to have.
  /// Required non-empty for inbound connection.
  pub periphery_public_key: String,
  /// Whether Periphery is currently connected.
  pub connected: AtomicBool,
  /// Stores latest connection error
  pub error: RwLock<Option<serror::Serror>>,
  /// Cancel the connection
  pub cancel: CancellationToken,
  /// Send bytes to Periphery
  pub sender: Sender<Bytes>,
  /// Send bytes from Periphery to channel handlers.
  /// Must be maintained if new connection replaces old
  /// at the same server id.
  pub channels: Arc<ConnectionChannels>,
}

impl PeripheryConnection {
  pub fn new(
    args: PeripheryConnectionArgs<'_>,
    channels: Arc<ConnectionChannels>,
  ) -> (Arc<PeripheryConnection>, BufferedReceiver<Bytes>) {
    let (sender, receiver) = buffered_channel();
    (
      PeripheryConnection {
        address: args.address.to_string(),
        core_private_key: args.core_private_key.to_string(),
        periphery_public_key: args.periphery_public_key.to_string(),
        sender,
        channels,
        connected: AtomicBool::new(false),
        error: RwLock::new(None),
        cancel: CancellationToken::new(),
      }
      .into(),
      receiver,
    )
  }

  pub async fn handle_login<W: Websocket, L: LoginFlow>(
    &self,
    socket: &mut W,
    identifiers: ConnectionIdentifiers<'_>,
  ) -> anyhow::Result<()> {
    let core_private_key = if let Some(private_key) =
      optional_str(&self.core_private_key)
    {
      private_key
    } else {
      core_private_key()
    };

    L::login(
      socket,
      identifiers,
      core_private_key,
      &PeripheryPublicKeyValidator::new(
        &self.address,
        &self.periphery_public_key,
      ),
    )
    .await
  }

  pub async fn handle_socket<W: Websocket>(
    &self,
    socket: W,
    receiver: &mut BufferedReceiver<Bytes>,
  ) {
    let handler_cancel = CancellationToken::new();

    self.set_connected(true);
    self.clear_error().await;

    let (mut ws_write, mut ws_read) = socket.split();

    let forward_writes = async {
      loop {
        let next = tokio::select! {
          next = receiver.recv() => next,
          _ = self.cancel.cancelled() => break,
          _ = handler_cancel.cancelled() => break,
        };

        let message = match next {
          Some(request) => Bytes::copy_from_slice(request),
          // Sender Dropped (shouldn't happen, a reference is held on 'connection').
          None => break,
        };

        match ws_write.send(message).await {
          Ok(_) => receiver.clear_buffer(),
          Err(e) => {
            self.set_error(e.into()).await;
            break;
          }
        }
      }
      // Cancel again if not already
      let _ = ws_write.close(None).await;
      handler_cancel.cancel();
    };

    let handle_reads = async {
      loop {
        let next = tokio::select! {
          next = ws_read.recv() => next,
          _ = self.cancel.cancelled() => break,
          _ = handler_cancel.cancelled() => break,
        };

        match next {
          Ok(WebsocketMessage::Binary(bytes)) => {
            self.handle_incoming_bytes(bytes).await
          }
          Ok(WebsocketMessage::Close(_))
          | Ok(WebsocketMessage::Closed) => {
            self.set_error(anyhow!("Connection closed")).await;
            break;
          }
          Err(e) => {
            self.set_error(e.into()).await;
          }
        };
      }
      // Cancel again if not already
      handler_cancel.cancel();
    };

    tokio::join!(forward_writes, handle_reads);

    self.set_connected(false);
  }

  pub async fn handle_incoming_bytes(&self, bytes: Bytes) {
    let id = match id_from_transport_bytes(&bytes) {
      Ok(res) => res,
      Err(e) => {
        // TODO: handle better
        warn!("Failed to read id | {e:#}");
        return;
      }
    };
    let Some(channel) = self.channels.get(&id).await else {
      // TODO: handle better
      debug!("Failed to send response | No response channel found");
      return;
    };
    if let Err(e) = channel.send(bytes).await {
      // TODO: handle better
      warn!("Failed to send response | Channel failure | {e:#}");
    }
  }

  pub async fn send(
    &self,
    value: Bytes,
  ) -> Result<(), SendError<Bytes>> {
    self.sender.send(value).await
  }

  pub fn set_connected(&self, connected: bool) {
    self.connected.store(connected, atomic::Ordering::Relaxed);
  }

  pub fn connected(&self) -> bool {
    self.connected.load(atomic::Ordering::Relaxed)
  }

  /// Polls connected 3 times (500ms in between) before bailing.
  pub async fn bail_if_not_connected(&self) -> anyhow::Result<()> {
    const POLL_TIMES: usize = 3;
    for i in 0..POLL_TIMES {
      if self.connected() {
        return Ok(());
      }
      if i < POLL_TIMES - 1 {
        tokio::time::sleep(Duration::from_millis(500)).await;
      }
    }
    if let Some(e) = self.error().await {
      Err(serror_into_anyhow_error(e))
    } else {
      Err(anyhow!("Server is not currently connected"))
    }
  }

  pub async fn error(&self) -> Option<serror::Serror> {
    self.error.read().await.clone()
  }

  pub async fn set_error(&self, e: anyhow::Error) {
    let mut error = self.error.write().await;
    *error = Some(e.into());
  }

  pub async fn clear_error(&self) {
    let mut error = self.error.write().await;
    *error = None;
  }

  pub fn cancel(&self) {
    self.cancel.cancel();
  }
}
