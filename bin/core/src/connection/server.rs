use std::{str::FromStr, time::Duration};

use anyhow::{Context, anyhow};
use axum::{
  extract::{Query, WebSocketUpgrade},
  http::{HeaderMap, StatusCode},
  response::Response,
};
use bytes::Bytes;
use database::mungos::mongodb::bson::{doc, oid::ObjectId};
use komodo_client::{
  api::write::{CreateServer, UpdateResourceMeta},
  entities::{
    onboarding_key::OnboardingKey,
    server::{PartialServerConfig, Server},
    user::system_user,
  },
};
use resolver_api::Resolve;
use serror::{AddStatusCode, AddStatusCodeError};
use transport::{
  MessageState, PeripheryConnectionQuery,
  auth::{
    HeaderConnectionIdentifiers, LoginFlow, LoginFlowArgs,
    PublicKeyValidator, ServerLoginFlow,
  },
  websocket::{Websocket, axum::AxumWebsocket},
};

use crate::{
  api::write::WriteArgs,
  config::core_private_key,
  helpers::query::id_or_name_filter,
  resource::KomodoResource,
  state::{db_client, periphery_connections},
};

use super::PeripheryConnectionArgs;

pub async fn handler(
  Query(PeripheryConnectionQuery {
    server: server_query,
  }): Query<PeripheryConnectionQuery>,
  mut headers: HeaderMap,
  ws: WebSocketUpgrade,
) -> serror::Result<Response> {
  let identifiers =
    HeaderConnectionIdentifiers::extract(&mut headers)
      .status_code(StatusCode::UNAUTHORIZED)?;

  if server_query.is_empty() {
    return Err(
      anyhow!("Must provide non-empty server specifier")
        .status_code(StatusCode::UNAUTHORIZED),
    );
  }

  // Handle connection vs. onboarding flow.
  match Server::coll()
    .find_one(id_or_name_filter(&server_query))
    .await
    .context("Failed to query database for Server")?
  {
    Some(server) => {
      existing_server_handler(server_query, server, identifiers, ws)
        .await
    }
    None if ObjectId::from_str(&server_query).is_err() => {
      onboard_server_handler(server_query, identifiers, ws).await
    }
    None => Err(
      anyhow!("Must provide name based Server specifier for onboarding flow, name cannot be valid ObjectId (hex)")
        .status_code(StatusCode::UNAUTHORIZED),
    ),
  }
}

async fn existing_server_handler(
  server_query: String,
  server: Server,
  identifiers: HeaderConnectionIdentifiers,
  ws: WebSocketUpgrade,
) -> serror::Result<Response> {
  if !server.config.enabled {
    return Err(anyhow!("Server is Disabled."))
      .status_code(StatusCode::BAD_REQUEST);
  }

  if !server.config.address.is_empty() {
    return Err(anyhow!(
      "Server is configured to use a Core -> Periphery connection."
    ))
    .status_code(StatusCode::BAD_REQUEST);
  }

  let connections = periphery_connections();

  // Ensure connected server can't get bumped off the connection.
  // Treat this as authorization issue.
  if let Some(existing_connection) = connections.get(&server.id).await
    && existing_connection.connected()
  {
    return Err(
      anyhow!("A Server '{server_query}' is already connected")
        .status_code(StatusCode::UNAUTHORIZED),
    );
  }

  let (connection, mut receiver) = periphery_connections()
    .insert(
      server.id.clone(),
      PeripheryConnectionArgs::from_server(&server),
    )
    .await;

  Ok(ws.on_upgrade(|socket| async move {
    let query =
      format!("server={}", urlencoding::encode(&server_query));
    let mut socket = AxumWebsocket(socket);

    if let Err(e) = socket.send(Bytes::from_owner([0])).await.context(
      "Failed to send the login flow indicator over connnection",
    ) {
      connection.set_error(e).await;
      return;
    };

    if let Err(e) = connection
      .handle_login::<_, ServerLoginFlow>(
        &mut socket,
        identifiers.build(query.as_bytes()),
      )
      .await
    {
      connection.set_error(e).await;
      return;
    }

    connection.handle_socket(socket, &mut receiver).await
  }))
}

async fn onboard_server_handler(
  server_query: String,
  identifiers: HeaderConnectionIdentifiers,
  ws: WebSocketUpgrade,
) -> serror::Result<Response> {
  Ok(ws.on_upgrade(|socket| async move {
    let query =
      format!("server={}", urlencoding::encode(&server_query));
    let mut socket = AxumWebsocket(socket);

    if let Err(e) = socket.send(Bytes::from_owner([1])).await.context(
      "Failed to send the login flow indicator over connnection",
    ).context("Server onboarding error") {
      warn!("{e:#}");
      return;
    };

    let onboarding_key = match ServerLoginFlow::login(LoginFlowArgs {
      socket: &mut socket,
      identifiers: identifiers.build(query.as_bytes()),
      private_key: core_private_key(),
      public_key_validator: CreationKeyValidator,
    })
    .await
    {
      Ok(onboarding_key) => onboarding_key,
      Err(e) => {
        debug!("Server {server_query} failed to onboard | {e:#}");
        return;
      }
    };

    let res = socket
      .recv_bytes()
      .with_timeout(Duration::from_secs(2))
      .await
      .and_then(|res| {
        res.and_then(|public_key_bytes| {
          String::from_utf8(public_key_bytes.into())
            .context("Public key bytes are not valid utf8")
        })
      });

    // Post onboarding login 1: Receive public key
    let public_key = match res
    {
      Ok(public_key) => public_key,
      Err(e) => {
        warn!("Server {server_query} failed to onboard | failed to receive Server public key | {e:#}");
        return;
      }
    };

    let config = if onboarding_key.copy_server.is_empty() {
      PartialServerConfig {
        enabled: Some(true),
        ..Default::default()
      }
    } else {
      let config = match db_client().servers.find_one(id_or_name_filter(&onboarding_key.copy_server)).await {
        Ok(Some(server)) => server.config,
        Ok(None) => {
          warn!("Server onboarding: Failed to find Server {}", onboarding_key.copy_server);
          Default::default()
        }
        Err(e) => {
          warn!("Failed to query database for onboarding key 'copy_server' | {e:?}");
          Default::default()
        }
      };
      PartialServerConfig {
        enabled: Some(true),
        address: None,
        ..config.into()
      }
    };

    let args = WriteArgs { user: system_user().to_owned() };

    let create = CreateServer { name: server_query, config, public_key: Some(public_key) };
    let server = match create.resolve(&args).await {
      Ok(server) => server,
      Err(e) => {
        warn!("Server onboarding flow failed at Server creation | {:#}", e.error);
        if let Err(e) = socket
          .send_error(&e.error)
          .await
          .context("Failed to send Server creation failed to client")
        {
          // Log additional error
          warn!("{e:#}");
        }
        return;
      }
    };

    let meta = UpdateResourceMeta { target: (&server).into(), tags: Some(onboarding_key.tags), description: None, template: None };
    match meta.resolve(&args).await {
      Ok(server) => server,
      Err(e) => {
        warn!("Server onboarding flow failed at Server meta update | {:#}", e.error);
        if let Err(e) = socket
          .send_error(&e.error)
          .await
          .context("Failed to send Server meta update failed to client")
        {
          // Log additional error
          warn!("{e:#}");
        }
        return;
      }
    };

    if let Err(e) = socket
      .send(MessageState::Successful.into())
      .await
      .context("Failed to send Server creation successful to client")
    {
      // Log additional error
      warn!("{e:#}");
    }

    // Server created, close and trigger reconnect
    // and handling using existing server handler.
    let _ = socket.close(None).await;

    // Add the server to onboarding key "Onboarded"
    let res = db_client()
      .onboarding_keys
      .update_one(
        doc! { "public_key": &onboarding_key.public_key },
        doc! { "$push": { "onboarded": server.id } },
      ).await;
    if let Err(e) = res {
      warn!("Failed to update onboarding key 'onboarded' | {e:?}");
    }
  }))
}

struct CreationKeyValidator;

impl PublicKeyValidator for CreationKeyValidator {
  type ValidationResult = OnboardingKey;
  async fn validate(
    &self,
    public_key: String,
  ) -> anyhow::Result<Self::ValidationResult> {
    let onboarding_key = db_client()
      .onboarding_keys
      .find_one(doc! { "public_key": &public_key })
      .await
      .context("Failed to query database for Server onboarding keys")?
      .context("Matching Server onboarding key not found")?;
    if onboarding_key.enabled {
      Ok(onboarding_key)
    } else {
      Err(anyhow!("Onboarding key is disabled"))
    }
  }
}
