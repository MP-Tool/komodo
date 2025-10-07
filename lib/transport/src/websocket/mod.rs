//! Wrappers to normalize behavior of websockets between Tungstenite and Axum,
//! as well as streamline process of handling socket messages.

use anyhow::{Context, anyhow};
use bytes::Bytes;
use futures_util::FutureExt;
use serror::deserialize_error_bytes;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
  message::{Message, MessageState},
  timeout::MaybeWithTimeout,
};

pub mod axum;
pub mod tungstenite;

/// Flattened websocket message possibilites
/// for easier handling.
pub enum WebsocketMessage<CloseFrame> {
  /// Standard message
  Message(Message),
  /// Graceful close message
  Close(Option<CloseFrame>),
  /// Stream closed
  Closed,
}

/// Standard traits for websocket
pub trait Websocket: Send {
  type CloseFrame: std::fmt::Debug + Send + Sync + 'static;

  /// Abstraction over websocket splitting
  fn split(self) -> (impl WebsocketSender, impl WebsocketReceiver);

  /// Looping receiver for websocket messages which only returns
  /// on significant messages.
  fn recv(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<
      Output = anyhow::Result<WebsocketMessage<Self::CloseFrame>>,
    > + Send,
  >;

  fn send(
    &mut self,
    message: impl Into<Message>,
  ) -> impl Future<Output = anyhow::Result<()>>;

  /// Send close message
  fn close(
    &mut self,
    frame: Option<Self::CloseFrame>,
  ) -> impl Future<Output = anyhow::Result<()>>;

  /// Looping receiver for websocket messages which only returns on messages.
  fn recv_message(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<Message>> + Send,
  > {
    MaybeWithTimeout::new(async {
      match self.recv().await? {
        WebsocketMessage::Message(message) => Ok(message),
        WebsocketMessage::Close(frame) => {
          Err(anyhow!("Connection closed with framed: {frame:?}"))
        }
        WebsocketMessage::Closed => {
          Err(anyhow!("Connection already closed"))
        }
      }
    })
  }

  /// Receive message + message.into_parts
  fn recv_parts(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<(Bytes, Uuid, MessageState)>>
    + Send,
  > {
    MaybeWithTimeout::new(
      self
        .recv_message()
        .map(|res| res.map(|message| message.into_parts()).flatten()),
    )
  }

  /// Auto deserializes non-successful message errors.
  /// Discards the channels.
  fn recv_result(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<Bytes>> + Send,
  > {
    MaybeWithTimeout::new(self.recv_parts().map(|res| {
      res
        .map(|(data, _, state)| match state {
          MessageState::Successful => Ok(data),
          _ => Err(deserialize_error_bytes(&data)),
        })
        .flatten()
    }))
  }
}

/// Traits for split websocket receiver
pub trait WebsocketReceiver: Send {
  type CloseFrame: std::fmt::Debug + Send + Sync + 'static;

  /// Cancellation sensitive receive.
  fn set_cancel(&mut self, _cancel: CancellationToken);

  /// Looping receiver for websocket messages which only returns
  /// on significant messages. Must implement cancel support.
  fn recv(
    &mut self,
  ) -> impl Future<
    Output = anyhow::Result<WebsocketMessage<Self::CloseFrame>>,
  > + Send;

  /// Looping receiver for websocket messages which only returns on messages.
  fn recv_message(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<Message>> + Send,
  > {
    MaybeWithTimeout::new(async {
      match self
        .recv()
        .await
        .context("Failed to read websocket message")?
      {
        WebsocketMessage::Message(message) => Ok(message),
        WebsocketMessage::Close(frame) => {
          Err(anyhow!("Connection closed with framed: {frame:?}"))
        }
        WebsocketMessage::Closed => {
          Err(anyhow!("Connection already closed"))
        }
      }
    })
  }

  /// Receive message + message.into_parts
  fn recv_parts(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<(Bytes, Uuid, MessageState)>>
    + Send,
  > {
    MaybeWithTimeout::new(
      self
        .recv_message()
        .map(|res| res.map(|message| message.into_parts()).flatten()),
    )
  }
}

/// Traits for split websocket receiver
pub trait WebsocketSender {
  type CloseFrame: std::fmt::Debug + Send + Sync + 'static;

  /// Streamlined sending on bytes
  fn send(
    &mut self,
    message: Message,
  ) -> impl Future<Output = anyhow::Result<()>> + Send;

  /// Send close message
  fn close(
    &mut self,
    frame: Option<Self::CloseFrame>,
  ) -> impl Future<Output = anyhow::Result<()>> + Send;
}
