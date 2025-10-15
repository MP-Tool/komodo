use komodo_client::{
  api::write::*,
  entities::{
    builder::Builder, permission::PermissionLevel, update::Update,
  },
};
use resolver_api::Resolve;

use crate::{permission::get_check_permissions, resource};

use super::WriteArgs;

impl Resolve<WriteArgs> for CreateBuilder {
  #[instrument("CreateBuilder", skip(user))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<Builder> {
    resource::create::<Builder>(&self.name, self.config, None, user)
      .await
  }
}

impl Resolve<WriteArgs> for CopyBuilder {
  #[instrument("CopyBuilder", skip(user))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<Builder> {
    let Builder { config, .. } = get_check_permissions::<Builder>(
      &self.id,
      user,
      PermissionLevel::Write.into(),
    )
    .await?;
    resource::create::<Builder>(&self.name, config.into(), None, user)
      .await
  }
}

impl Resolve<WriteArgs> for DeleteBuilder {
  #[instrument("DeleteBuilder", skip(user))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<Builder> {
    Ok(resource::delete::<Builder>(&self.id, user).await?)
  }
}

impl Resolve<WriteArgs> for UpdateBuilder {
  #[instrument("UpdateBuilder", skip(user))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<Builder> {
    Ok(
      resource::update::<Builder>(&self.id, self.config, user)
        .await?,
    )
  }
}

impl Resolve<WriteArgs> for RenameBuilder {
  #[instrument("RenameBuilder", skip(user))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<Update> {
    Ok(resource::rename::<Builder>(&self.id, &self.name, user).await?)
  }
}
