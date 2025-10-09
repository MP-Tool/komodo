use std::sync::OnceLock;

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

use crate::docker::{docker_client, docker_login};

// =====
// IMAGE
// =====

impl Resolve<super::Args> for InspectImage {
  #[instrument(name = "InspectImage", level = "debug")]
  async fn resolve(self, _: &super::Args) -> anyhow::Result<Image> {
    Ok(docker_client().inspect_image(&self.name).await?)
  }
}

//

impl Resolve<super::Args> for ImageHistory {
  #[instrument(name = "ImageHistory", level = "debug")]
  async fn resolve(
    self,
    _: &super::Args,
  ) -> anyhow::Result<Vec<ImageHistoryResponseItem>> {
    Ok(docker_client().image_history(&self.name).await?)
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
  #[instrument(name = "PullImage", skip_all, fields(name = &self.name))]
  async fn resolve(self, _: &super::Args) -> anyhow::Result<Log> {
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
      return locked.clone_res().map_err(Into::into);
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

    res.map_err(Into::into)
  }
}

//

impl Resolve<super::Args> for DeleteImage {
  #[instrument(name = "DeleteImage")]
  async fn resolve(self, _: &super::Args) -> anyhow::Result<Log> {
    let command = format!("docker image rm {}", self.name);
    Ok(run_komodo_command("Delete Image", None, command).await)
  }
}

//

impl Resolve<super::Args> for PruneImages {
  #[instrument(name = "PruneImages")]
  async fn resolve(self, _: &super::Args) -> anyhow::Result<Log> {
    let command = String::from("docker image prune -a -f");
    Ok(run_komodo_command("Prune Images", None, command).await)
  }
}

// =======
// NETWORK
// =======

impl Resolve<super::Args> for InspectNetwork {
  #[instrument(name = "InspectNetwork", level = "debug")]
  async fn resolve(self, _: &super::Args) -> anyhow::Result<Network> {
    Ok(docker_client().inspect_network(&self.name).await?)
  }
}

//

impl Resolve<super::Args> for CreateNetwork {
  #[instrument(name = "CreateNetwork", skip(self))]
  async fn resolve(self, _: &super::Args) -> anyhow::Result<Log> {
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
  #[instrument(name = "DeleteNetwork", skip(self))]
  async fn resolve(self, _: &super::Args) -> anyhow::Result<Log> {
    let command = format!("docker network rm {}", self.name);
    Ok(run_komodo_command("Delete Network", None, command).await)
  }
}

//

impl Resolve<super::Args> for PruneNetworks {
  #[instrument(name = "PruneNetworks", skip(self))]
  async fn resolve(self, _: &super::Args) -> anyhow::Result<Log> {
    let command = String::from("docker network prune -f");
    Ok(run_komodo_command("Prune Networks", None, command).await)
  }
}

// ======
// VOLUME
// ======

impl Resolve<super::Args> for InspectVolume {
  #[instrument(name = "InspectVolume", level = "debug")]
  async fn resolve(self, _: &super::Args) -> anyhow::Result<Volume> {
    Ok(docker_client().inspect_volume(&self.name).await?)
  }
}

//

impl Resolve<super::Args> for DeleteVolume {
  #[instrument(name = "DeleteVolume")]
  async fn resolve(self, _: &super::Args) -> anyhow::Result<Log> {
    let command = format!("docker volume rm {}", self.name);
    Ok(run_komodo_command("Delete Volume", None, command).await)
  }
}

//

impl Resolve<super::Args> for PruneVolumes {
  #[instrument(name = "PruneVolumes")]
  async fn resolve(self, _: &super::Args) -> anyhow::Result<Log> {
    let command = String::from("docker volume prune -a -f");
    Ok(run_komodo_command("Prune Volumes", None, command).await)
  }
}
