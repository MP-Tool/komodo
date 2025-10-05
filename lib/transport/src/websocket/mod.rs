//! Wrappers to normalize behavior of websockets between Tungstenite and Axum,
//! as well as streamline process of handling socket messages.

use std::time::Duration;

use anyhow::anyhow;
use bytes::Bytes;
use futures_util::FutureExt;
use pin_project_lite::pin_project;
use serror::{deserialize_error_bytes, serialize_error_bytes};

use crate::MessageState;

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
pub trait Websocket: Send {
  type CloseFrame: std::fmt::Debug + Send + Sync + 'static;
  type Error: std::error::Error + Send + Sync + 'static;

  /// Abstraction over websocket splitting
  fn split(self) -> (impl WebsocketSender, impl WebsocketReceiver);

  /// Looping receiver for websocket messages which only returns
  /// on significant messages.
  fn recv(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<
      Output = Result<
        WebsocketMessage<Self::CloseFrame>,
        Self::Error,
      >,
    > + Send,
  >;

  /// Looping receiver for websocket messages which only returns on bytes.
  fn recv_bytes(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = Result<Bytes, anyhow::Error>> + Send,
  > {
    MaybeWithTimeout {
      inner: async {
        match self.recv().await? {
          WebsocketMessage::Binary(bytes) => Ok(bytes),
          WebsocketMessage::Close(frame) => {
            Err(anyhow!("Connection closed with framed: {frame:?}"))
          }
          WebsocketMessage::Closed => {
            Err(anyhow!("Connection already closed"))
          }
        }
      },
    }
  }

  fn recv_result(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = Result<anyhow::Result<Bytes>, anyhow::Error>>
    + Send,
  > {
    MaybeWithTimeout {
      inner: self.recv_bytes().map(|res| {
        res.map(|bytes| {
          if bytes.is_empty() {
            return Err(anyhow!(
              "Message does not contain state byte"
            ));
          }
          let mut bytes: Vec<u8> = bytes.into();
          let state = bytes.pop();
          let bytes: Bytes = bytes.into();
          match state.map(MessageState::from_byte) {
            Some(MessageState::Successful) => Ok(bytes),
            _ => Err(deserialize_error_bytes(&bytes)),
          }
        })
      }),
    }
  }

  /// Streamlined sending on bytes
  fn send(
    &mut self,
    bytes: Bytes,
  ) -> impl Future<Output = Result<(), Self::Error>>;

  fn send_data(
    &mut self,
    mut data: Vec<u8>,
    state: MessageState,
  ) -> impl Future<Output = Result<(), Self::Error>> {
    data.push(state.as_byte());
    self.send(data.into())
  }

  fn send_error(
    &mut self,
    e: &anyhow::Error,
  ) -> impl Future<Output = Result<(), Self::Error>> {
    let mut bytes = serialize_error_bytes(e);
    bytes.push(MessageState::Failed.as_byte());
    self.send(bytes.into())
  }

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

pin_project! {
  pub struct MaybeWithTimeout<F> {
    #[pin]
    inner: F,
  }
}

impl<F: Future> Future for MaybeWithTimeout<F> {
  type Output = F::Output;
  fn poll(
    self: std::pin::Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Self::Output> {
    let mut inner = self.project().inner;
    inner.as_mut().poll(cx)
  }
}

impl<F: Future + Send> MaybeWithTimeout<F> {
  pub fn with_timeout(
    self,
    timeout: Duration,
  ) -> impl Future<Output = anyhow::Result<F::Output>> + Send {
    tokio::time::timeout(timeout, self.inner)
      .map(|res| res.map_err(|_| anyhow!("Timed out")))
  }
}
