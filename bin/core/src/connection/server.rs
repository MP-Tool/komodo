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
  api::write::CreateServer,
  entities::{
    server::{Server, ServerConfig},
    server_onboarding_key::ServerOnboardingKey,
    user::system_user,
  },
};
use resolver_api::Resolve;
use serror::{
  AddStatusCode, AddStatusCodeError, serialize_error_bytes,
};
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

    let creation_key = match ServerLoginFlow::login(LoginFlowArgs {
      socket: &mut socket,
      identifiers: identifiers.build(query.as_bytes()),
      private_key: core_private_key(),
      public_key_validator: CreationKeyValidator,
    })
    .await
    {
      Ok(creation_key) => creation_key,
      Err(e) => {
        debug!("Server {server_query} failed to onboard via creation key | {e:#}");
        return;
      }
    };

    // Post onboarding login 1: Receive public key
    let periphery_public_key = match socket
      .recv_bytes_with_timeout(Duration::from_secs(2))
      .await
      .and_then(|public_key_bytes| String::from_utf8(public_key_bytes.into())
      .context("Public key bytes are not valid utf8"))
    {
      Ok(public_key_bytes) => public_key_bytes,
      Err(e) => {
        warn!("Server {server_query} failed to onboard | failed to receive Server public key | {e:#}");
        return;
      }
    };
    
    let config = ServerConfig {
      periphery_public_key,
      enabled: true,
      ..creation_key.default_config
    };

    let request = CreateServer { name: server_query, config: config.into() };
    if let Err(e) = request.resolve(&WriteArgs { user: system_user().to_owned() }).await {
      warn!("Server onboarding flow failed at Server creation | {:#}", e.error);
      let mut bytes = serialize_error_bytes(&e.error);
      bytes.push(MessageState::Failed.as_byte());
      if let Err(e) = socket
        .send(bytes.into())
        .await
        .context("Failed to send Server creation failed to client")
      {
        // Log additional error
        warn!("{e:#}");
      }
    } else {
      if let Err(e) = socket
        .send(MessageState::Successful.into())
        .await
        .context("Failed to send Server creation successful to client")
      {
        // Log additional error
        warn!("{e:#}");
      }
    };

    // Server created, close and trigger reconnect
    // and handling using existing server handler.
    let _ = socket.close(None).await;
  }))
}

struct CreationKeyValidator;

impl PublicKeyValidator for CreationKeyValidator {
  type ValidationResult = ServerOnboardingKey;
  async fn validate(
    &self,
    public_key: String,
  ) -> anyhow::Result<Self::ValidationResult> {
    db_client()
      .server_onboarding_keys
      .find_one(doc! { "public_key": &public_key })
      .await
      .context("Failed to query database for Server onboarding keys")?
      .context("Matching Server onboarding key not found")
  }
}
