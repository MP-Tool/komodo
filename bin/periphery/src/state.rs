use std::{
  collections::HashMap,
  sync::{Arc, OnceLock},
};

use anyhow::{Context, anyhow};
use arc_swap::ArcSwap;
use cache::CloneCache;
use komodo_client::entities::docker::container::ContainerStats;
use noise::key::{RotatableKeyPair, SpkiPublicKey};
use periphery_client::transport::EncodedTransportMessage;
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use transport::channel::BufferedChannel;
use uuid::Uuid;

use crate::{
  config::periphery_config,
  docker::DockerClient,
  stats::StatsClient,
  terminal::{StdinMsg, Terminal},
};

/// Should call in startup to ensure Periphery errors without valid private key.
pub fn periphery_keys() -> &'static RotatableKeyPair {
  static PERIPHERY_KEYS: OnceLock<RotatableKeyPair> = OnceLock::new();
  PERIPHERY_KEYS.get_or_init(|| {
    let config = periphery_config();
    if let Some(private_key_spec) = config.private_key.as_deref() {
      RotatableKeyPair::from_private_key_spec(private_key_spec)
    } else {
      RotatableKeyPair::from_private_key_spec(&format!(
        "file:{}",
        config
          .root_directory
          .join("keys/periphery.key")
          .to_str()
          .expect("Invalid root directory")
      ))
    }
    .unwrap()
  })
}

pub fn core_public_keys() -> &'static CorePublicKeys {
  static CORE_PUBLIC_KEYS: OnceLock<CorePublicKeys> = OnceLock::new();
  CORE_PUBLIC_KEYS.get_or_init(CorePublicKeys::default)
}

pub struct CorePublicKeys(ArcSwap<Vec<SpkiPublicKey>>);

impl Default for CorePublicKeys {
  fn default() -> Self {
    let keys = CorePublicKeys(Default::default());
    keys.refresh();
    keys
  }
}

impl CorePublicKeys {
  pub fn load(&self) -> arc_swap::Guard<Arc<Vec<SpkiPublicKey>>> {
    self.0.load()
  }

  pub fn is_valid(&self, public_key: &str) -> bool {
    let keys = self.0.load();
    keys.is_empty() || keys.iter().any(|pk| pk.as_str() == public_key)
  }

  pub fn refresh(&self) {
    let config = periphery_config();
    let Some(core_public_keys) = config.core_public_keys.as_ref()
    else {
      return;
    };
    let core_public_keys = core_public_keys
      .iter()
      .flat_map(|public_key| {
        let res = if let Some(path) = public_key.strip_prefix("file:")
        {
          SpkiPublicKey::from_file(path)
        } else {
          SpkiPublicKey::from_maybe_pem(public_key)
        };
        match (res, config.server_enabled) {
          (Ok(public_key), _) => Some(public_key),
          (Err(e), false) => {
            // If only outbound connections, only warn.
            // It will be written the next time `RotateCoreKeys` is executed.
            warn!("{e:#}");
            None
          }
          (Err(e), true) => {
            // This is too dangerous to allow if server_enabled.
            panic!("{e:#}");
          }
        }
      })
      .collect::<Vec<_>>();
    self.0.store(Arc::new(core_public_keys));
  }
}

// Core Address / Host -> Channel
pub type CoreConnection = BufferedChannel<EncodedTransportMessage>;
pub type CoreConnections = CloneCache<String, Arc<CoreConnection>>;

pub fn core_connections() -> &'static CoreConnections {
  static CORE_CONNECTIONS: OnceLock<CoreConnections> =
    OnceLock::new();
  CORE_CONNECTIONS.get_or_init(Default::default)
}

pub fn stats_client() -> &'static RwLock<StatsClient> {
  static STATS_CLIENT: OnceLock<RwLock<StatsClient>> =
    OnceLock::new();
  STATS_CLIENT.get_or_init(|| RwLock::new(StatsClient::default()))
}

pub type PtyName = String;
pub type PtyMap =
  tokio::sync::RwLock<HashMap<PtyName, Arc<Terminal>>>;

pub fn terminals() -> &'static PtyMap {
  static TERMINALS: OnceLock<PtyMap> = OnceLock::new();
  TERMINALS.get_or_init(Default::default)
}

pub type TerminalChannels = CloneCache<Uuid, Arc<TerminalChannel>>;

pub fn terminal_channels() -> &'static TerminalChannels {
  static TERMINAL_CHANNELS: OnceLock<TerminalChannels> =
    OnceLock::new();
  TERMINAL_CHANNELS.get_or_init(Default::default)
}

#[derive(Debug)]
pub struct TerminalChannel {
  pub sender: mpsc::Sender<StdinMsg>,
  pub cancel: CancellationToken,
}

pub fn terminal_triggers() -> &'static TerminalTriggers {
  static TERMINAL_TRIGGERS: OnceLock<TerminalTriggers> =
    OnceLock::new();
  TERMINAL_TRIGGERS.get_or_init(Default::default)
}

/// Periphery must wait for Core to finish setting
/// up channel forwarding before sending message,
/// or the first sent messages may be missed.
#[derive(Default)]
pub struct TerminalTriggers(CloneCache<Uuid, Arc<TerminalTrigger>>);

impl TerminalTriggers {
  pub async fn insert(&self, channel: Uuid) {
    let (sender, receiver) = oneshot::channel();
    let trigger = Arc::new(TerminalTrigger {
      sender: Some(sender).into(),
      receiver: Some(receiver).into(),
    });
    self.0.insert(channel, trigger).await;
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
    sender
      .send(())
      .map_err(|_| anyhow!("TerminalTrigger sender already used"))
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

pub fn docker_client() -> &'static SwappableDockerClient {
  static DOCKER_CLIENT: OnceLock<SwappableDockerClient> =
    OnceLock::new();
  DOCKER_CLIENT.get_or_init(SwappableDockerClient::init)
}

#[derive(Default)]
pub struct SwappableDockerClient(ArcSwap<Option<DockerClient>>);

impl SwappableDockerClient {
  pub fn init() -> Self {
    let docker = DockerClient::connect()
      // Only logs on first init, although keeps trying to connect
      .inspect_err(|e| warn!("{e:#}"))
      .ok();
    Self(ArcSwap::new(Arc::new(docker)))
  }

  pub fn load(&self) -> arc_swap::Guard<Arc<Option<DockerClient>>> {
    let res = self.0.load();
    if res.is_some() {
      return res;
    }
    self.reload();
    self.0.load()
  }

  pub fn reload(&self) {
    self.0.store(Arc::new(DockerClient::connect().ok()));
  }
}

pub type ContainerStatsMap = HashMap<String, ContainerStats>;

pub fn container_stats() -> &'static ArcSwap<ContainerStatsMap> {
  static CONTAINER_STATS: OnceLock<ArcSwap<ContainerStatsMap>> =
    OnceLock::new();
  CONTAINER_STATS.get_or_init(Default::default)
}
