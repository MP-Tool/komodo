use std::{collections::VecDeque, sync::Arc, time::Duration};

use anyhow::{Context, anyhow};
use bytes::Bytes;
use encoding::{Decode as _, WithChannel};
use komodo_client::{
  api::write::TerminalRecreateMode,
  entities::{ContainerTerminalMode, server::TerminalInfo},
};
use periphery_client::transport::EncodedTerminalMessage;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::state::{terminal_channels, terminal_triggers, terminals};

pub async fn handle_message(message: EncodedTerminalMessage) {
  let WithChannel {
    channel: channel_id,
    mut data,
  } = match message.decode() {
    Ok(res) => res,
    Err(e) => {
      warn!("Received invalid Terminal bytes | {e:#}");
      return;
    }
  };
  let msg = match data.first() {
    Some(&0x00) => StdinMsg::Bytes(data.drain(1..).collect()),
    Some(&0xFF) => {
      if let Ok(dimensions) =
        serde_json::from_slice::<ResizeDimensions>(&data[1..])
      {
        StdinMsg::Resize(dimensions)
      } else {
        return;
      }
    }
    Some(_) => StdinMsg::Bytes(data),
    // Empty bytes are the "begin" trigger for Terminal Executions
    None => {
      if let Err(e) = terminal_triggers().send(&channel_id).await {
        warn!("{e:#}")
      }
      return;
    }
  };
  let Some(channel) = terminal_channels().get(&channel_id).await
  else {
    warn!("No terminal channel for {channel_id}");
    return;
  };
  if let Err(e) = channel.sender.send(msg).await {
    warn!("No receiver for {channel_id} | {e:?}");
  };
}

pub async fn create_terminal(
  name: String,
  command: String,
  recreate: TerminalRecreateMode,
  container: Option<(String, ContainerTerminalMode)>,
) -> anyhow::Result<Arc<Terminal>> {
  trace!(
    "CreateTerminal: {name} | command: {command} | recreate: {recreate:?}"
  );
  let mut terminals = terminals().write().await;
  use TerminalRecreateMode::*;
  if matches!(recreate, Never | DifferentCommand)
    && let Some(terminal) = terminals.get(&name)
  {
    if terminal.command == command {
      return Ok(terminal.clone());
    } else if matches!(recreate, Never) {
      return Err(anyhow!(
        "Terminal {name} already exists, but has command {} instead of {command}",
        terminal.command
      ));
    }
  }
  let terminal = Arc::new(
    Terminal::new(command, container)
      .await
      .context("Failed to init terminal")?,
  );
  if let Some(prev) = terminals.insert(name, terminal.clone()) {
    prev.cancel();
  }
  Ok(terminal)
}

pub async fn delete_terminal(name: &str) {
  if let Some(terminal) = terminals().write().await.remove(name) {
    terminal.cancel.cancel();
  }
}

pub async fn list_terminals(
  container: Option<&str>,
) -> Vec<TerminalInfo> {
  let mut terminals = terminals()
    .read()
    .await
    .iter()
    .filter(|(_, terminal)| {
      // If no container passed, returns all
      let Some(container) = container else {
        return true;
      };
      let Some(term_container) =
        terminal.container.as_ref().map(|c| c.0.as_str())
      else {
        return false;
      };
      term_container == container
    })
    .map(|(name, terminal)| TerminalInfo {
      name: name.to_string(),
      command: terminal.command.clone(),
      stored_size_kb: terminal.history.size_kb(),
    })
    .collect::<Vec<_>>();
  terminals.sort_by(|a, b| a.name.cmp(&b.name));
  terminals
}

pub async fn get_terminal(
  name: &str,
) -> anyhow::Result<Arc<Terminal>> {
  terminals()
    .read()
    .await
    .get(name)
    .cloned()
    .with_context(|| format!("No terminal at {name}"))
}

pub async fn clean_up_terminals() {
  terminals()
    .write()
    .await
    .retain(|_, terminal| !terminal.cancel.is_cancelled());
}

pub async fn delete_all_terminals() {
  terminals()
    .write()
    .await
    .drain()
    .for_each(|(_, terminal)| terminal.cancel());
  // The terminals poll cancel every 500 millis, need to wait for them
  // to finish cancelling.
  tokio::time::sleep(Duration::from_millis(100)).await;
}

#[derive(Clone, serde::Deserialize)]
pub struct ResizeDimensions {
  rows: u16,
  cols: u16,
}

#[derive(Clone)]
pub enum StdinMsg {
  Bytes(Vec<u8>),
  Resize(ResizeDimensions),
}

pub type StdinSender = mpsc::Sender<StdinMsg>;
pub type StdoutReceiver = broadcast::Receiver<Bytes>;

pub struct Terminal {
  /// The command that was used as the root command, eg `shell`
  command: String,

  pub cancel: CancellationToken,

  pub stdin: StdinSender,
  pub stdout: StdoutReceiver,

  pub history: Arc<History>,

  /// If terminal is for a container.
  pub container: Option<(String, ContainerTerminalMode)>,
}

impl Terminal {
  async fn new(
    command: String,
    container: Option<(String, ContainerTerminalMode)>,
  ) -> anyhow::Result<Terminal> {
    trace!("Creating terminal with command: {command}");

    let terminal = native_pty_system()
      .openpty(PtySize::default())
      .context("Failed to open terminal")?;

    let mut command_split = command.split(' ').map(|arg| arg.trim());
    let cmd =
      command_split.next().context("Command cannot be empty")?;

    let mut cmd = CommandBuilder::new(cmd);

    for arg in command_split {
      cmd.arg(arg);
    }

    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");

    let mut child = terminal
      .slave
      .spawn_command(cmd)
      .context("Failed to spawn child command")?;

    // Check the child didn't stop immediately (after a little wait) with error
    tokio::time::sleep(Duration::from_millis(100)).await;
    if let Some(status) = child
      .try_wait()
      .context("Failed to check child process exit status")?
    {
      return Err(anyhow!(
        "Child process exited immediately with code {}",
        status.exit_code()
      ));
    }

    let mut terminal_write = terminal
      .master
      .take_writer()
      .context("Failed to take terminal writer")?;
    let mut terminal_read = terminal
      .master
      .try_clone_reader()
      .context("Failed to clone terminal reader")?;

    let cancel = CancellationToken::new();

    // CHILD WAIT TASK
    let _cancel = cancel.clone();
    tokio::task::spawn_blocking(move || {
      loop {
        if _cancel.is_cancelled() {
          trace!("child wait handle cancelled from outside");
          if let Err(e) = child.kill() {
            debug!("Failed to kill child | {e:?}");
          }
          break;
        }
        match child.try_wait() {
          Ok(Some(code)) => {
            debug!("child exited with code {code}");
            _cancel.cancel();
            break;
          }
          Ok(None) => {
            std::thread::sleep(Duration::from_millis(500));
          }
          Err(e) => {
            debug!("failed to wait for child | {e:?}");
            _cancel.cancel();
            break;
          }
        }
      }
    });

    // WS (channel) -> STDIN TASK
    // Theres only one consumer here, so use mpsc
    let (stdin, mut channel_read) =
      tokio::sync::mpsc::channel::<StdinMsg>(8192);
    let _cancel = cancel.clone();
    tokio::task::spawn_blocking(move || {
      loop {
        if _cancel.is_cancelled() {
          trace!("terminal write: cancelled from outside");
          break;
        }
        match channel_read.blocking_recv() {
          Some(StdinMsg::Bytes(bytes)) => {
            if let Err(e) = terminal_write.write_all(&bytes) {
              debug!("Failed to write to PTY: {e:?}");
              _cancel.cancel();
              break;
            }
          }
          Some(StdinMsg::Resize(dimensions)) => {
            if let Err(e) = terminal.master.resize(PtySize {
              cols: dimensions.cols,
              rows: dimensions.rows,
              pixel_width: 0,
              pixel_height: 0,
            }) {
              debug!("Failed to resize | {e:?}");
              _cancel.cancel();
              break;
            };
          }
          None => {
            debug!("WS -> PTY channel read error: Disconnected");
            _cancel.cancel();
            break;
          }
        }
      }
    });

    let history = Arc::new(History::default());

    // PTY -> WS (channel) TASK
    // Uses broadcast to output to multiple client simultaneously
    let (write, stdout) =
      tokio::sync::broadcast::channel::<Bytes>(8192);
    let _cancel = cancel.clone();
    let _history = history.clone();
    tokio::task::spawn_blocking(move || {
      let mut buf = [0u8; 8192];
      loop {
        if _cancel.is_cancelled() {
          trace!("terminal read: cancelled from outside");
          break;
        }
        match terminal_read.read(&mut buf) {
          Ok(0) => {
            // EOF
            trace!("Got PTY read EOF");
            _cancel.cancel();
            break;
          }
          Ok(n) => {
            _history.push(&buf[..n]);
            if let Err(e) =
              write.send(Bytes::copy_from_slice(&buf[..n]))
            {
              debug!("PTY -> WS channel send error: {e:?}");
              _cancel.cancel();
              break;
            }
          }
          Err(e) => {
            debug!("Failed to read for PTY: {e:?}");
            _cancel.cancel();
            break;
          }
        }
      }
    });

    trace!("terminal tasks spawned");

    Ok(Terminal {
      command,
      cancel,
      stdin,
      stdout,
      history,
      container,
    })
  }

  pub fn cancel(&self) {
    trace!("Cancel called");
    self.cancel.cancel();
  }
}

/// 1 MiB rolling max history size per terminal
const MAX_BYTES: usize = 1024 * 1024;

pub struct History {
  buf: std::sync::RwLock<VecDeque<u8>>,
}

impl Default for History {
  fn default() -> Self {
    History {
      buf: VecDeque::with_capacity(MAX_BYTES).into(),
    }
  }
}

impl History {
  /// Push some bytes, evicting the oldest when full.
  fn push(&self, bytes: &[u8]) {
    let mut buf = self.buf.write().unwrap();
    for byte in bytes {
      if buf.len() == MAX_BYTES {
        buf.pop_front();
      }
      buf.push_back(*byte);
    }
  }

  pub fn bytes_parts(&self) -> (Bytes, Bytes) {
    let buf = self.buf.read().unwrap();
    let (a, b) = buf.as_slices();
    (Bytes::copy_from_slice(a), Bytes::copy_from_slice(b))
  }

  pub fn size_kb(&self) -> f64 {
    self.buf.read().unwrap().len() as f64 / 1024.0
  }
}
