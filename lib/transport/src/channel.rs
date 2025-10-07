use anyhow::{Context, anyhow};
use bytes::Bytes;
use futures_util::FutureExt;
use tokio::sync::{Mutex, MutexGuard, mpsc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
  message::{Message, MessageState},
  timeout::MaybeWithTimeout,
};

const RESPONSE_BUFFER_MAX_LEN: usize = 1_024;

#[derive(Debug)]
pub struct BufferedChannel {
  pub sender: Sender,
  pub receiver: Mutex<BufferedReceiver>,
}

impl Default for BufferedChannel {
  fn default() -> Self {
    let (sender, receiver) = buffered_channel();
    BufferedChannel {
      sender,
      receiver: receiver.into(),
    }
  }
}

impl BufferedChannel {
  pub fn receiver(
    &self,
  ) -> anyhow::Result<MutexGuard<'_, BufferedReceiver>> {
    self
      .receiver
      .try_lock()
      .context("Receiver is already locked")
  }
}

/// Create a channel
pub fn channel() -> (Sender, Receiver) {
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
pub fn buffered_channel() -> (Sender, BufferedReceiver) {
  let (sender, receiver) = channel();
  (sender, BufferedReceiver::new(receiver))
}

#[derive(Debug, Clone)]
pub struct Sender(mpsc::Sender<Message>);

impl Sender {
  pub async fn send(
    &self,
    message: impl Into<Message>,
  ) -> Result<(), mpsc::error::SendError<Message>> {
    self.0.send(message.into()).await
  }
}

#[derive(Debug)]
pub struct Receiver {
  receiver: mpsc::Receiver<Message>,
  cancel: Option<CancellationToken>,
}

impl Receiver {
  pub fn set_cancel(&mut self, cancel: CancellationToken) {
    self.cancel = Some(cancel);
  }

  pub fn poll_recv(
    &mut self,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Option<Message>> {
    if let Some(cancel) = &self.cancel
      && cancel.is_cancelled()
    {
      return std::task::Poll::Ready(None);
    }
    self.receiver.poll_recv(cx)
  }

  pub fn recv(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<Message>> + Send,
  > {
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

  pub fn recv_parts(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<(Bytes, Uuid, MessageState)>>
    + Send,
  > {
    MaybeWithTimeout::new(self.recv().map(|res| {
      res
        .context("Channel is permanently closed.")
        .and_then(Message::into_parts)
    }))
  }
}

/// Control when the latest message is dropped, in case it must be re-transmitted.
#[derive(Debug)]
pub struct BufferedReceiver {
  receiver: Receiver,
  buffer: Option<Message>,
}

impl BufferedReceiver {
  pub fn new(receiver: Receiver) -> BufferedReceiver {
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
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<Message>> + Send,
  > {
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

  pub fn recv_parts(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<(Bytes, Uuid, MessageState)>>
    + Send,
  > {
    MaybeWithTimeout::new(
      self.recv().map(|res| res.and_then(Message::into_parts)),
    )
  }

  /// Clears buffer.
  /// Should be called after transmission confirmed.
  pub fn clear_buffer(&mut self) {
    self.buffer = None;
  }
}
