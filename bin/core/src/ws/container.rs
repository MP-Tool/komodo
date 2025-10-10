use axum::{
  extract::{Query, WebSocketUpgrade, ws::Message},
  response::IntoResponse,
};
use futures::SinkExt;
use komodo_client::{
  api::terminal::{
    ConnectContainerAttachQuery, ConnectContainerExecQuery,
  },
  entities::{permission::PermissionLevel, server::Server},
};

use crate::permission::get_check_permissions;

#[instrument(name = "ConnectContainerExec", skip(ws))]
pub async fn exec(
  Query(ConnectContainerExecQuery {
    server,
    container,
    shell,
    recreate,
  }): Query<ConnectContainerExecQuery>,
  ws: WebSocketUpgrade,
) -> impl IntoResponse {
  ws.on_upgrade(async move |socket| {
    let Some((mut client_socket, user)) =
      super::user_ws_login(socket).await
    else {
      return;
    };

    let server = match get_check_permissions::<Server>(
      &server,
      &user,
      PermissionLevel::Read.terminal(),
    )
    .await
    {
      Ok(server) => server,
      Err(e) => {
        debug!("could not get server | {e:#}");
        let _ = client_socket
          .send(Message::text(format!("ERROR: {e:#}")))
          .await;
        let _ = client_socket.close().await;
        return;
      }
    };

    super::handle_container_exec_terminal(
      client_socket,
      &server,
      container,
      shell,
      recreate,
    )
    .await
  })
}

#[instrument(name = "ConnectContainerAttach", skip(ws))]
pub async fn attach(
  Query(ConnectContainerAttachQuery {
    server,
    container,
    recreate,
  }): Query<ConnectContainerAttachQuery>,
  ws: WebSocketUpgrade,
) -> impl IntoResponse {
  ws.on_upgrade(async move |socket| {
    let Some((mut client_socket, user)) =
      super::user_ws_login(socket).await
    else {
      return;
    };

    let server = match get_check_permissions::<Server>(
      &server,
      &user,
      PermissionLevel::Read.terminal(),
    )
    .await
    {
      Ok(server) => server,
      Err(e) => {
        debug!("could not get server | {e:#}");
        let _ = client_socket
          .send(Message::text(format!("ERROR: {e:#}")))
          .await;
        let _ = client_socket.close().await;
        return;
      }
    };

    super::handle_container_attach_terminal(
      client_socket,
      &server,
      container,
      recreate,
    )
    .await
  })
}
