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
  auth::{ServerHeaderIdentifiers, ServerLoginFlow},
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
  let identifiers = ServerHeaderIdentifiers::extract(&mut headers)
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

  let (connection, mut write_receiver) = periphery_connections()
    .insert(
      server.id.clone(),
      PeripheryConnectionArgs {
        address: "",
        private_key: &server.config.private_key,
        expected_public_key: &server.config.public_key,
      },
    )
    .await;

  Ok(ws.on_upgrade(|socket| async move {
    let query = format!("server={}", urlencoding::encode(&_server));
    let handler = super::WebsocketHandler {
      socket: AxumWebsocket(socket),
      connection_identifiers: identifiers.build(query.as_bytes()),
      write_receiver: &mut write_receiver,
      connection: &connection,
    };

    if let Err(e) = handler.handle::<ServerLoginFlow>().await {
      connection.set_error(e).await;
    }
  }))
}
