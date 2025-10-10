use anyhow::{Context, anyhow};
use axum::extract::ws::CloseFrame;
use bytes::Bytes;
use futures_util::{
  SinkExt, Stream, StreamExt, TryStreamExt,
  stream::{SplitSink, SplitStream},
};
use tokio_util::sync::CancellationToken;

use crate::{
  message::{CastBytes, EncodedTransportMessage},
  timeout::MaybeWithTimeout,
};

use super::{
  Websocket, WebsocketMessage, WebsocketReceiver, WebsocketSender,
};

pub struct AxumWebsocket(pub axum::extract::ws::WebSocket);

impl Websocket for AxumWebsocket {
  type CloseFrame = CloseFrame;

  fn split(self) -> (impl WebsocketSender, impl WebsocketReceiver) {
    let (tx, rx) = self.0.split();
    (AxumWebsocketSender(tx), AxumWebsocketReceiver::new(rx))
  }

  async fn send_inner(&mut self, bytes: Bytes) -> anyhow::Result<()> {
    self
      .0
      .send(axum::extract::ws::Message::Binary(bytes))
      .await
      .context("Failed to send message bytes over websocket")
  }

  async fn close(&mut self) -> anyhow::Result<()> {
    self
      .0
      .send(axum::extract::ws::Message::Close(None))
      .await
      .context("Failed to send websocket close frame")
  }

  fn recv_inner(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<
      Output = anyhow::Result<WebsocketMessage<Self::CloseFrame>>,
    >,
  > {
    MaybeWithTimeout::new(try_next(&mut self.0))
  }
}

pub type InnerWebsocketSender =
  SplitSink<axum::extract::ws::WebSocket, axum::extract::ws::Message>;

pub struct AxumWebsocketSender(pub InnerWebsocketSender);

impl WebsocketSender for AxumWebsocketSender {
  async fn send_inner(&mut self, bytes: Bytes) -> anyhow::Result<()> {
    self
      .0
      .send(axum::extract::ws::Message::Binary(bytes))
      .await
      .context("Failed to send message over websocket")
  }

  async fn close(&mut self) -> anyhow::Result<()> {
    self
      .0
      .send(axum::extract::ws::Message::Close(None))
      .await
      .context("Failed to send websocket close frame")
  }
}

async fn try_next<S>(
  stream: &mut S,
) -> anyhow::Result<WebsocketMessage<CloseFrame>>
where
  S: Stream<Item = Result<axum::extract::ws::Message, axum::Error>>
    + Unpin,
{
  loop {
    match stream.try_next().await? {
      Some(axum::extract::ws::Message::Binary(bytes)) => {
        return Ok(WebsocketMessage::Message(
          EncodedTransportMessage::from_vec(bytes.into()),
        ));
      }
      Some(axum::extract::ws::Message::Text(text)) => {
        let bytes: Bytes = text.into();
        return Ok(WebsocketMessage::Message(
          EncodedTransportMessage::from_vec(bytes.into()),
        ));
      }
      Some(axum::extract::ws::Message::Close(frame)) => {
        return Ok(WebsocketMessage::Close(frame));
      }
      None => return Ok(WebsocketMessage::Closed),
      // Ignored messages
      Some(axum::extract::ws::Message::Ping(_))
      | Some(axum::extract::ws::Message::Pong(_)) => continue,
    }
  }
}

pub type InnerWebsocketReceiver =
  SplitStream<axum::extract::ws::WebSocket>;

pub struct AxumWebsocketReceiver {
  receiver: InnerWebsocketReceiver,
  cancel: Option<CancellationToken>,
}

impl AxumWebsocketReceiver {
  pub fn new(receiver: InnerWebsocketReceiver) -> Self {
    Self {
      receiver,
      cancel: None,
    }
  }
}

impl WebsocketReceiver for AxumWebsocketReceiver {
  type CloseFrame = CloseFrame;

  fn set_cancel(&mut self, cancel: CancellationToken) {
    self.cancel = Some(cancel);
  }

  async fn recv_inner(
    &mut self,
  ) -> anyhow::Result<WebsocketMessage<Self::CloseFrame>> {
    let fut = try_next(&mut self.receiver);
    if let Some(cancel) = &self.cancel {
      tokio::select! {
        res = fut => res,
        _ = cancel.cancelled() => Err(anyhow!("Cancelled before receive"))
      }
    } else {
      fut.await
    }
  }
}
