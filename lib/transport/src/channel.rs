use std::ops::Deref;

use anyhow::Context;
use tokio::sync::{Mutex, MutexGuard, mpsc};

const RESPONSE_BUFFER_MAX_LEN: usize = 1_024;

#[derive(Debug)]
pub struct BufferedChannel<T> {
  pub sender: mpsc::Sender<T>,
  pub receiver: Mutex<BufferedReceiver<T>>,
}

impl<T: Deref> Default for BufferedChannel<T> {
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

/// Create a buffered channel
pub fn buffered_channel<T: Deref>()
-> (mpsc::Sender<T>, BufferedReceiver<T>) {
  let (sender, receiver) = mpsc::channel(RESPONSE_BUFFER_MAX_LEN);
  (sender, BufferedReceiver::new(receiver))
}

/// Wrapper around channel receiver to control when
/// the latest message is dropped,
/// in case it must be re-transmitted.
#[derive(Debug)]
pub struct BufferedReceiver<T> {
  receiver: mpsc::Receiver<T>,
  buffer: Option<T>,
}

impl<T: Deref> BufferedReceiver<T> {
  pub fn new(receiver: mpsc::Receiver<T>) -> BufferedReceiver<T> {
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
  pub async fn recv(&mut self) -> Option<&<T as Deref>::Target> {
    if self.buffer.is_none() {
      self.buffer = Some(self.receiver.recv().await?);
    }
    self.buffer.as_deref()
  }

  /// Clears buffer.
  /// Should be called after transmission confirmed.
  pub fn clear_buffer(&mut self) {
    self.buffer = None;
  }
}
