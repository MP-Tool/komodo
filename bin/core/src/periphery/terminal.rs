use std::{
  pin::Pin,
  sync::Arc,
  task::{self, Poll},
};

use anyhow::Context;
use bytes::Bytes;
use cache::CloneCache;
use futures::Stream;
use periphery_client::{
  api::terminal::{
    ConnectContainerExec, ConnectTerminal, END_OF_OUTPUT,
    ExecuteContainerExec, ExecuteTerminal,
  },
  transport::EncodedTransportMessage,
};
use transport::channel::{Receiver, Sender, channel};
use uuid::Uuid;

use crate::{
  periphery::PeripheryClient, state::periphery_connections,
};

impl PeripheryClient {
  pub async fn connect_terminal(
    &self,
    terminal: String,
  ) -> anyhow::Result<(
    Uuid,
    Sender<EncodedTransportMessage>,
    Receiver<Vec<u8>>,
  )> {
    tracing::trace!(
      "request | type: ConnectTerminal | terminal name: {terminal}",
    );

    let connection =
      periphery_connections().get(&self.id).await.with_context(
        || format!("No connection found for server {}", self.id),
      )?;

    let channel_id = self
      .request(ConnectTerminal { terminal })
      .await
      .context("Failed to create terminal connection")?;

    let (sender, receiever) = channel();
    connection.terminals.insert(channel_id, sender).await;

    connection
      .sender
      .send_terminal(channel_id, Bytes::new())
      .await
      .context(
        "Failed to send TerminalTrigger to begin forwarding.",
      )?;

    Ok((channel_id, connection.sender.clone(), receiever))
  }

  pub async fn connect_container_exec(
    &self,
    container: String,
    shell: String,
  ) -> anyhow::Result<(
    Uuid,
    Sender<EncodedTransportMessage>,
    Receiver<Vec<u8>>,
  )> {
    tracing::trace!(
      "request | type: ConnectContainerExec | container name: {container} | shell: {shell}",
    );

    let connection =
      periphery_connections().get(&self.id).await.with_context(
        || format!("No connection found for server {}", self.id),
      )?;

    let channel_id = self
      .request(ConnectContainerExec { container, shell })
      .await
      .context("Failed to create container exec connection")?;

    let (sender, receiever) = channel();
    connection.terminals.insert(channel_id, sender).await;

    connection
      .sender
      .send_terminal(channel_id, Bytes::new())
      .await
      .context(
        "Failed to send TerminalTrigger to begin forwarding.",
      )?;

    Ok((channel_id, connection.sender.clone(), receiever))
  }

  /// Executes command on specified terminal,
  /// and streams the response ending in [KOMODO_EXIT_CODE][komodo_client::entities::KOMODO_EXIT_CODE]
  /// sentinal value as the expected final line of the stream.
  ///
  /// Example final line:
  /// ```text
  /// __KOMODO_EXIT_CODE:0
  /// ```
  ///
  /// This means the command exited with code 0 (success).
  ///
  /// If this value is NOT the final item before stream closes, it means
  /// the terminal exited mid command, before giving status. Example: running `exit`.
  #[tracing::instrument(level = "debug", skip(self))]
  pub async fn execute_terminal(
    &self,
    terminal: String,
    command: String,
  ) -> anyhow::Result<
    impl Stream<Item = anyhow::Result<Vec<u8>>> + 'static,
  > {
    tracing::trace!(
      "sending request | type: ExecuteTerminal | terminal name: {terminal} | command: {command}",
    );

    let connection =
      periphery_connections().get(&self.id).await.with_context(
        || format!("No connection found for server {}", self.id),
      )?;

    let channel_id = self
      .request(ExecuteTerminal { terminal, command })
      .await
      .context("Failed to create execute terminal connection")?;

    let (terminal_sender, terminal_receiver) = channel();
    connection
      .terminals
      .insert(channel_id, terminal_sender)
      .await;

    connection
      .sender
      .send_terminal(channel_id, Bytes::new())
      .await
      .context(
        "Failed to send TerminalTrigger to begin forwarding.",
      )?;

    Ok(ReceiverStream {
      channel_id,
      receiver: terminal_receiver,
      channels: connection.terminals.clone(),
    })
  }

  /// Executes command on specified container,
  /// and streams the response ending in [KOMODO_EXIT_CODE][komodo_client::entities::KOMODO_EXIT_CODE]
  /// sentinal value as the expected final line of the stream.
  ///
  /// Example final line:
  /// ```text
  /// __KOMODO_EXIT_CODE:0
  /// ```
  ///
  /// This means the command exited with code 0 (success).
  ///
  /// If this value is NOT the final item before stream closes, it means
  /// the container shell exited mid command, before giving status. Example: running `exit`.
  #[tracing::instrument(level = "debug", skip(self))]
  pub async fn execute_container_exec(
    &self,
    container: String,
    shell: String,
    command: String,
  ) -> anyhow::Result<ReceiverStream> {
    tracing::trace!(
      "sending request | type: ExecuteContainerExec | container: {container} | shell: {shell} | command: {command}",
    );

    let connection =
      periphery_connections().get(&self.id).await.with_context(
        || format!("No connection found for server {}", self.id),
      )?;

    let channel_id = self
      .request(ExecuteContainerExec {
        container,
        shell,
        command,
      })
      .await
      .context("Failed to create execute terminal connection")?;

    let (terminal_sender, terminal_receiver) = channel();
    connection
      .terminals
      .insert(channel_id, terminal_sender)
      .await;

    // Trigger forwarding to begin now that forwarding channel is ready.
    // This is required to not miss messages.
    connection
      .sender
      .send_terminal(channel_id, Bytes::new())
      .await?;

    Ok(ReceiverStream {
      channel_id,
      receiver: terminal_receiver,
      channels: connection.terminals.clone(),
    })
  }
}

pub struct ReceiverStream {
  channel_id: Uuid,
  channels: Arc<CloneCache<Uuid, Sender<Vec<u8>>>>,
  receiver: Receiver<Vec<u8>>,
}

impl Stream for ReceiverStream {
  type Item = anyhow::Result<Vec<u8>>;
  fn poll_next(
    mut self: Pin<&mut Self>,
    cx: &mut task::Context<'_>,
  ) -> Poll<Option<Self::Item>> {
    match self.receiver.poll_recv(cx) {
      Poll::Ready(Some(bytes))
        if bytes == END_OF_OUTPUT.as_bytes() =>
      {
        self.cleanup();
        Poll::Ready(None)
      }
      Poll::Ready(Some(bytes)) => Poll::Ready(Some(Ok(bytes))),
      // Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
      Poll::Ready(None) => {
        self.cleanup();
        Poll::Ready(None)
      }
      Poll::Pending => Poll::Pending,
    }
  }
}

impl ReceiverStream {
  fn cleanup(&self) {
    // Not the prettiest but it should be fine
    let channels = self.channels.clone();
    let id = self.channel_id;
    tokio::spawn(async move {
      channels.remove(&id).await;
    });
  }
}
