use anyhow::Context;
use database::mungos::mongodb::{Collection, bson::doc};
use indexmap::IndexSet;
use komodo_client::entities::{
  Operation, ResourceTarget, ResourceTargetVariant, komodo_timestamp,
  optional_string,
  permission::SpecificPermission,
  resource::Resource,
  server::{
    PartialServerConfig, Server, ServerConfig, ServerConfigDiff,
    ServerListItem, ServerListItemInfo, ServerQuerySpecifics,
  },
  update::Update,
  user::User,
};

use crate::{
  config::core_config,
  connection::PeripheryConnectionArgs,
  monitor::update_cache_for_server,
  periphery::PeripheryClient,
  state::{
    action_states, db_client, periphery_connections,
    server_status_cache,
  },
};

impl super::KomodoResource for Server {
  type Config = ServerConfig;
  type PartialConfig = PartialServerConfig;
  type ConfigDiff = ServerConfigDiff;
  type Info = ();
  type ListItem = ServerListItem;
  type QuerySpecifics = ServerQuerySpecifics;

  fn resource_type() -> ResourceTargetVariant {
    ResourceTargetVariant::Server
  }

  fn resource_target(id: impl Into<String>) -> ResourceTarget {
    ResourceTarget::Server(id.into())
  }

  fn creator_specific_permissions() -> IndexSet<SpecificPermission> {
    [
      SpecificPermission::Terminal,
      SpecificPermission::Inspect,
      SpecificPermission::Attach,
      SpecificPermission::Logs,
      SpecificPermission::Processes,
    ]
    .into_iter()
    .collect()
  }

  fn coll() -> &'static Collection<Resource<Self::Config, Self::Info>>
  {
    &db_client().servers
  }

  async fn to_list_item(
    server: Resource<Self::Config, Self::Info>,
  ) -> Self::ListItem {
    let status = server_status_cache().get(&server.id).await;
    let (
      version,
      public_key,
      terminals_disabled,
      container_exec_disabled,
    ) = match status.as_ref().and_then(|s| s.periphery_info.as_ref())
    {
      Some(info) => (
        Some(info.version.clone()),
        Some(info.public_key.clone()),
        info.terminals_disabled,
        info.container_exec_disabled,
      ),
      None => (None, None, true, true),
    };
    ServerListItem {
      name: server.name,
      id: server.id,
      template: server.template,
      tags: server.tags,
      resource_type: ResourceTargetVariant::Server,
      info: ServerListItemInfo {
        state: status.as_ref().map(|s| s.state).unwrap_or_default(),
        region: server.config.region,
        address: optional_string(server.config.address),
        external_address: optional_string(
          server.config.external_address,
        ),
        send_unreachable_alerts: server
          .config
          .send_unreachable_alerts,
        send_cpu_alerts: server.config.send_cpu_alerts,
        send_mem_alerts: server.config.send_mem_alerts,
        send_disk_alerts: server.config.send_disk_alerts,
        send_version_mismatch_alerts: server
          .config
          .send_version_mismatch_alerts,
        version,
        public_key,
        terminals_disabled,
        container_exec_disabled,
      },
    }
  }

  async fn busy(id: &String) -> anyhow::Result<bool> {
    action_states()
      .server
      .get(id)
      .await
      .unwrap_or_default()
      .busy()
  }

  // CREATE

  fn create_operation() -> Operation {
    Operation::CreateServer
  }

  fn user_can_create(user: &User) -> bool {
    user.admin
      || (!core_config().disable_non_admin_create
        && user.create_server_permissions)
  }

  async fn validate_create_config(
    _config: &mut Self::PartialConfig,
    _user: &User,
  ) -> anyhow::Result<()> {
    Ok(())
  }

  async fn post_create(
    created: &Resource<Self::Config, Self::Info>,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    update_cache_for_server(created, true).await;
    Ok(())
  }

  // UPDATE

  fn update_operation() -> Operation {
    Operation::UpdateServer
  }

  async fn validate_update_config(
    _id: &str,
    _config: &mut Self::PartialConfig,
    _user: &User,
  ) -> anyhow::Result<()> {
    Ok(())
  }

  async fn post_update(
    updated: &Self,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    if updated.config.enabled {
      // Init periphery client to trigger reconnection
      // if relevant parameters change.
      let _ = PeripheryClient::new(
        updated.id.clone(),
        PeripheryConnectionArgs {
          address: &updated.config.address,
          core_private_key: &updated.config.core_private_key,
          periphery_public_key: &updated.config.periphery_public_key,
        },
        &updated.config.passkey,
      )
      .await;
    } else {
      periphery_connections().remove(&updated.id).await;
    }
    update_cache_for_server(updated, true).await;
    Ok(())
  }

  // RENAME

  fn rename_operation() -> Operation {
    Operation::RenameServer
  }

  // DELETE

  fn delete_operation() -> Operation {
    Operation::DeleteServer
  }

  async fn pre_delete(
    resource: &Resource<Self::Config, Self::Info>,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    let db = db_client();

    let id = &resource.id;

    db.builders
      .update_many(
        doc! { "config.params.server_id": &id },
        doc! { "$set": { "config.params.server_id": "" } },
      )
      .await
      .context("failed to detach server from builders")?;

    db.deployments
      .update_many(
        doc! { "config.server_id": &id },
        doc! { "$set": { "config.server_id": "" } },
      )
      .await
      .context("failed to detach server from deployments")?;

    db.stacks
      .update_many(
        doc! { "config.server_id": &id },
        doc! { "$set": { "config.server_id": "" } },
      )
      .await
      .context("failed to detach server from stacks")?;

    db.repos
      .update_many(
        doc! { "config.server_id": &id },
        doc! { "$set": { "config.server_id": "" } },
      )
      .await
      .context("failed to detach server from repos")?;

    db.alerts
      .update_many(
        doc! { "target.type": "Server", "target.id": &id },
        doc! { "$set": {
          "resolved": true,
          "resolved_ts": komodo_timestamp()
        } },
      )
      .await
      .context("failed to close deleted server alerts")?;

    Ok(())
  }

  async fn post_delete(
    resource: &Resource<Self::Config, Self::Info>,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    tokio::join!(
      server_status_cache().remove(&resource.id),
      periphery_connections().remove(&resource.id),
    );
    Ok(())
  }
}
