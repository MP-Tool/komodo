use std::sync::{Arc, OnceLock};

use anyhow::Context;
use arc_swap::ArcSwap;
use cache::CloneCache;
use komodo_client::entities::{
  action::ActionState, build::BuildState,
  deployment::DeploymentState, procedure::ProcedureState,
  repo::RepoState, stack::StackState,
};

use crate::{
  auth::jwt::JwtClient,
  config::core_config,
  connection::PeripheryConnections,
  helpers::{
    action_state::ActionStates, all_resources::AllResourcesById,
  },
  monitor::{
    CachedDeploymentStatus, CachedRepoStatus, CachedServerStatus,
    CachedStackStatus, History,
  },
};

static DB_CLIENT: OnceLock<database::Client> = OnceLock::new();

pub fn db_client() -> &'static database::Client {
  DB_CLIENT
    .get()
    .expect("db_client accessed before initialized")
}

/// Must be called in app startup sequence.
pub async fn init_db_client() {
  let client = database::Client::new(&core_config().database)
    .await
    .context("failed to initialize database client")
    .unwrap();
  DB_CLIENT
    .set(client)
    .expect("db_client initialized more than once");
}

pub fn jwt_client() -> &'static JwtClient {
  static JWT_CLIENT: OnceLock<JwtClient> = OnceLock::new();
  JWT_CLIENT.get_or_init(|| match JwtClient::new(core_config()) {
    Ok(client) => client,
    Err(e) => {
      error!("failed to initialialize JwtClient | {e:#}");
      panic!("Exiting");
    }
  })
}

/// server id => connection
pub fn periphery_connections() -> &'static PeripheryConnections {
  static CONNECTIONS: OnceLock<PeripheryConnections> =
    OnceLock::new();
  CONNECTIONS.get_or_init(Default::default)
}

pub fn action_states() -> &'static ActionStates {
  static ACTION_STATES: OnceLock<ActionStates> = OnceLock::new();
  ACTION_STATES.get_or_init(ActionStates::default)
}

pub type ServerStatusCache =
  CloneCache<String, Arc<CachedServerStatus>>;

pub fn server_status_cache() -> &'static ServerStatusCache {
  static SERVER_STATUS_CACHE: OnceLock<ServerStatusCache> =
    OnceLock::new();
  SERVER_STATUS_CACHE.get_or_init(Default::default)
}

pub type StackStatusCache =
  CloneCache<String, Arc<History<CachedStackStatus, StackState>>>;

pub fn stack_status_cache() -> &'static StackStatusCache {
  static STACK_STATUS_CACHE: OnceLock<StackStatusCache> =
    OnceLock::new();
  STACK_STATUS_CACHE.get_or_init(Default::default)
}

/// Cache of ids to status
pub type DeploymentStatusCache = CloneCache<
  String,
  Arc<History<CachedDeploymentStatus, DeploymentState>>,
>;

/// Cache of ids to status
pub fn deployment_status_cache() -> &'static DeploymentStatusCache {
  static DEPLOYMENT_STATUS_CACHE: OnceLock<DeploymentStatusCache> =
    OnceLock::new();
  DEPLOYMENT_STATUS_CACHE.get_or_init(Default::default)
}

pub type BuildStateCache = CloneCache<String, BuildState>;

pub fn build_state_cache() -> &'static BuildStateCache {
  static BUILD_STATE_CACHE: OnceLock<BuildStateCache> =
    OnceLock::new();
  BUILD_STATE_CACHE.get_or_init(Default::default)
}

pub type RepoStatusCache = CloneCache<String, Arc<CachedRepoStatus>>;

pub fn repo_status_cache() -> &'static RepoStatusCache {
  static REPO_STATUS_CACHE: OnceLock<RepoStatusCache> =
    OnceLock::new();
  REPO_STATUS_CACHE.get_or_init(Default::default)
}

pub type RepoStateCache = CloneCache<String, RepoState>;

pub fn repo_state_cache() -> &'static RepoStateCache {
  static REPO_STATE_CACHE: OnceLock<RepoStateCache> = OnceLock::new();
  REPO_STATE_CACHE.get_or_init(Default::default)
}

pub type ProcedureStateCache = CloneCache<String, ProcedureState>;

pub fn procedure_state_cache() -> &'static ProcedureStateCache {
  static PROCEDURE_STATE_CACHE: OnceLock<ProcedureStateCache> =
    OnceLock::new();
  PROCEDURE_STATE_CACHE.get_or_init(Default::default)
}

pub type ActionStateCache = CloneCache<String, ActionState>;

pub fn action_state_cache() -> &'static ActionStateCache {
  static ACTION_STATE_CACHE: OnceLock<ActionStateCache> =
    OnceLock::new();
  ACTION_STATE_CACHE.get_or_init(Default::default)
}

pub fn all_resources_cache() -> &'static ArcSwap<AllResourcesById> {
  static ALL_RESOURCES: OnceLock<ArcSwap<AllResourcesById>> =
    OnceLock::new();
  ALL_RESOURCES.get_or_init(Default::default)
}
