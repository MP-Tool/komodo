//! Wrappers to normalize behavior of websockets between Tungstenite and Axum,
//! as well as streamline process of handling socket messages.

use std::time::Duration;

use anyhow::{Context, anyhow};
use bytes::Bytes;
use futures_util::FutureExt;
use pin_project_lite::pin_project;
use serror::deserialize_error_bytes;
use uuid::Uuid;

use crate::message::{Message, MessageState};

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

  fn send(
    &mut self,
    message: impl Into<Message>,
  ) -> impl Future<Output = Result<(), Self::Error>>;

  /// Send close message
  fn close(
    &mut self,
    frame: Option<Self::CloseFrame>,
  ) -> impl Future<Output = Result<(), Self::Error>>;

  /// Looping receiver for websocket messages which only returns on messages.
  fn recv_message(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<Message>> + Send,
  > {
    MaybeWithTimeout {
      inner: async {
        match self.recv().await? {
          WebsocketMessage::Message(message) => Ok(message),
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

  /// Receive message + message.into_parts
  fn recv_parts(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<(Bytes, Uuid, MessageState)>>
    + Send,
  > {
    MaybeWithTimeout {
      inner: self
        .recv_message()
        .map(|res| res.map(|message| message.into_parts()).flatten()),
    }
  }

  /// Auto deserializes non-successful message errors.
  /// Discards the channels.
  fn recv_result(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<Bytes>> + Send,
  > {
    MaybeWithTimeout {
      inner: self.recv_parts().map(|res| {
        res
          .map(|(data, _, state)| match state {
            MessageState::Successful => Ok(data),
            _ => Err(deserialize_error_bytes(&data)),
          })
          .flatten()
      }),
    }
  }
}

/// Traits for split websocket receiver
pub trait WebsocketReceiver: Send {
  type CloseFrame: std::fmt::Debug + Send + Sync + 'static;
  type Error: std::error::Error + Send + Sync + 'static;

  /// Looping receiver for websocket messages which only returns
  /// on significant messages.
  fn recv(
    &mut self,
  ) -> impl Future<
    Output = Result<WebsocketMessage<Self::CloseFrame>, Self::Error>,
  > + Send;

  /// Looping receiver for websocket messages which only returns on messages.
  fn recv_message(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<Message>> + Send,
  > {
    MaybeWithTimeout {
      inner: async {
        match self.recv().await? {
          WebsocketMessage::Message(message) => Ok(message),
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

  /// Receive message + message.into_parts
  fn recv_parts(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<(Bytes, Uuid, MessageState)>>
    + Send,
  > {
    MaybeWithTimeout {
      inner: self
        .recv_message()
        .map(|res| res.map(|message| message.into_parts()).flatten()),
    }
  }
}

/// Traits for split websocket receiver
pub trait WebsocketSender {
  type CloseFrame: std::fmt::Debug + Send + Sync + 'static;
  type Error: std::error::Error + Send + Sync + 'static;

  /// Streamlined sending on bytes
  fn send(
    &mut self,
    message: Message,
  ) -> impl Future<Output = Result<(), Self::Error>> + Send;

  /// Send close message
  fn close(
    &mut self,
    frame: Option<Self::CloseFrame>,
  ) -> impl Future<Output = Result<(), Self::Error>> + Send;
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

impl<
  O,
  E: Into<anyhow::Error>,
  F: Future<Output = Result<O, E>> + Send,
> MaybeWithTimeout<F>
{
  pub fn with_timeout(
    self,
    timeout: Duration,
  ) -> impl Future<Output = anyhow::Result<O>> + Send {
    tokio::time::timeout(timeout, self.inner).map(|res| {
      res
        .context("Timed out waiting for message.")
        .map(|inner| inner.map_err(Into::into))
        .flatten()
    })
  }
}
