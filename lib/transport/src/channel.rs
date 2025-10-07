use anyhow::Context;
use bytes::Bytes;
use tokio::sync::{Mutex, MutexGuard, mpsc};
use uuid::Uuid;

use crate::message::{BorrowedMessage, Message, MessageState};

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
  (Sender(sender), Receiver(receiver))
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
pub struct Receiver(mpsc::Receiver<Message>);

impl Receiver {
  pub async fn recv(&mut self) -> Option<Message> {
    self.0.recv().await
  }

  pub async fn recv_parts(
    &mut self,
  ) -> anyhow::Result<(Bytes, Uuid, MessageState)> {
    let message = self.recv().await.context("Channel is broken")?;
    message.into_parts()
  }

  pub fn poll_recv(
    &mut self,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Option<Message>> {
    self.0.poll_recv(cx)
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

  /// - If 'buffer: Some(bytes)':
  ///   - Immediately returns borrow of buffer.
  /// - Else:
  ///   - Wait for next item.
  ///   - store in buffer.
  ///   - return borrow of buffer.
  pub async fn recv(&mut self) -> Option<BorrowedMessage<'_>> {
    if self.buffer.is_none() {
      self.buffer = Some(self.receiver.recv().await?);
    }
    self.buffer.as_ref().map(Message::borrow)
  }

  /// Clears buffer.
  /// Should be called after transmission confirmed.
  pub fn clear_buffer(&mut self) {
    self.buffer = None;
  }
}
