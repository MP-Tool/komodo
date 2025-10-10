use anyhow::{Context, anyhow};
use encoding::{
  Encode, EncodedJsonMessage, EncodedResult, JsonMessage, WithChannel,
};
use futures_util::FutureExt;
use periphery_client::transport::{
  DecodedTransportMessage, EncodedResponseMessage,
  EncodedTransportMessage, TransportMessage,
};
use serde::Serialize;
use tokio::sync::{Mutex, MutexGuard, mpsc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::timeout::MaybeWithTimeout;

const RESPONSE_BUFFER_MAX_LEN: usize = 1_024;

#[derive(Debug)]
pub struct BufferedChannel<T> {
  pub sender: Sender<T>,
  pub receiver: Mutex<BufferedReceiver<T>>,
}

impl<T: Send + Clone> Default for BufferedChannel<T> {
  fn default() -> Self {
    let (sender, receiver) = buffered_channel();
    BufferedChannel {
      sender,
      receiver: receiver.into(),
    }
  }
}

impl<T> BufferedChannel<T> {
  pub fn receiver(
    &self,
  ) -> anyhow::Result<MutexGuard<'_, BufferedReceiver<T>>> {
    self
      .receiver
      .try_lock()
      .context("Receiver is already locked")
  }
}

/// Create a channel
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
  let (sender, receiver) = mpsc::channel(RESPONSE_BUFFER_MAX_LEN);
  (
    Sender(sender),
    Receiver {
      receiver,
      cancel: None,
    },
  )
}

/// Create a buffered channel
pub fn buffered_channel<T: Send + Clone>()
-> (Sender<T>, BufferedReceiver<T>) {
  let (sender, receiver) = channel();
  (sender, BufferedReceiver::new(receiver))
}

#[derive(Debug, Clone)]
pub struct Sender<T>(mpsc::Sender<T>);

impl<T> Sender<T> {
  pub async fn send(&self, data: T) -> anyhow::Result<()> {
    self.0.send(data).await.map_err(|e| anyhow!("{e:?}"))
  }
}

impl Sender<EncodedTransportMessage> {
  pub async fn send_message(
    &self,
    message: impl Encode<EncodedTransportMessage>,
  ) -> anyhow::Result<()> {
    self.send(message.encode()).await
  }

  pub async fn send_request<'a, T: Serialize + Send>(
    &self,
    channel: Uuid,
    request: &'a T,
  ) -> anyhow::Result<()>
  where
    &'a T: Send,
  {
    let data = JsonMessage(request).encode()?;
    let message =
      DecodedTransportMessage::Request(WithChannel { channel, data });
    self.send_message(message).await
  }

  pub async fn send_in_progress(
    &self,
    channel: Uuid,
  ) -> anyhow::Result<()> {
    let message = DecodedTransportMessage::Response(WithChannel {
      channel,
      data: None,
    });
    self.send_message(message).await
  }

  pub async fn send_response(
    &self,
    channel: Uuid,
    response: EncodedResult<EncodedJsonMessage>,
  ) -> anyhow::Result<()> {
    let message = TransportMessage::Response(EncodedResponseMessage(
      WithChannel {
        channel,
        data: Some(response).encode(),
      }
      .encode(),
    ));
    self.send_message(message).await
  }

  pub async fn send_terminal(
    &self,
    channel: Uuid,
    data: impl Into<Vec<u8>>,
  ) -> anyhow::Result<()> {
    let message = DecodedTransportMessage::Terminal(WithChannel {
      channel,
      data: data.into(),
    });
    self.send_message(message).await
  }
}

#[derive(Debug)]
pub struct Receiver<T> {
  receiver: mpsc::Receiver<T>,
  cancel: Option<CancellationToken>,
}

impl<T: Send> Receiver<T> {
  pub fn set_cancel(&mut self, cancel: CancellationToken) {
    self.cancel = Some(cancel);
  }

  pub fn poll_recv(
    &mut self,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Option<T>> {
    if let Some(cancel) = &self.cancel
      && cancel.is_cancelled()
    {
      return std::task::Poll::Ready(None);
    }
    self.receiver.poll_recv(cx)
  }

  pub fn recv(
    &mut self,
  ) -> MaybeWithTimeout<impl Future<Output = anyhow::Result<T>> + Send>
  {
    MaybeWithTimeout::new(async {
      let recv = self
        .receiver
        .recv()
        .map(|res| res.context("Channel is permanently closed"));
      if let Some(cancel) = &self.cancel {
        tokio::select! {
          message = recv => message,
          _ = cancel.cancelled() => Err(anyhow!("Stream cancelled"))
        }
      } else {
        recv.await
      }
    })
  }
}

/// Control when the latest message is dropped, in case it must be re-transmitted.
#[derive(Debug)]
pub struct BufferedReceiver<T> {
  receiver: Receiver<T>,
  buffer: Option<T>,
}

impl<T: Send + Clone> BufferedReceiver<T> {
  pub fn new(receiver: Receiver<T>) -> BufferedReceiver<T> {
    BufferedReceiver {
      receiver,
      buffer: None,
    }
  }

  pub fn set_cancel(&mut self, cancel: CancellationToken) {
    self.receiver.set_cancel(cancel);
  }

  /// - If 'buffer: Some(bytes)':
  ///   - Immediately returns borrow of buffer.
  /// - Else:
  ///   - Wait for next item.
  ///   - store in buffer.
  ///   - return borrow of buffer.
  pub fn recv(
    &mut self,
  ) -> MaybeWithTimeout<impl Future<Output = anyhow::Result<T>> + Send>
  {
    MaybeWithTimeout::new(async {
      if let Some(buffer) = self.buffer.clone() {
        Ok(buffer)
      } else {
        let message = self.receiver.recv().await?;
        self.buffer = Some(message.clone());
        Ok(message)
      }
    })
  }

  /// Clears buffer.
  /// Should be called after transmission confirmed.
  pub fn clear_buffer(&mut self) {
    self.buffer = None;
  }
}
