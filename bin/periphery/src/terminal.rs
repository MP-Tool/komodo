use std::{
  collections::{HashMap, VecDeque},
  sync::{Arc, OnceLock},
  time::Duration,
};

use anyhow::{Context, anyhow};
use bytes::Bytes;
use cache::CloneCache;
use komodo_client::{
  api::write::TerminalRecreateMode, entities::server::TerminalInfo,
};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use transport::message::{
  Decode,
  wrappers::{ChannelWrapper, WithChannel},
};
use uuid::Uuid;

#[derive(Debug)]
pub struct TerminalChannel {
  pub sender: mpsc::Sender<StdinMsg>,
  pub cancel: CancellationToken,
}

pub type TerminalChannels = CloneCache<Uuid, Arc<TerminalChannel>>;

pub fn terminal_channels() -> &'static TerminalChannels {
  static TERMINAL_CHANNELS: OnceLock<TerminalChannels> =
    OnceLock::new();
  TERMINAL_CHANNELS.get_or_init(Default::default)
}

#[derive(Debug)]
pub struct TerminalTrigger {
  sender: Mutex<Option<oneshot::Sender<()>>>,
  receiver: Mutex<Option<oneshot::Receiver<()>>>,
}

impl TerminalTrigger {
  /// This consumes the Trigger Sender.
  pub async fn send(&self) -> anyhow::Result<()> {
    let mut sender = self.sender.lock().await;
    let sender = sender
      .take()
      .context("Called TerminalTrigger 'send' more than once.")?;
    sender.send(()).map_err(|_| anyhow!("Sender already used"))
  }

  /// This consumes the Trigger Receiver.
  pub async fn wait(&self) -> anyhow::Result<()> {
    let mut receiver = self.receiver.lock().await;
    let receiver = receiver
      .take()
      .context("Called TerminalTrigger 'wait' more than once.")?;
    receiver.await.context("Failed to receive TerminalTrigger")
  }
}

/// Periphery must wait for Core to finish setting
/// up channel forwarding before sending message,
/// or the first sent messages may be missed.
#[derive(Default)]
pub struct TerminalTriggers(CloneCache<Uuid, Arc<TerminalTrigger>>);

pub fn terminal_triggers() -> &'static TerminalTriggers {
  static TERMINAL_TRIGGERS: OnceLock<TerminalTriggers> =
    OnceLock::new();
  TERMINAL_TRIGGERS.get_or_init(Default::default)
}

impl TerminalTriggers {
  pub async fn insert(&self, channel: Uuid) -> Arc<TerminalTrigger> {
    let (sender, receiver) = oneshot::channel();
    let trigger = Arc::new(TerminalTrigger {
      sender: Some(sender).into(),
      receiver: Some(receiver).into(),
    });
    self.0.insert(channel, trigger.clone()).await;
    trigger
  }

  pub async fn send(&self, channel: &Uuid) -> anyhow::Result<()> {
    let trigger = self.0.get(channel).await.with_context(|| {
      format!("No trigger found for channel {channel}")
    })?;
    trigger.send().await
  }

  pub async fn wait(&self, channel: &Uuid) -> anyhow::Result<()> {
    let trigger = self.0.get(channel).await.with_context(|| {
      format!("No trigger found for channel {channel}")
    })?;
    trigger.wait().await?;
    self.0.remove(channel).await;
    Ok(())
  }
}

pub async fn handle_message(message: ChannelWrapper<Bytes>) {
  let WithChannel {
    channel: channel_id,
    data,
  } = match message.decode() {
    Ok(res) => res,
    Err(e) => {
      warn!("Received invalid Terminal bytes | {e:#}");
      return;
    }
  };
  let msg = match data.first() {
    Some(&0x00) => {
      let mut data: Vec<_> = data.into();
      StdinMsg::Bytes(data.drain(1..).collect())
    }
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

type PtyName = String;
type PtyMap = tokio::sync::RwLock<HashMap<PtyName, Arc<Terminal>>>;
type StdinSender = mpsc::Sender<StdinMsg>;
type StdoutReceiver = broadcast::Receiver<Bytes>;

pub async fn create_terminal(
  name: String,
  command: String,
  recreate: TerminalRecreateMode,
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
    Terminal::new(command)
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

pub async fn list_terminals() -> Vec<TerminalInfo> {
  let mut terminals = terminals()
    .read()
    .await
    .iter()
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

fn terminals() -> &'static PtyMap {
  static TERMINALS: OnceLock<PtyMap> = OnceLock::new();
  TERMINALS.get_or_init(Default::default)
}

#[derive(Clone, serde::Deserialize)]
pub struct ResizeDimensions {
  rows: u16,
  cols: u16,
}

#[derive(Clone)]
pub enum StdinMsg {
  Bytes(Bytes),
  Resize(ResizeDimensions),
}

pub struct Terminal {
  /// The command that was used as the root command, eg `shell`
  command: String,

  pub cancel: CancellationToken,

  pub stdin: StdinSender,
  pub stdout: StdoutReceiver,

  pub history: Arc<History>,
}

impl Terminal {
  async fn new(command: String) -> anyhow::Result<Terminal> {
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
