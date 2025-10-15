use std::str::FromStr;

use anyhow::{Context, anyhow};
use axum::{
  extract::{Query, WebSocketUpgrade},
  http::{HeaderMap, StatusCode},
  response::Response,
};
use database::mungos::mongodb::bson::{doc, oid::ObjectId};
use komodo_client::{
  api::write::{CreateBuilder, CreateServer, UpdateResourceMeta},
  entities::{
    builder::{PartialBuilderConfig, PartialServerBuilderConfig},
    komodo_timestamp,
    onboarding_key::OnboardingKey,
    server::{PartialServerConfig, Server},
    user::system_user,
  },
};
use periphery_client::{
  api::PeripheryConnectionQuery, transport::LoginMessage,
};
use resolver_api::Resolve;
use serror::{AddStatusCode, AddStatusCodeError};
use tracing::Instrument;
use transport::{
  auth::{
    HeaderConnectionIdentifiers, LoginFlow, LoginFlowArgs,
    PublicKeyValidator, ServerLoginFlow,
  },
  websocket::{
    Websocket, WebsocketExt as _, axum::AxumWebsocket,
    login::LoginWebsocketExt,
  },
};

use crate::{
  api::write::WriteArgs,
  config::core_keys,
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

    if let Err(e) = socket
      .send_message(LoginMessage::OnboardingFlow(false))
      .await
      .context("Failed to send Login OnboardingFlow false message")
    {
      connection.set_error(e).await;
      return;
    };

    let span = info_span!(
      "PeripheryLogin",
      server_id = server.id,
      direction = "PeripheryToCore"
    );
    let login = async {
      connection
        .handle_login::<_, ServerLoginFlow>(
          &mut socket,
          identifiers.build(query.as_bytes()),
        )
        .await
    }
    .instrument(span)
    .await;

    if let Err(e) = login {
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

    if let Err(e) = socket.send_message(LoginMessage::OnboardingFlow(true)).await.context(
      "Failed to send Login OnboardingFlow true message",
    ).context("Server onboarding error") {
      warn!("{e:#}");
      return;
    };

    let onboarding_key = match ServerLoginFlow::login(LoginFlowArgs {
      socket: &mut socket,
      identifiers: identifiers.build(query.as_bytes()),
      private_key: core_keys().load().private.as_str(),
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

    // Post onboarding login 1: Receive public key
    let public_key = match socket
      .recv_login_public_key()
      .await
    {
      Ok(public_key) => public_key,
      Err(e) => {
        warn!("Server {server_query} failed to onboard | failed to receive Server public key | {e:#}");
        return;
      }
    };

    let server_id = match create_server_maybe_builder(
      server_query,
      public_key.into_inner(),
      onboarding_key.copy_server,
      onboarding_key.tags,
      onboarding_key.create_builder
    ).await {
      Ok(server_id) => server_id,
      Err(e) => {
        warn!("{e:#}");
        if let Err(e) = socket
          .send_login_error(&e)
          .await
          .context("Failed to send Server creation failed to client")
        {
          // Log additional error
          warn!("{e:#}");
        }
        return;
      }
    };

    if let Err(e) = socket
      .send_message(LoginMessage::Success)
      .await
      .context("Failed to send Login Onboarding Successful message")
    {
      // Log additional error
      warn!("{e:#}");
    }

    // Server created, close and trigger reconnect
    // and handling using existing server handler.
    let _ = socket.close().await;

    // Add the server to onboarding key "Onboarded"
    let res = db_client()
      .onboarding_keys
      .update_one(
        doc! { "public_key": &onboarding_key.public_key },
        doc! { "$push": { "onboarded": server_id } },
      ).await;
    if let Err(e) = res {
      warn!("Failed to update onboarding key 'onboarded' | {e:?}");
    }
  }))
}

async fn create_server_maybe_builder(
  server_query: String,
  public_key: String,
  copy_server: String,
  tags: Vec<String>,
  create_builder: bool,
) -> anyhow::Result<String> {
  let config = if copy_server.is_empty() {
    PartialServerConfig {
      enabled: Some(true),
      ..Default::default()
    }
  } else {
    let config = match db_client()
      .servers
      .find_one(id_or_name_filter(&copy_server))
      .await
    {
      Ok(Some(server)) => server.config,
      Ok(None) => {
        warn!(
          "Server onboarding: Failed to find Server {}",
          copy_server
        );
        Default::default()
      }
      Err(e) => {
        warn!(
          "Failed to query database for onboarding key 'copy_server' | {e:?}"
        );
        Default::default()
      }
    };
    PartialServerConfig {
      enabled: Some(true),
      address: None,
      ..config.into()
    }
  };

  let args = WriteArgs {
    user: system_user().to_owned(),
  };

  let server = CreateServer {
    name: server_query.clone(),
    config,
    public_key: Some(public_key),
  }
  .resolve(&args)
  .await
  .map_err(|e| e.error)
  .context("Server onboarding flow failed at Server creation")?;

  // Don't need to fail, only warn on this
  if let Err(e) = (UpdateResourceMeta {
    target: (&server).into(),
    tags: Some(tags),
    description: None,
    template: None,
  })
  .resolve(&args)
  .await
  .map_err(|e| e.error)
  .context("Server onboarding flow failed at Server creation")
  {
    warn!("{e:#}");
  };

  if create_builder {
    // Don't need to fail, only warn on this
    if let Err(e) = (CreateBuilder {
      name: server_query,
      config: PartialBuilderConfig::Server(
        PartialServerBuilderConfig {
          server_id: Some(server.id.clone()),
        },
      ),
    })
    .resolve(&args)
    .await
    .map_err(|e| e.error)
    .context("Server onboarding flow failed at Builder creation")
    {
      warn!("{e:#}");
    };
  }

  Ok(server.id)
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
    // Check enabled and not expired.
    if onboarding_key.enabled
      && (onboarding_key.expires == 0
        || onboarding_key.expires > komodo_timestamp())
    {
      Ok(onboarding_key)
    } else {
      Err(anyhow!("Onboarding key is invalid"))
    }
  }
}
