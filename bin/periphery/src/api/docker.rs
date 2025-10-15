use std::sync::OnceLock;

use anyhow::Context;
use cache::TimeoutCache;
use command::run_komodo_command;
use komodo_client::entities::{
  deployment::extract_registry_domain,
  docker::{
    image::{Image, ImageHistoryResponseItem},
    network::Network,
    volume::Volume,
  },
  komodo_timestamp,
  update::Log,
};
use periphery_client::api::docker::*;
use resolver_api::Resolve;

use crate::{docker::docker_login, state::docker_client};

// =====
// IMAGE
// =====

impl Resolve<super::Args> for InspectImage {
  async fn resolve(self, _: &super::Args) -> anyhow::Result<Image> {
    let client = docker_client().load();
    let client = client
      .iter()
      .next()
      .context("Could not connect to docker client")?;
    client.inspect_image(&self.name).await
  }
}

//

impl Resolve<super::Args> for ImageHistory {
  async fn resolve(
    self,
    _: &super::Args,
  ) -> anyhow::Result<Vec<ImageHistoryResponseItem>> {
    let client = docker_client().load();
    let client = client
      .iter()
      .next()
      .context("Could not connect to docker client")?;
    client.image_history(&self.name).await
  }
}

//

/// Wait this long after a pull to allow another pull through
const PULL_TIMEOUT: i64 = 5_000;

fn pull_cache() -> &'static TimeoutCache<String, Log> {
  static PULL_CACHE: OnceLock<TimeoutCache<String, Log>> =
    OnceLock::new();
  PULL_CACHE.get_or_init(Default::default)
}

impl Resolve<super::Args> for PullImage {
  #[instrument("PullImage", skip_all, fields(name = self.name, core = args.core))]
  async fn resolve(self, args: &super::Args) -> anyhow::Result<Log> {
    let PullImage {
      name,
      account,
      token,
    } = self;
    // Acquire the image lock
    let lock = pull_cache().get_lock(name.clone()).await;

    // Lock the image lock, prevents simultaneous pulls by
    // ensuring simultaneous pulls will wait for first to finish
    // and checking cached results.
    let mut locked = lock.lock().await;

    // Early return from cache if lasted pulled with PULL_TIMEOUT
    if locked.last_ts + PULL_TIMEOUT > komodo_timestamp() {
      return locked.clone_res();
    }

    let res = async {
      docker_login(
        &extract_registry_domain(&name)?,
        account.as_deref().unwrap_or_default(),
        token.as_deref(),
      )
      .await?;
      anyhow::Ok(
        run_komodo_command(
          "Docker Pull",
          None,
          format!("docker pull {name}"),
        )
        .await,
      )
    }
    .await;

    // Set the cache with results. Any other calls waiting on the lock will
    // then immediately also use this same result.
    locked.set(&res, komodo_timestamp());

    res
  }
}

//

impl Resolve<super::Args> for DeleteImage {
  #[instrument("DeleteImage", skip_all, fields(image_name = self.name, core = args.core))]
  async fn resolve(self, args: &super::Args) -> anyhow::Result<Log> {
    let command = format!("docker image rm {}", self.name);
    Ok(run_komodo_command("Delete Image", None, command).await)
  }
}

//

impl Resolve<super::Args> for PruneImages {
  #[instrument("PruneImages", skip_all, fields(core = args.core))]
  async fn resolve(self, args: &super::Args) -> anyhow::Result<Log> {
    let command = String::from("docker image prune -a -f");
    Ok(run_komodo_command("Prune Images", None, command).await)
  }
}

// =======
// NETWORK
// =======

impl Resolve<super::Args> for InspectNetwork {
  async fn resolve(self, _: &super::Args) -> anyhow::Result<Network> {
    let client = docker_client().load();
    let client = client
      .iter()
      .next()
      .context("Could not connect to docker client")?;
    client.inspect_network(&self.name).await
  }
}

//

impl Resolve<super::Args> for CreateNetwork {
  #[instrument("CreateNetwork", skip(args), fields(core = args.core))]
  async fn resolve(self, args: &super::Args) -> anyhow::Result<Log> {
    let CreateNetwork { name, driver } = self;
    let driver = match driver {
      Some(driver) => format!(" -d {driver}"),
      None => String::new(),
    };
    let command = format!("docker network create{driver} {name}");
    Ok(run_komodo_command("Create Network", None, command).await)
  }
}

//

impl Resolve<super::Args> for DeleteNetwork {
  #[instrument("DeleteNetwork", skip_all, fields(network_name = self.name, core = args.core))]
  async fn resolve(self, args: &super::Args) -> anyhow::Result<Log> {
    let command = format!("docker network rm {}", self.name);
    Ok(run_komodo_command("Delete Network", None, command).await)
  }
}

//

impl Resolve<super::Args> for PruneNetworks {
  #[instrument("PruneNetworks", skip_all, fields(core = args.core))]
  async fn resolve(self, args: &super::Args) -> anyhow::Result<Log> {
    let command = String::from("docker network prune -f");
    Ok(run_komodo_command("Prune Networks", None, command).await)
  }
}

// ======
// VOLUME
// ======

impl Resolve<super::Args> for InspectVolume {
  async fn resolve(self, _: &super::Args) -> anyhow::Result<Volume> {
    let client = docker_client().load();
    let client = client
      .iter()
      .next()
      .context("Could not connect to docker client")?;
    client.inspect_volume(&self.name).await
  }
}

//

impl Resolve<super::Args> for DeleteVolume {
  #[instrument("DeleteVolume", skip_all, fields(volume_name = self.name, core = args.core))]
  async fn resolve(self, args: &super::Args) -> anyhow::Result<Log> {
    let command = format!("docker volume rm {}", self.name);
    Ok(run_komodo_command("Delete Volume", None, command).await)
  }
}

//

impl Resolve<super::Args> for PruneVolumes {
  #[instrument("PruneVolumes", skip_all, fields(core = args.core))]
  async fn resolve(self, args: &super::Args) -> anyhow::Result<Log> {
    let command = String::from("docker volume prune -a -f");
    Ok(run_komodo_command("Prune Volumes", None, command).await)
  }
}
