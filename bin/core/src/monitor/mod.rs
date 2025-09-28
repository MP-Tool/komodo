use std::sync::{Arc, OnceLock};

use async_timing_util::wait_until_timelength;
use cache::CloneCache;
use database::mungos::{find::find_collect, mongodb::bson::doc};
use futures::future::join_all;
use helpers::insert_stacks_status_unknown;
use komodo_client::entities::{
  build::Build,
  deployment::{Deployment, DeploymentState},
  docker::{
    container::ContainerListItem, image::ImageListItem,
    network::NetworkListItem, volume::VolumeListItem,
  },
  komodo_timestamp, optional_string,
  repo::Repo,
  server::{PeripheryInformation, Server, ServerHealth, ServerState},
  stack::{ComposeProject, Stack, StackService, StackState},
  stats::{SystemInformation, SystemStats},
};
use periphery_client::api::{
  self, PollStatusResponse, git::GetLatestCommit,
};
use serror::Serror;
use tokio::sync::Mutex;

use crate::{
  config::core_config,
  helpers::periphery_client,
  monitor::{alert::check_alerts, record::record_server_stats},
  state::{db_client, deployment_status_cache, repo_status_cache},
};

use self::helpers::{
  insert_deployments_status_unknown, insert_repos_status_unknown,
  insert_server_status,
};

mod alert;
mod helpers;
mod record;
mod resources;

#[derive(Default, Debug)]
pub struct History<Curr: Default, Prev> {
  pub curr: Curr,
  pub prev: Option<Prev>,
}

#[derive(Default, Clone, Debug)]
pub struct CachedServerStatus {
  pub id: String,
  pub state: ServerState,
  pub health: Option<ServerHealth>,
  pub periphery_info: Option<PeripheryInformation>,
  pub system_info: Option<SystemInformation>,
  pub system_stats: Option<SystemStats>,
  pub containers: Option<Vec<ContainerListItem>>,
  pub networks: Option<Vec<NetworkListItem>>,
  pub images: Option<Vec<ImageListItem>>,
  pub volumes: Option<Vec<VolumeListItem>>,
  pub projects: Option<Vec<ComposeProject>>,
  /// Store the error in reaching periphery
  pub err: Option<serror::Serror>,
}

#[derive(Default, Clone, Debug)]
pub struct CachedDeploymentStatus {
  /// The deployment id
  pub id: String,
  pub state: DeploymentState,
  pub container: Option<ContainerListItem>,
  pub update_available: bool,
}

#[derive(Default, Clone, Debug)]
pub struct CachedRepoStatus {
  pub latest_hash: Option<String>,
  pub latest_message: Option<String>,
}

#[derive(Default, Clone, Debug)]
pub struct CachedStackStatus {
  /// The stack id
  pub id: String,
  /// The stack state
  pub state: StackState,
  /// The services connected to the stack
  pub services: Vec<StackService>,
}

const ADDITIONAL_MS: u128 = 500;

pub fn spawn_monitor_loop() {
  let interval: async_timing_util::Timelength = core_config()
    .monitoring_interval
    .try_into()
    .expect("Invalid monitoring interval");
  tokio::spawn(async move {
    refresh_server_cache(komodo_timestamp()).await;
    loop {
      let ts = (wait_until_timelength(interval, ADDITIONAL_MS).await
        - ADDITIONAL_MS) as i64;
      refresh_server_cache(ts).await;
    }
  });
}

async fn refresh_server_cache(ts: i64) {
  let servers =
    match find_collect(&db_client().servers, None, None).await {
      Ok(servers) => servers,
      Err(e) => {
        error!(
          "failed to get server list (manage status cache) | {e:#}"
        );
        return;
      }
    };
  let futures = servers.into_iter().map(|server| async move {
    update_cache_for_server(&server, false).await;
  });
  join_all(futures).await;
  tokio::join!(check_alerts(ts), record_server_stats(ts));
}

/// Makes sure cache for server doesn't update too frequently / simultaneously.
/// If forced, will still block against simultaneous update.
fn update_cache_for_server_controller()
-> &'static CloneCache<String, Arc<Mutex<i64>>> {
  static CACHE: OnceLock<CloneCache<String, Arc<Mutex<i64>>>> =
    OnceLock::new();
  CACHE.get_or_init(Default::default)
}

/// The background loop will call this with force: false,
/// which exits early if the lock is busy or it was completed too recently.
/// If force is true, it will wait on simultaneous calls, and will
/// ignore the restriction on being completed too recently.
#[instrument(level = "debug")]
pub async fn update_cache_for_server(server: &Server, force: bool) {
  // Concurrency controller to ensure it isn't done too often
  // when it happens in other contexts.
  let controller = update_cache_for_server_controller()
    .get_or_insert_default(&server.id)
    .await;
  let mut lock = match controller.try_lock() {
    Ok(lock) => lock,
    Err(_) if force => controller.lock().await,
    Err(_) => return,
  };

  let now = komodo_timestamp();

  // early return if called again sooner than 1s.
  if !force && *lock > now - 1_000 {
    return;
  }

  *lock = now;

  let resources = UpdateCacheResources::load(server).await;

  // Handle server disabled
  if !server.config.enabled {
    resources.insert_status_unknown().await;
    insert_server_status(
      server,
      ServerState::Disabled,
      None,
      None,
      None,
      (None, None, None, None, None),
      None,
    )
    .await;
    return;
  }

  let periphery = match periphery_client(server).await {
    Ok(periphery) => periphery,
    Err(e) => {
      resources.insert_status_unknown().await;
      insert_server_status(
        server,
        ServerState::NotOk,
        None,
        None,
        None,
        (None, None, None, None, None),
        Serror::from(&e),
      )
      .await;
      return;
    }
  };

  let PollStatusResponse {
    periphery_info,
    system_info,
    system_stats,
    mut containers,
    networks,
    images,
    volumes,
    projects,
  } = match periphery
    .request(api::PollStatus {
      include_stats: server.config.stats_monitoring,
    })
    .await
  {
    Ok(info) => info,
    Err(e) => {
      resources.insert_status_unknown().await;
      insert_server_status(
        server,
        ServerState::NotOk,
        None,
        None,
        None,
        (None, None, None, None, None),
        Serror::from(&e),
      )
      .await;
      return;
    }
  };

  containers.iter_mut().for_each(|container| {
    container.server_id = Some(server.id.clone())
  });
  tokio::join!(
    resources::update_deployment_cache(
      server.name.clone(),
      resources.deployments,
      &containers,
      &images,
      &resources.builds,
    ),
    resources::update_stack_cache(
      server.name.clone(),
      resources.stacks,
      &containers,
      &images
    ),
  );
  insert_server_status(
    server,
    ServerState::Ok,
    Some(periphery_info),
    Some(system_info),
    system_stats.map(|stats| filter_volumes(server, stats)),
    (
      Some(containers.clone()),
      Some(networks),
      Some(images),
      Some(volumes),
      Some(projects),
    ),
    None,
  )
  .await;

  let status_cache = repo_status_cache();
  for repo in resources.repos {
    let (latest_hash, latest_message) = periphery
      .request(GetLatestCommit {
        name: repo.name.clone(),
        path: optional_string(&repo.config.path),
      })
      .await
      .ok()
      .flatten()
      .map(|c| (c.hash, c.message))
      .unzip();
    status_cache
      .insert(
        repo.id,
        CachedRepoStatus {
          latest_hash,
          latest_message,
        }
        .into(),
      )
      .await;
  }
}

struct UpdateCacheResources {
  stacks: Vec<Stack>,
  deployments: Vec<Deployment>,
  builds: Vec<Build>,
  repos: Vec<Repo>,
}

impl UpdateCacheResources {
  pub async fn load(server: &Server) -> Self {
    let (stacks, deployments, builds, repos) = tokio::join!(
      find_collect(
        &db_client().stacks,
        doc! { "config.server_id": &server.id },
        None,
      ),
      find_collect(
        &db_client().deployments,
        doc! { "config.server_id": &server.id },
        None,
      ),
      find_collect(&db_client().builds, doc! {}, None,),
      find_collect(
        &db_client().repos,
        doc! { "config.server_id": &server.id },
        None,
      ),
    );

    let stacks = stacks.inspect_err(|e|  error!("failed to get stacks list from db (update status cache) | server: {} | {e:#}", server.name)).unwrap_or_default();
    let deployments =  deployments.inspect_err(|e| error!("failed to get deployments list from db (update status cache) | server : {} | {e:#}", server.name)).unwrap_or_default();
    let builds =  builds.inspect_err(|e| error!("failed to get builds list from db (update status cache) | server : {} | {e:#}", server.name)).unwrap_or_default();
    let repos = repos.inspect_err(|e|  error!("failed to get repos list from db (update status cache) | server: {} | {e:#}", server.name)).unwrap_or_default();

    Self {
      stacks,
      deployments,
      builds,
      repos,
    }
  }

  pub async fn insert_status_unknown(self) {
    insert_stacks_status_unknown(self.stacks).await;
    insert_deployments_status_unknown(self.deployments).await;
    insert_repos_status_unknown(self.repos).await;
  }
}

fn filter_volumes(
  server: &Server,
  mut stats: SystemStats,
) -> SystemStats {
  stats.disks.retain(|disk| {
    // Always filter out volume mounts
    !disk.mount.starts_with("/var/lib/docker/volumes")
    // Filter out any that were declared to ignore in server config
      && !server
        .config
        .ignore_mounts
        .iter()
        .any(|mount| disk.mount.starts_with(mount))
  });
  stats
}
