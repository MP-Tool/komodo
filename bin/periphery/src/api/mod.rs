use command::run_komodo_command;
use derive_variants::EnumVariants;
use encoding::{EncodedJsonMessage, EncodedResponse};
use futures::FutureExt;
use komodo_client::entities::{
  config::{DockerRegistry, GitProvider},
  server::PeripheryInformation,
  stats::SystemProcess,
  update::Log,
};
use periphery_client::api::{
  build::*, compose::*, container::*, docker::*, git::*, keys::*,
  stats::*, terminal::*, *,
};
use resolver_api::Resolve;
use serde::{Deserialize, Serialize};

use crate::{
  api::compose::list_compose_projects,
  config::{periphery_config, periphery_keys},
  docker::docker_client,
  stats::stats_client,
};

pub mod terminal;

mod build;
mod compose;
mod container;
mod deploy;
mod docker;
mod git;
mod keys;

#[derive(Debug)]
pub struct Args {
  pub core: String,
}

#[derive(
  Serialize, Deserialize, Debug, Clone, Resolve, EnumVariants,
)]
#[args(Args)]
#[response(EncodedResponse<EncodedJsonMessage>)]
#[error(anyhow::Error)]
#[variant_derive(Debug)]
#[serde(tag = "type", content = "params")]
#[allow(clippy::enum_variant_names, clippy::large_enum_variant)]
pub enum PeripheryRequest {
  // Stats / Info (Read)
  PollStatus(PollStatus),
  GetHealth(GetHealth),
  GetVersion(GetVersion),
  GetSystemProcesses(GetSystemProcesses),
  GetLatestCommit(GetLatestCommit),

  // Config (Read)
  ListGitProviders(ListGitProviders),
  ListDockerRegistries(ListDockerRegistries),
  ListSecrets(ListSecrets),

  // Repo (Write)
  CloneRepo(CloneRepo),
  PullRepo(PullRepo),
  PullOrCloneRepo(PullOrCloneRepo),
  RenameRepo(RenameRepo),
  DeleteRepo(DeleteRepo),

  // Build
  GetDockerfileContentsOnHost(GetDockerfileContentsOnHost),
  WriteDockerfileContentsToHost(WriteDockerfileContentsToHost),
  Build(Build),
  PruneBuilders(PruneBuilders),
  PruneBuildx(PruneBuildx),

  // Compose (Read)
  GetComposeContentsOnHost(GetComposeContentsOnHost),
  GetComposeLog(GetComposeLog),
  GetComposeLogSearch(GetComposeLogSearch),

  // Compose (Write)
  WriteComposeContentsToHost(WriteComposeContentsToHost),
  WriteCommitComposeContents(WriteCommitComposeContents),
  ComposePull(ComposePull),
  ComposeUp(ComposeUp),
  ComposeExecution(ComposeExecution),
  ComposeRun(ComposeRun),

  // Container (Read)
  InspectContainer(InspectContainer),
  GetContainerLog(GetContainerLog),
  GetContainerLogSearch(GetContainerLogSearch),
  GetContainerStats(GetContainerStats),
  GetContainerStatsList(GetContainerStatsList),
  GetFullContainerStats(GetFullContainerStats),

  // Container (Write)
  Deploy(Deploy),
  StartContainer(StartContainer),
  RestartContainer(RestartContainer),
  PauseContainer(PauseContainer),
  UnpauseContainer(UnpauseContainer),
  StopContainer(StopContainer),
  StartAllContainers(StartAllContainers),
  RestartAllContainers(RestartAllContainers),
  PauseAllContainers(PauseAllContainers),
  UnpauseAllContainers(UnpauseAllContainers),
  StopAllContainers(StopAllContainers),
  RemoveContainer(RemoveContainer),
  RenameContainer(RenameContainer),
  PruneContainers(PruneContainers),

  // Networks (Read)
  InspectNetwork(InspectNetwork),

  // Networks (Write)
  CreateNetwork(CreateNetwork),
  DeleteNetwork(DeleteNetwork),
  PruneNetworks(PruneNetworks),

  // Image (Read)
  InspectImage(InspectImage),
  ImageHistory(ImageHistory),

  // Image (Write)
  PullImage(PullImage),
  DeleteImage(DeleteImage),
  PruneImages(PruneImages),

  // Volume (Read)
  InspectVolume(InspectVolume),

  // Volume (Write)
  DeleteVolume(DeleteVolume),
  PruneVolumes(PruneVolumes),

  // All in one (Write)
  PruneSystem(PruneSystem),

  // Terminal
  ListTerminals(ListTerminals),
  CreateTerminal(CreateTerminal),
  DeleteTerminal(DeleteTerminal),
  DeleteAllTerminals(DeleteAllTerminals),
  ConnectTerminal(ConnectTerminal),
  ConnectContainerExec(ConnectContainerExec),
  ConnectContainerAttach(ConnectContainerAttach),
  DisconnectTerminal(DisconnectTerminal),
  ExecuteTerminal(ExecuteTerminal),
  ExecuteContainerExec(ExecuteContainerExec),

  // Keys
  RotatePrivateKey(RotatePrivateKey),
  RotateCorePublicKey(RotateCorePublicKey),
}

//

impl Resolve<Args> for GetHealth {
  #[instrument(name = "GetHealth", level = "debug", skip_all)]
  async fn resolve(
    self,
    _: &Args,
  ) -> anyhow::Result<GetHealthResponse> {
    Ok(GetHealthResponse {})
  }
}

//

impl Resolve<Args> for GetVersion {
  #[instrument(name = "GetVersion", level = "debug", skip(self))]
  async fn resolve(
    self,
    _: &Args,
  ) -> anyhow::Result<GetVersionResponse> {
    Ok(GetVersionResponse {
      version: env!("CARGO_PKG_VERSION").to_string(),
    })
  }
}

//

impl Resolve<Args> for PollStatus {
  async fn resolve(
    self,
    _: &Args,
  ) -> anyhow::Result<PollStatusResponse> {
    // Docker lists
    let docker_lists = async {
      let client = docker_client().load();
      let Some(client) = client.iter().next() else {
        return Default::default();
      };
      let containers =
        client.list_containers().await.unwrap_or_default();
      // Todo: handle errors better
      (
        tokio::join!(
          client
            .list_networks(&containers)
            .map(Result::unwrap_or_default),
          client
            .list_images(&containers)
            .map(Result::unwrap_or_default),
          client
            .list_volumes(&containers)
            .map(Result::unwrap_or_default)
        ),
        containers,
      )
    };

    let (
      ((networks, images, volumes), containers),
      projects,
      stats_client,
    ) = tokio::join!(
      docker_lists,
      list_compose_projects().map(Result::unwrap_or_default),
      stats_client().read(),
    );

    let system_stats = if self.include_stats {
      Some(stats_client.stats.clone())
    } else {
      None
    };

    let config = periphery_config();

    Ok(PollStatusResponse {
      periphery_info: PeripheryInformation {
        version: env!("CARGO_PKG_VERSION").to_string(),
        public_key: periphery_keys().load().public.to_string(),
        terminals_disabled: config.disable_terminals,
        container_terminals_disabled: config
          .disable_container_terminals,
        stats_polling_rate: config.stats_polling_rate,
        docker_connected: docker_client().load().is_some(),
      },
      system_info: stats_client.info.clone(),
      system_stats,
      containers,
      networks,
      images,
      volumes,
      projects,
    })
  }
}

//

impl Resolve<Args> for GetSystemProcesses {
  #[instrument(name = "GetSystemProcesses", level = "debug")]
  async fn resolve(
    self,
    _: &Args,
  ) -> anyhow::Result<Vec<SystemProcess>> {
    Ok(stats_client().read().await.get_processes())
  }
}

//

impl Resolve<Args> for ListGitProviders {
  #[instrument(name = "ListGitProviders", level = "debug", skip_all)]
  async fn resolve(
    self,
    _: &Args,
  ) -> anyhow::Result<Vec<GitProvider>> {
    Ok(periphery_config().git_providers.0.clone())
  }
}

impl Resolve<Args> for ListDockerRegistries {
  #[instrument(
    name = "ListDockerRegistries",
    level = "debug",
    skip_all
  )]
  async fn resolve(
    self,
    _: &Args,
  ) -> anyhow::Result<Vec<DockerRegistry>> {
    Ok(periphery_config().docker_registries.0.clone())
  }
}

//

impl Resolve<Args> for ListSecrets {
  #[instrument(name = "ListSecrets", level = "debug", skip_all)]
  async fn resolve(self, _: &Args) -> anyhow::Result<Vec<String>> {
    Ok(
      periphery_config()
        .secrets
        .keys()
        .cloned()
        .collect::<Vec<_>>(),
    )
  }
}

impl Resolve<Args> for PruneSystem {
  #[instrument(name = "PruneSystem", skip_all)]
  async fn resolve(self, _: &Args) -> anyhow::Result<Log> {
    let command = String::from("docker system prune -a -f --volumes");
    Ok(run_komodo_command("Prune System", None, command).await)
  }
}
