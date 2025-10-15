use komodo_client::{
  api::write::*,
  entities::{
    action::Action, permission::PermissionLevel, update::Update,
  },
};
use resolver_api::Resolve;

use crate::{permission::get_check_permissions, resource};

use super::WriteArgs;

impl Resolve<WriteArgs> for CreateAction {
  #[instrument("CreateAction", skip(user), fields(user_id = user.id))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<Action> {
    resource::create::<Action>(&self.name, self.config, None, user)
      .await
  }
}

impl Resolve<WriteArgs> for CopyAction {
  #[instrument("CopyAction", skip(user), fields(user_id = user.id))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<Action> {
    let Action { config, .. } = get_check_permissions::<Action>(
      &self.id,
      user,
      PermissionLevel::Write.into(),
    )
    .await?;
    resource::create::<Action>(&self.name, config.into(), None, user)
      .await
  }
}

impl Resolve<WriteArgs> for UpdateAction {
  #[instrument("UpdateAction", skip(user), fields(user_id = user.id))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<Action> {
    Ok(resource::update::<Action>(&self.id, self.config, user).await?)
  }
}

impl Resolve<WriteArgs> for RenameAction {
  #[instrument("RenameAction", skip(user), fields(user_id = user.id))]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<Update> {
    Ok(resource::rename::<Action>(&self.id, &self.name, user).await?)
  }
}

impl Resolve<WriteArgs> for DeleteAction {
  #[instrument(
    "DeleteAction",
    skip(self, user),
    fields(user_id = user.id, action_id = self.id)
  )]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> serror::Result<Action> {
    Ok(resource::delete::<Action>(&self.id, user).await?)
  }
}
