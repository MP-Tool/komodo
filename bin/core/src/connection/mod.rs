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
use database::mungos::{by_id::update_one_by_id, mongodb::bson::doc};
use komodo_client::entities::{
  builder::{AwsBuilderConfig, UrlBuilderConfig},
  optional_str,
  server::Server,
};
use serror::serror_into_anyhow_error;
use tokio::sync::{
  RwLock,
  mpsc::{Sender, error::SendError},
};
use tokio_util::sync::CancellationToken;
use transport::{
  auth::{
    ConnectionIdentifiers, LoginFlow, LoginFlowArgs,
    PublicKeyValidator,
  },
  bytes::id_from_transport_bytes,
  channel::{BufferedReceiver, buffered_channel},
  websocket::{
    Websocket, WebsocketMessage, WebsocketReceiver as _,
    WebsocketSender as _,
  },
};

use crate::{
  config::core_private_key, periphery::ConnectionChannels,
  state::db_client,
};

pub mod client;
pub mod server;

#[derive(Default)]
pub struct PeripheryConnections(
  CloneCache<String, Arc<PeripheryConnection>>,
);

impl PeripheryConnections {
  /// Insert a recreated connection.
  /// Ensures the fields which must be persisted between
  /// connection recreation are carried over.
  pub async fn insert(
    &self,
    server_id: String,
    args: PeripheryConnectionArgs<'_>,
  ) -> (Arc<PeripheryConnection>, BufferedReceiver<Bytes>) {
    let (connection, receiver) = if let Some(existing_connection) =
      self.0.remove(&server_id).await
    {
      existing_connection.with_new_args(args)
    } else {
      PeripheryConnection::new(args)
    };

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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PeripheryConnectionArgs<'a> {
  /// Usually the server id
  pub id: &'a str,
  pub address: Option<&'a str>,
  periphery_public_key: Option<&'a str>,
}

impl PublicKeyValidator for PeripheryConnectionArgs<'_> {
  type ValidationResult = String;
  async fn validate(
    &self,
    public_key: String,
  ) -> anyhow::Result<Self::ValidationResult> {
    let core_to_periphery = self.address.is_some();
    match (self.periphery_public_key, core_to_periphery) {
      // The key matches expected.
      (Some(expected), _) if public_key == expected => Ok(public_key),
      // Core -> Periphery connections with no explicit
      // Periphery public key are not validated.
      (None, true) => Ok(public_key),
      // Auth failed.
      (Some(_), _) | (None, false) => {
        spawn_update_attempted_public_key(
          self.id.to_string(),
          Some(public_key.clone()),
        );
        let e = anyhow!("{public_key} is invalid")
          .context(
            "Ensure public key matches configured Periphery Public Key",
          )
          .context("Core failed to validate Periphery public key");
        Err(e)
      }
    }
  }
}

impl<'a> PeripheryConnectionArgs<'a> {
  pub fn from_server(server: &'a Server) -> Self {
    Self {
      id: &server.id,
      address: optional_str(&server.config.address),
      periphery_public_key: optional_str(&server.info.public_key),
    }
  }

  pub fn from_url_builder(
    id: &'a str,
    config: &'a UrlBuilderConfig,
  ) -> Self {
    Self {
      id,
      address: optional_str(&config.address),
      periphery_public_key: optional_str(
        &config.periphery_public_key,
      ),
    }
  }

  pub fn from_aws_builder(
    id: &'a str,
    address: &'a str,
    config: &'a AwsBuilderConfig,
  ) -> Self {
    Self {
      id,
      address: Some(address),
      periphery_public_key: optional_str(
        &config.periphery_public_key,
      ),
    }
  }

  pub fn to_owned(self) -> OwnedPeripheryConnectionArgs {
    OwnedPeripheryConnectionArgs {
      id: self.id.to_string(),
      address: self.address.map(str::to_string),
      periphery_public_key: self
        .periphery_public_key
        .map(str::to_string),
    }
  }

  pub fn matches<'b>(
    self,
    args: impl Into<PeripheryConnectionArgs<'b>>,
  ) -> bool {
    self == args.into()
  }
}

#[derive(Debug, Clone)]
pub struct OwnedPeripheryConnectionArgs {
  /// Usually the Server id.
  pub id: String,
  /// Specify outbound connection address.
  /// Inbound connections have this as None
  pub address: Option<String>,
  /// The public key to expect Periphery to have.
  /// If None, must have 'periphery_public_keys' set
  /// in Core config, or will error
  pub periphery_public_key: Option<String>,
}

impl OwnedPeripheryConnectionArgs {
  pub fn borrow(&self) -> PeripheryConnectionArgs<'_> {
    PeripheryConnectionArgs {
      id: &self.id,
      address: self.address.as_deref(),
      periphery_public_key: self.periphery_public_key.as_deref(),
    }
  }
}

impl From<PeripheryConnectionArgs<'_>>
  for OwnedPeripheryConnectionArgs
{
  fn from(value: PeripheryConnectionArgs<'_>) -> Self {
    value.to_owned()
  }
}

impl<'a> From<&'a OwnedPeripheryConnectionArgs>
  for PeripheryConnectionArgs<'a>
{
  fn from(value: &'a OwnedPeripheryConnectionArgs) -> Self {
    value.borrow()
  }
}

#[derive(Debug)]
pub struct PeripheryConnection {
  /// The connection args
  pub args: OwnedPeripheryConnectionArgs,
  /// Send and receive bytes over the connection socket.
  pub sender: Sender<Bytes>,
  /// Cancel the connection
  pub cancel: CancellationToken,
  /// Whether Periphery is currently connected.
  pub connected: AtomicBool,
  // These fields must be maintained if new connection replaces old
  // at the same server id.
  /// Stores latest connection error
  pub error: Arc<RwLock<Option<serror::Serror>>>,
  /// Forward bytes from Periphery to specific channel handlers.
  pub channels: Arc<ConnectionChannels>,
}

impl PeripheryConnection {
  pub fn new(
    args: impl Into<OwnedPeripheryConnectionArgs>,
  ) -> (Arc<PeripheryConnection>, BufferedReceiver<Bytes>) {
    let (sender, receiever) = buffered_channel();
    (
      PeripheryConnection {
        sender,
        args: args.into(),
        cancel: CancellationToken::new(),
        connected: AtomicBool::new(false),
        error: Default::default(),
        channels: Default::default(),
      }
      .into(),
      receiever,
    )
  }

  pub fn with_new_args(
    &self,
    args: impl Into<OwnedPeripheryConnectionArgs>,
  ) -> (Arc<PeripheryConnection>, BufferedReceiver<Bytes>) {
    // Ensure this connection is cancelled.
    self.cancel();
    let (sender, receiever) = buffered_channel();
    (
      PeripheryConnection {
        sender,
        args: args.into(),
        cancel: CancellationToken::new(),
        connected: AtomicBool::new(false),
        error: self.error.clone(),
        channels: self.channels.clone(),
      }
      .into(),
      receiever,
    )
  }

  pub async fn handle_login<W: Websocket, L: LoginFlow>(
    &self,
    socket: &mut W,
    identifiers: ConnectionIdentifiers<'_>,
  ) -> anyhow::Result<()> {
    L::login(LoginFlowArgs {
      socket,
      identifiers,
      private_key: core_private_key(),
      public_key_validator: self.args.borrow(),
    })
    .await?;
    // Clear attempted public key after successful login
    spawn_update_attempted_public_key(self.args.id.clone(), None);
    Ok(())
  }

  pub async fn handle_socket<W: Websocket>(
    &self,
    socket: W,
    receiver: &mut BufferedReceiver<Bytes>,
  ) {
    let cancel = self.cancel.child_token();

    self.set_connected(true);
    self.clear_error().await;

    let (mut ws_write, mut ws_read) = socket.split();

    let forward_writes = async {
      loop {
        let next = tokio::select! {
          next = receiver.recv() => next,
          _ = cancel.cancelled() => break,
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
      cancel.cancel();
    };

    let handle_reads = async {
      loop {
        let next = tokio::select! {
          next = ws_read.recv() => next,
          _ = cancel.cancelled() => break,
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
      cancel.cancel();
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

/// Spawn task to set the 'attempted_public_key'
/// for easy manual connection acceptance later on.
fn spawn_update_attempted_public_key(
  id: String,
  public_key: impl Into<Option<String>>,
) {
  let public_key = public_key.into();
  tokio::spawn(async move {
    if let Err(e) = update_one_by_id(
      &db_client().servers,
      &id,
      doc! {
        "$set": {
          "info.attempted_public_key": &public_key.as_deref().unwrap_or_default(),
        }
      },
      None,
    )
    .await
    {
      warn!(
        "Failed to update attempted public_key for Server {id} | {e:?}"
      );
    };
  });
}
