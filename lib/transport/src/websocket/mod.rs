//! Wrappers to normalize behavior of websockets between Tungstenite and Axum,
//! as well as streamline process of handling socket messages.

use std::time::Duration;

use anyhow::{Context, anyhow};
use bytes::Bytes;
use futures_util::FutureExt;

pub mod axum;
pub mod tungstenite;

/// Flattened websocket message possibilites
/// for easier handling.
pub enum WebsocketMessage<CloseFrame> {
  /// Standard message
  Binary(Bytes),
  /// Graceful close message
  Close(Option<CloseFrame>),
  /// Stream closed
  Closed,
}

/// Standard traits for websocket
pub trait Websocket {
  type CloseFrame: std::fmt::Debug + Send + Sync + 'static;
  type Error: std::error::Error + Send + Sync + 'static;

  /// Abstraction over websocket splitting
  fn split(self) -> (impl WebsocketSender, impl WebsocketReceiver);

  /// Looping receiver for websocket messages which only returns
  /// on significant messages.
  fn recv(
    &mut self,
  ) -> impl Future<
    Output = Result<WebsocketMessage<Self::CloseFrame>, Self::Error>,
  >;

  /// Looping receiver for websocket messages which only returns on bytes.
  fn recv_bytes(
    &mut self,
  ) -> impl Future<Output = Result<Bytes, anyhow::Error>> {
    async {
      match self.recv().await? {
        WebsocketMessage::Binary(bytes) => Ok(bytes),
        WebsocketMessage::Close(frame) => {
          Err(anyhow!("Connection closed with framed: {frame:?}"))
        }
        WebsocketMessage::Closed => {
          Err(anyhow!("Connection already closed"))
        }
      }
    }
  }

  /// Looping receiver for websocket messages which only returns on bytes.
  /// Includes timeout.
  fn recv_bytes_with_timeout(
    &mut self,
    timeout: Duration,
  ) -> impl Future<Output = Result<Bytes, anyhow::Error>> {
    tokio::time::timeout(timeout, self.recv_bytes())
      .map(|res| res.context("Failed to receive bytes").flatten())
  }

  /// Streamlined sending on bytes
  fn send(
    &mut self,
    bytes: Bytes,
  ) -> impl Future<Output = Result<(), Self::Error>>;

  /// Send close message
  fn close(
    &mut self,
    frame: Option<Self::CloseFrame>,
  ) -> impl Future<Output = Result<(), Self::Error>>;
}

/// Traits for split websocket receiver
pub trait WebsocketReceiver {
  type CloseFrame: std::fmt::Debug + Send + Sync + 'static;
  type Error: std::error::Error + Send + Sync + 'static;

  /// Looping receiver for websocket messages which only returns
  /// on significant messages.
  fn recv(
    &mut self,
  ) -> impl Future<
    Output = Result<WebsocketMessage<Self::CloseFrame>, Self::Error>,
  > + Send
  + Sync;
}

/// Traits for split websocket receiver
pub trait WebsocketSender {
  type CloseFrame: std::fmt::Debug + Send + Sync + 'static;
  type Error: std::error::Error + Send + Sync + 'static;

  /// Streamlined sending on bytes
  fn send(
    &mut self,
    bytes: Bytes,
  ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

  /// Send close message
  fn close(
    &mut self,
    frame: Option<Self::CloseFrame>,
  ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;
}
