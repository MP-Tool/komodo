use komodo_client::entities::{
  config::{DockerRegistry, GitProvider},
  docker::{
    container::ContainerListItem, image::ImageListItem,
    network::NetworkListItem, volume::VolumeListItem,
  },
  server::PeripheryInformation,
  stack::ComposeProject,
  stats::{SystemInformation, SystemStats},
  update::Log,
};
use resolver_api::Resolve;
use serde::{Deserialize, Serialize};

pub mod build;
pub mod compose;
pub mod container;
pub mod docker;
pub mod git;
pub mod keys;
pub mod stats;
pub mod terminal;

//

#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[response(GetHealthResponse)]
#[error(serror::Error)]
pub struct GetHealth {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetHealthResponse {}

//

#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[response(GetVersionResponse)]
#[error(serror::Error)]
pub struct GetVersion {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetVersionResponse {
  pub version: String,
}

//

/// This is the data Core uses to update all Server-related status caches.
#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[response(PollStatusResponse)]
#[error(serror::Error)]
pub struct PollStatus {
  /// Some servers have stats monitoring disabled.
  pub include_stats: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PollStatusResponse {
  pub periphery_info: PeripheryInformation,
  /// Basic system information
  pub system_info: SystemInformation,
  /// Current System Stats (Cpu, Mem, Disk)
  pub system_stats: Option<SystemStats>,

  // Docker lists
  pub containers: Vec<ContainerListItem>,
  pub networks: Vec<NetworkListItem>,
  pub images: Vec<ImageListItem>,
  pub volumes: Vec<VolumeListItem>,
  pub projects: Vec<ComposeProject>,
}

//

#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[response(ListGitProvidersResponse)]
#[error(serror::Error)]
pub struct ListGitProviders {}

pub type ListGitProvidersResponse = Vec<GitProvider>;

//

#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[response(ListDockerRegistriesResponse)]
#[error(serror::Error)]
pub struct ListDockerRegistries {}

pub type ListDockerRegistriesResponse = Vec<DockerRegistry>;

//

#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[response(Vec<String>)]
#[error(serror::Error)]
pub struct ListSecrets {}

//

#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[response(Log)]
#[error(serror::Error)]
pub struct PruneSystem {}
