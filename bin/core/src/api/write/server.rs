use anyhow::Context;
use formatting::format_serror;
use komodo_client::{
  api::write::*,
  entities::{
    NoData, Operation,
    permission::PermissionLevel,
    server::{PartialServerConfig, Server},
    to_docker_compatible_name,
    update::{Update, UpdateStatus},
  },
};
use periphery_client::api;
use resolver_api::Resolve;

use crate::{
  helpers::{
    periphery_client,
    update::{add_update, make_update, update_update},
  },
  permission::get_check_permissions,
  resource,
};

use super::WriteArgs;

impl Resolve<WriteArgs> for CreateServer {
  #[instrument(name = "CreateServer", skip(user))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<Server> {
    resource::create::<Server>(&self.name, self.config, user).await
  }
}

impl Resolve<WriteArgs> for CopyServer {
  #[instrument(name = "CopyServer", skip(user))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<Server> {
    let Server { config, .. } = get_check_permissions::<Server>(
      &self.id,
      user,
      PermissionLevel::Read.into(),
    )
    .await?;

    resource::create::<Server>(&self.name, config.into(), user).await
  }
}

impl Resolve<WriteArgs> for DeleteServer {
  #[instrument(name = "DeleteServer", skip(args))]
  async fn resolve(self, args: &WriteArgs) -> serror::Result<Server> {
    Ok(resource::delete::<Server>(&self.id, args).await?)
  }
}

impl Resolve<WriteArgs> for UpdateServer {
  #[instrument(name = "UpdateServer", skip(user))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<Server> {
    Ok(resource::update::<Server>(&self.id, self.config, user).await?)
  }
}

impl Resolve<WriteArgs> for RenameServer {
  #[instrument(name = "RenameServer", skip(user))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<Update> {
    Ok(resource::rename::<Server>(&self.id, &self.name, user).await?)
  }
}

impl Resolve<WriteArgs> for CreateNetwork {
  #[instrument(name = "CreateNetwork", skip(user))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<Update> {
    let server = get_check_permissions::<Server>(
      &self.server,
      user,
      PermissionLevel::Write.into(),
    )
    .await?;

    let periphery = periphery_client(&server).await?;

    let mut update =
      make_update(&server, Operation::CreateNetwork, user);
    update.status = UpdateStatus::InProgress;
    update.id = add_update(update.clone()).await?;

    match periphery
      .request(api::docker::CreateNetwork {
        name: to_docker_compatible_name(&self.name),
        driver: None,
      })
      .await
    {
      Ok(log) => update.logs.push(log),
      Err(e) => update.push_error_log(
        "create network",
        format_serror(&e.context("Failed to create network").into()),
      ),
    };

    update.finalize();
    update_update(update.clone()).await?;

    Ok(update)
  }
}

impl Resolve<WriteArgs> for CreateTerminal {
  #[instrument(name = "CreateTerminal", skip(user))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<NoData> {
    let server = get_check_permissions::<Server>(
      &self.server,
      user,
      PermissionLevel::Write.terminal(),
    )
    .await?;

    let periphery = periphery_client(&server).await?;

    periphery
      .request(api::terminal::CreateTerminal {
        name: self.name,
        command: self.command,
        recreate: self.recreate,
      })
      .await
      .context("Failed to create terminal on Periphery")?;

    Ok(NoData {})
  }
}

impl Resolve<WriteArgs> for DeleteTerminal {
  #[instrument(name = "DeleteTerminal", skip(user))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<NoData> {
    let server = get_check_permissions::<Server>(
      &self.server,
      user,
      PermissionLevel::Write.terminal(),
    )
    .await?;

    let periphery = periphery_client(&server).await?;

    periphery
      .request(api::terminal::DeleteTerminal {
        terminal: self.terminal,
      })
      .await
      .context("Failed to delete terminal on Periphery")?;

    Ok(NoData {})
  }
}

impl Resolve<WriteArgs> for DeleteAllTerminals {
  #[instrument(name = "DeleteAllTerminals", skip(user))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<NoData> {
    let server = get_check_permissions::<Server>(
      &self.server,
      user,
      PermissionLevel::Write.terminal(),
    )
    .await?;

    let periphery = periphery_client(&server).await?;

    periphery
      .request(api::terminal::DeleteAllTerminals {})
      .await
      .context("Failed to delete all terminals on Periphery")?;

    Ok(NoData {})
  }
}

//

impl Resolve<WriteArgs> for RotateServerPrivateKey {
  #[instrument(name = "RotateServerPrivateKey", skip(args))]
  async fn resolve(self, args: &WriteArgs) -> serror::Result<NoData> {
    let server = get_check_permissions::<Server>(
      &self.server,
      &args.user,
      PermissionLevel::Write.into(),
    )
    .await?;

    let periphery = periphery_client(&server).await?;

    let periphery_public_key = periphery
      .request(api::keys::RotatePrivateKey {})
      .await
      .context("Failed to rotate Periphery private key")?
      .public_key
      .into();

    UpdateServer {
      id: server.id,
      config: PartialServerConfig {
        periphery_public_key,
        ..Default::default()
      },
    }
    .resolve(args)
    .await?;

    Ok(NoData {})
  }
}
