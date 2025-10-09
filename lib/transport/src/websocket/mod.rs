//! Wrappers to normalize behavior of websockets between Tungstenite and Axum

use anyhow::{Context, anyhow};
use bytes::Bytes;
use serde::Serialize;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
  message::{
    CastBytes, Decode, Encode, Message, MessageBytes,
    json::JsonMessage, wrappers::WithChannel,
  },
  timeout::MaybeWithTimeout,
};

pub mod axum;
pub mod tungstenite;

/// Flattened websocket message possibilites
/// for easier handling.
pub enum WebsocketMessage<CloseFrame> {
  /// Standard message
  Message(MessageBytes),
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

  fn send_inner(
    &mut self,
    bytes: Bytes,
  ) -> impl Future<Output = anyhow::Result<()>> + Send;

  /// Send close message
  fn close(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<()>> + Send;

  /// Looping receiver for websocket messages which only returns
  /// on significant messages.
  fn recv_inner(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<
      Output = anyhow::Result<WebsocketMessage<Self::CloseFrame>>,
    > + Send,
  >;
}

pub trait WebsocketExt: Websocket {
  fn send(
    &mut self,
    message: impl Encode<MessageBytes>,
  ) -> impl Future<Output = anyhow::Result<()>> + Send {
    self.send_inner(message.encode().into_vec().into())
  }

  /// Looping receiver for websocket messages which only returns on messages.
  fn recv(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<Message>> + Send,
  > {
    MaybeWithTimeout::new(async {
      match self.recv_inner().await? {
        WebsocketMessage::Message(message) => message.decode(),
        WebsocketMessage::Close(frame) => {
          Err(anyhow!("Connection closed with framed: {frame:?}"))
        }
        WebsocketMessage::Closed => {
          Err(anyhow!("Connection already closed"))
        }
      }
    })
  }
}

impl<W: Websocket> WebsocketExt for W {}

/// Traits for split websocket receiver
pub trait WebsocketSender {
  /// Streamlined sending on bytes
  fn send_inner(
    &mut self,
    bytes: Bytes,
  ) -> impl Future<Output = anyhow::Result<()>> + Send;

  /// Send close message
  fn close(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

pub trait WebsocketSenderExt: WebsocketSender + Send {
  fn send(
    &mut self,
    message: impl Encode<MessageBytes>,
  ) -> impl Future<Output = anyhow::Result<()>> + Send {
    self.send_inner(message.encode().into_vec().into())
  }

  fn send_request<'a, T: Serialize + Send>(
    &mut self,
    channel: Uuid,
    request: &'a T,
  ) -> impl Future<Output = anyhow::Result<()>> + Send
  where
    &'a T: Send,
  {
    async move {
      let data = JsonMessage(request).encode()?;
      let message =
        Message::Request(WithChannel { channel, data }.encode());
      self.send(message).await
    }
  }

  fn send_in_progress(
    &mut self,
    channel: Uuid,
  ) -> impl Future<Output = anyhow::Result<()>> + Send {
    let message = Message::Response(
      WithChannel {
        channel,
        data: None.encode(),
      }
      .encode(),
    );
    self.send(message)
  }

  fn send_response<'a, T: Serialize + Send>(
    &mut self,
    channel: Uuid,
    response: anyhow::Result<&'a T>,
  ) -> impl Future<Output = anyhow::Result<()>> + Send
  where
    &'a T: Send,
  {
    let data = response
      .and_then(|json| JsonMessage(json).encode())
      .encode();
    let message = Message::Response(
      WithChannel {
        channel,
        data: Some(data).encode(),
      }
      .encode(),
    );
    self.send(message)
  }

  fn send_terminal(
    &mut self,
    channel: Uuid,
    data: impl Into<Bytes>,
  ) -> impl Future<Output = anyhow::Result<()>> + Send {
    let message = Message::Terminal(
      WithChannel {
        channel,
        data: data.into(),
      }
      .encode(),
    );
    self.send(message)
  }
}

impl<S: WebsocketSender + Send> WebsocketSenderExt for S {}

/// Traits for split websocket receiver
pub trait WebsocketReceiver: Send {
  type CloseFrame: std::fmt::Debug + Send + Sync + 'static;

  /// Cancellation sensitive receive.
  fn set_cancel(&mut self, _cancel: CancellationToken);

  /// Looping receiver for websocket messages which only returns
  /// on significant messages. Must implement cancel support.
  fn recv_inner(
    &mut self,
  ) -> impl Future<
    Output = anyhow::Result<WebsocketMessage<Self::CloseFrame>>,
  > + Send;
}

pub trait WebsocketReceiverExt: WebsocketReceiver {
  /// Looping receiver for websocket messages which only returns on messages.
  fn recv(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<Message>> + Send,
  > {
    MaybeWithTimeout::new(async {
      match self
        .recv_inner()
        .await
        .context("Failed to read websocket message")?
      {
        WebsocketMessage::Message(message) => message.decode(),
        WebsocketMessage::Close(frame) => {
          Err(anyhow!("Connection closed with framed: {frame:?}"))
        }
        WebsocketMessage::Closed => {
          Err(anyhow!("Connection already closed"))
        }
      }
    })
  }
}

impl<R: WebsocketReceiver> WebsocketReceiverExt for R {}
