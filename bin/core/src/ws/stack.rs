use axum::{
  extract::{Query, WebSocketUpgrade, ws::Message},
  response::IntoResponse,
};
use futures::SinkExt;
use komodo_client::{
  api::terminal::{ConnectStackAttachQuery, ConnectStackExecQuery},
  entities::{
    permission::PermissionLevel, server::Server, stack::Stack,
  },
};

use crate::{
  permission::get_check_permissions, resource::get,
  state::stack_status_cache,
};

#[instrument(name = "ConnectStackExec", skip(ws))]
pub async fn exec(
  Query(ConnectStackExecQuery {
    stack,
    service,
    shell,
  }): Query<ConnectStackExecQuery>,
  ws: WebSocketUpgrade,
) -> impl IntoResponse {
  ws.on_upgrade(|socket| async move {
    let Some((client_socket, server, container)) =
      login_get_server_container(socket, &stack, &service).await
    else {
      return;
    };

    super::handle_container_exec_terminal(
      client_socket,
      &server,
      container,
      shell,
    )
    .await
  })
}

#[instrument(name = "ConnectStackAttach", skip(ws))]
pub async fn attach(
  Query(ConnectStackAttachQuery { stack, service }): Query<
    ConnectStackAttachQuery,
  >,
  ws: WebSocketUpgrade,
) -> impl IntoResponse {
  ws.on_upgrade(|socket| async move {
    let Some((client_socket, server, container)) =
      login_get_server_container(socket, &stack, &service).await
    else {
      return;
    };

    super::handle_container_attach_terminal(
      client_socket,
      &server,
      container,
    )
    .await
  })
}

async fn login_get_server_container(
  socket: axum::extract::ws::WebSocket,
  stack: &str,
  service: &str,
) -> Option<(axum::extract::ws::WebSocket, Server, String)> {
  let Some((mut client_socket, user)) =
    super::user_ws_login(socket).await
  else {
    return None;
  };

  let stack = match get_check_permissions::<Stack>(
    stack,
    &user,
    PermissionLevel::Read.terminal(),
  )
  .await
  {
    Ok(stack) => stack,
    Err(e) => {
      debug!("could not get stack | {e:#}");
      let _ = client_socket
        .send(Message::text(format!("ERROR: {e:#}")))
        .await;
      let _ = client_socket.close().await;
      return None;
    }
  };

  let server = match get::<Server>(&stack.config.server_id).await {
    Ok(server) => server,
    Err(e) => {
      debug!("could not get server | {e:#}");
      let _ = client_socket
        .send(Message::text(format!("ERROR: {e:#}")))
        .await;
      let _ = client_socket.close().await;
      return None;
    }
  };

  let Some(status) = stack_status_cache().get(&stack.id).await else {
    debug!("could not get stack status");
    let _ = client_socket
      .send(Message::text(String::from(
        "ERROR: could not get stack status",
      )))
      .await;
    let _ = client_socket.close().await;
    return None;
  };

  let container = match status
    .curr
    .services
    .iter()
    .find(|s| s.service == service)
    .map(|s| s.container.as_ref())
  {
    Some(Some(container)) => container.name.clone(),
    Some(None) => {
      let _ = client_socket
        .send(Message::text(format!(
          "ERROR: Service {service} container could not be found"
        )))
        .await;
      let _ = client_socket.close().await;
      return None;
    }
    None => {
      let _ = client_socket
        .send(Message::text(format!(
          "ERROR: Service {service} could not be found"
        )))
        .await;
      let _ = client_socket.close().await;
      return None;
    }
  };

  Some((client_socket, server, container))
}
