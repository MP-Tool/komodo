use std::{sync::Arc, time::Duration};

use anyhow::{Context, anyhow};
use futures::{Stream, StreamExt, TryStreamExt};
use komodo_client::{
  api::write::TerminalRecreateMode,
  entities::{KOMODO_EXIT_CODE, NoData, server::TerminalInfo},
};
use periphery_client::api::terminal::*;
use resolver_api::Resolve;
use tokio_util::{codec::LinesCodecError, sync::CancellationToken};
use transport::{channel::Sender, message::EncodedTransportMessage};
use uuid::Uuid;

use crate::{
  config::periphery_config, connection::core_channels, terminal::*,
};

//

impl Resolve<super::Args> for ListTerminals {
  #[instrument(name = "ListTerminals", level = "debug")]
  async fn resolve(
    self,
    _: &super::Args,
  ) -> anyhow::Result<Vec<TerminalInfo>> {
    clean_up_terminals().await;
    Ok(list_terminals().await)
  }
}

//

impl Resolve<super::Args> for CreateTerminal {
  #[instrument(name = "CreateTerminal", level = "debug")]
  async fn resolve(self, _: &super::Args) -> anyhow::Result<NoData> {
    if periphery_config().disable_terminals {
      return Err(anyhow!(
        "Terminals are disabled in the periphery config"
      ));
    }
    create_terminal(self.name, self.command, self.recreate)
      .await
      .map(|_| NoData {})
  }
}

//

impl Resolve<super::Args> for DeleteTerminal {
  #[instrument(name = "DeleteTerminal", level = "debug")]
  async fn resolve(self, _: &super::Args) -> anyhow::Result<NoData> {
    delete_terminal(&self.terminal).await;
    Ok(NoData {})
  }
}

//

impl Resolve<super::Args> for DeleteAllTerminals {
  #[instrument(name = "DeleteAllTerminals", level = "debug")]
  async fn resolve(self, _: &super::Args) -> anyhow::Result<NoData> {
    delete_all_terminals().await;
    Ok(NoData {})
  }
}

//

impl Resolve<super::Args> for ConnectTerminal {
  #[instrument(name = "ConnectTerminal", level = "debug")]
  async fn resolve(self, args: &super::Args) -> anyhow::Result<Uuid> {
    if periphery_config().disable_terminals {
      return Err(anyhow!(
        "Terminals are disabled in the periphery config"
      ));
    }

    let channel =
      core_channels().get(&args.core).await.with_context(|| {
        format!("Failed to find channel for {}", args.core)
      })?;

    clean_up_terminals().await;

    let terminal = get_terminal(&self.terminal).await?;

    let channel_id = Uuid::new_v4();

    tokio::spawn(async move {
      handle_terminal_forwarding(
        &channel.sender,
        channel_id,
        terminal,
      )
      .await
    });

    Ok(channel_id)
  }
}

//

impl Resolve<super::Args> for ConnectContainerExec {
  #[instrument(name = "ConnectContainerExec", level = "debug")]
  async fn resolve(self, args: &super::Args) -> anyhow::Result<Uuid> {
    if periphery_config().disable_container_exec {
      return Err(anyhow!(
        "Container exec is disabled in the periphery config"
      ));
    }

    let channel =
      core_channels().get(&args.core).await.with_context(|| {
        format!("Failed to find channel for {}", args.core)
      })?;

    let ConnectContainerExec { container, shell } = self;

    if container.contains("&&") || shell.contains("&&") {
      return Err(anyhow!(
        "The use of '&&' is forbidden in the container name or shell"
      ));
    }

    // Create (recreate if shell changed)
    let terminal = create_terminal(
      container.clone(),
      format!("docker exec -it {container} {shell}"),
      TerminalRecreateMode::DifferentCommand,
    )
    .await
    .context("Failed to create terminal for container exec")?;

    let channel_id = Uuid::new_v4();

    tokio::spawn(async move {
      handle_terminal_forwarding(
        &channel.sender,
        channel_id,
        terminal,
      )
      .await
    });

    Ok(channel_id)
  }
}

//

impl Resolve<super::Args> for DisconnectTerminal {
  #[instrument(name = "DisconnectTerminal", level = "debug")]
  async fn resolve(self, _: &super::Args) -> anyhow::Result<NoData> {
    if let Some(channel) = terminal_channels().remove(&self.id).await
    {
      channel.cancel.cancel();
    }
    Ok(NoData {})
  }
}

//

impl Resolve<super::Args> for ExecuteTerminal {
  #[instrument(name = "ExecuteTerminal", level = "debug")]
  async fn resolve(self, args: &super::Args) -> anyhow::Result<Uuid> {
    if periphery_config().disable_terminals {
      return Err(anyhow!(
        "Terminals are disabled in the Periphery config"
      ));
    }

    let channel =
      core_channels().get(&args.core).await.with_context(|| {
        format!("Failed to find channel for {}", args.core)
      })?;

    let terminal = get_terminal(&self.terminal).await?;

    let channel_id = Uuid::new_v4();

    let stdout = setup_execute_command_on_terminal(
      channel_id,
      &terminal,
      &self.command,
    )
    .await?;

    tokio::spawn(async move {
      forward_execute_command_on_terminal_response(
        &channel.sender,
        channel_id,
        stdout,
      )
      .await
    });

    Ok(channel_id)
  }
}

//

impl Resolve<super::Args> for ExecuteContainerExec {
  #[instrument(name = "ExecuteContainerExec", level = "debug")]
  async fn resolve(self, args: &super::Args) -> anyhow::Result<Uuid> {
    if periphery_config().disable_container_exec {
      return Err(anyhow!(
        "Container exec is disabled in the Periphery config"
      ));
    }

    let Self {
      container,
      shell,
      command,
    } = self;

    if container.contains("&&") || shell.contains("&&") {
      return Err(anyhow!(
        "The use of '&&' is forbidden in the container name or shell"
      ));
    }

    let channel =
      core_channels().get(&args.core).await.with_context(|| {
        format!("Failed to find channel for {}", args.core)
      })?;

    // Create terminal (recreate if shell changed)
    let terminal = create_terminal(
      container.clone(),
      format!("docker exec -it {container} {shell}"),
      TerminalRecreateMode::DifferentCommand,
    )
    .await
    .context("Failed to create terminal for container exec")?;

    // Wait a bit for terminal to initialize
    tokio::time::sleep(Duration::from_millis(500)).await;

    let channel_id = Uuid::new_v4();

    let stdout = setup_execute_command_on_terminal(
      channel_id, &terminal, &command,
    )
    .await?;

    tokio::spawn(async move {
      forward_execute_command_on_terminal_response(
        &channel.sender,
        channel_id,
        stdout,
      )
      .await
    });

    Ok(channel_id)
  }
}

async fn handle_terminal_forwarding(
  sender: &Sender<EncodedTransportMessage>,
  channel: Uuid,
  terminal: Arc<Terminal>,
) {
  let cancel = CancellationToken::new();

  let (_, trigger) = tokio::join!(
    terminal_channels().insert(
      channel,
      Arc::new(TerminalChannel {
        sender: terminal.stdin.clone(),
        cancel: cancel.clone(),
      }),
    ),
    terminal_triggers().insert(channel),
  );

  if let Err(e) = trigger.wait().await {
    warn!(
      "Failed to init terminal | Failed to receive begin trigger | {e:#}"
    );
    terminal_channels().remove(&channel).await;
    return;
  }

  let init_res = async {
    let (a, b) = terminal.history.bytes_parts();
    if !a.is_empty() {
      sender
        .send_terminal(channel, a)
        .await
        .context("Failed to send history part a")?;
    }
    if !b.is_empty() {
      sender
        .send_terminal(channel, b)
        .await
        .context("Failed to send history part b")?;
    }
    anyhow::Ok(())
  }
  .await;

  if let Err(e) = init_res {
    // TODO: Handle error
    warn!("Failed to init terminal | {e:#}");
    terminal_channels().remove(&channel).await;
    return;
  }

  // Forward stdout -> WS
  let mut stdout = terminal.stdout.resubscribe();
  loop {
    let res = tokio::select! {
      res = stdout.recv() => res,
      _ = terminal.cancel.cancelled() => {
        trace!("ws write: cancelled from outside");
        // let _ = ws_sender.send("PTY KILLED")).await;
        // if let Err(e) = ws_write.close().await {
        //   debug!("Failed to close ws: {e:?}");
        // };
        break
      },
      _ = cancel.cancelled() => {
        // let _ = ws_write.send(Message::Text(Utf8Bytes::from_static("WS KILLED"))).await;
        // if let Err(e) = ws_write.close().await {
        //   debug!("Failed to close ws: {e:?}");
        // };
        break
      }
    };

    let bytes = match res {
      Ok(bytes) => bytes,
      Err(e) => {
        debug!("Terminal receiver failed | {e:?}");
        terminal.cancel();
        break;
      }
    };

    if let Err(e) = sender.send_terminal(channel, bytes).await {
      debug!("Failed to send to WS: {e:?}");
      cancel.cancel();
      break;
    }
  }

  // Clean up
  if let Some(terminal_channel) =
    terminal_channels().remove(&channel).await
  {
    trace!("Cancel called for {channel}");
    terminal_channel.cancel.cancel();
  }
  clean_up_terminals().await;
}

/// This is run before spawning task handler
async fn setup_execute_command_on_terminal(
  channel_id: Uuid,
  terminal: &Terminal,
  command: &str,
) -> anyhow::Result<
  impl Stream<Item = Result<String, LinesCodecError>> + 'static,
> {
  // Read the bytes into lines
  // This is done to check the lines for the EOF sentinal
  let mut stdout = tokio_util::codec::FramedRead::new(
    tokio_util::io::StreamReader::new(
      tokio_stream::wrappers::BroadcastStream::new(
        terminal.stdout.resubscribe(),
      )
      .map(|res| res.map_err(std::io::Error::other)),
    ),
    tokio_util::codec::LinesCodec::new(),
  );

  let full_command = format!(
    "printf '\n{START_OF_OUTPUT}\n\n'; {command}; rc=$?; printf '\n{KOMODO_EXIT_CODE}%d\n{END_OF_OUTPUT}\n' \"$rc\"\n"
  );

  terminal
    .stdin
    .send(StdinMsg::Bytes(full_command.into()))
    .await
    .context("Failed to send command to terminal stdin")?;

  // Only start the response AFTER the start sentinel is printed
  loop {
    match stdout
      .try_next()
      .await
      .context("Failed to read stdout line")?
    {
      Some(line) if line == START_OF_OUTPUT => break,
      // Keep looping until the start sentinel received.
      Some(_) => {}
      None => {
        return Err(anyhow!(
          "Stdout stream terminated before start sentinel received"
        ));
      }
    }
  }

  terminal_triggers().insert(channel_id).await;

  Ok(stdout)
}

async fn forward_execute_command_on_terminal_response(
  sender: &Sender<EncodedTransportMessage>,
  channel: Uuid,
  mut stdout: impl Stream<Item = Result<String, LinesCodecError>> + Unpin,
) {
  if let Err(e) = terminal_triggers().wait(&channel).await {
    warn!("{e:#}");
    return;
  }

  loop {
    match stdout.next().await {
      Some(Ok(line)) if line.as_str() == END_OF_OUTPUT => {
        if let Err(e) = sender.send_terminal(channel, line).await {
          warn!("Got ws_sender send error on END_OF_OUTPUT | {e:?}");
        }
        break;
      }
      Some(Ok(line)) => {
        if let Err(e) =
          sender.send_terminal(channel, line + "\n").await
        {
          warn!("Got ws_sender send error | {e:?}");
          break;
        }
      }
      Some(Err(e)) => {
        warn!("Got stdout stream error | {e:?}");
        break;
      }
      None => {
        clean_up_terminals().await;
        break;
      }
    }
  }
}
