use anyhow::anyhow;
use axum::{
  extract::{Query, WebSocketUpgrade},
  http::{HeaderMap, StatusCode},
  response::Response,
};
use komodo_client::entities::server::Server;
use serror::{AddStatusCode, AddStatusCodeError};
use transport::{
  PeripheryConnectionQuery,
  auth::{HeaderConnectionIdentifiers, ServerLoginFlow},
  websocket::axum::AxumWebsocket,
};

use crate::{
  connection::PeripheryConnectionArgs, state::periphery_connections,
};

pub async fn handler(
  Query(PeripheryConnectionQuery { server: _server }): Query<
    PeripheryConnectionQuery,
  >,
  mut headers: HeaderMap,
  ws: WebSocketUpgrade,
) -> serror::Result<Response> {
  let identifiers =
    HeaderConnectionIdentifiers::extract(&mut headers)
      .status_code(StatusCode::UNAUTHORIZED)?;

  let server = crate::resource::get::<Server>(&_server)
    .await
    .status_code(StatusCode::BAD_REQUEST)?;

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
      anyhow!("A Server '{_server}' is already connected")
        .status_code(StatusCode::UNAUTHORIZED),
    );
  }

  let (connection, mut receiver) = periphery_connections()
    .insert(
      server.id.clone(),
      PeripheryConnectionArgs {
        address: "",
        core_private_key: &server.config.core_private_key,
        periphery_public_key: &server.config.periphery_public_key,
      },
    )
    .await;

  Ok(ws.on_upgrade(|socket| async move {
    let query = format!("server={}", urlencoding::encode(&_server));
    let mut socket = AxumWebsocket(socket);

    if let Err(e) = connection
      .handle_login::<_, ServerLoginFlow>(
        &mut socket,
        identifiers.build(query.as_bytes()),
      )
      .await
    {
      connection.set_error(e).await;
    }

    connection.handle_socket(socket, &mut receiver).await
  }))
}
