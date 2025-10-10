use crate::{
  auth::{auth_api_key_check_enabled, auth_jwt_check_enabled},
  helpers::query::get_user,
  periphery::PeripheryClient,
  state::periphery_connections,
};
use anyhow::anyhow;
use axum::{
  Router,
  extract::ws::{self, WebSocket},
  routing::get,
};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use komodo_client::{
  api::write::TerminalRecreateMode,
  entities::{server::Server, user::User},
  ws::WsLoginMessage,
};
use periphery_client::{
  api::terminal::DisconnectTerminal,
  transport::EncodedTransportMessage,
};
use tokio_util::sync::CancellationToken;
use transport::channel::{Receiver, Sender};
use uuid::Uuid;

mod container;
mod deployment;
mod stack;
mod terminal;
mod update;

pub fn router() -> Router {
  Router::new()
    // Periphery facing
    .route("/periphery", get(crate::connection::server::handler))
    // User facing
    .route("/update", get(update::handler))
    .route("/terminal", get(terminal::handler))
    .route("/container/terminal", get(container::exec))
    .route("/container/terminal/attach", get(container::attach))
    .route("/deployment/terminal", get(deployment::exec))
    .route("/deployment/terminal/attach", get(deployment::attach))
    .route("/stack/terminal", get(stack::exec))
    .route("/stack/terminal/attach", get(stack::attach))
}

#[instrument(level = "debug")]
async fn user_ws_login(
  mut socket: WebSocket,
) -> Option<(WebSocket, User)> {
  let login_msg = match socket.recv().await {
    Some(Ok(ws::Message::Text(login_msg))) => {
      LoginMessage::Ok(login_msg.to_string())
    }
    Some(Ok(msg)) => {
      LoginMessage::Err(format!("invalid login message: {msg:?}"))
    }
    Some(Err(e)) => {
      LoginMessage::Err(format!("failed to get login message: {e:?}"))
    }
    None => {
      LoginMessage::Err("failed to get login message".to_string())
    }
  };
  let login_msg = match login_msg {
    LoginMessage::Ok(login_msg) => login_msg,
    LoginMessage::Err(msg) => {
      let _ = socket.send(ws::Message::text(msg)).await;
      let _ = socket.close().await;
      return None;
    }
  };
  match WsLoginMessage::from_json_str(&login_msg) {
    // Login using a jwt
    Ok(WsLoginMessage::Jwt { jwt }) => {
      match auth_jwt_check_enabled(&jwt).await {
        Ok(user) => {
          let _ = socket.send(ws::Message::text("LOGGED_IN")).await;
          Some((socket, user))
        }
        Err(e) => {
          let _ = socket
            .send(ws::Message::text(format!(
              "failed to authenticate user using jwt | {e:#}"
            )))
            .await;
          let _ = socket.close().await;
          None
        }
      }
    }
    // login using api keys
    Ok(WsLoginMessage::ApiKeys { key, secret }) => {
      match auth_api_key_check_enabled(&key, &secret).await {
        Ok(user) => {
          let _ = socket.send(ws::Message::text("LOGGED_IN")).await;
          Some((socket, user))
        }
        Err(e) => {
          let _ = socket
            .send(ws::Message::text(format!(
              "failed to authenticate user using api keys | {e:#}"
            )))
            .await;
          let _ = socket.close().await;
          None
        }
      }
    }
    Err(e) => {
      let _ = socket
        .send(ws::Message::text(format!(
          "failed to parse login message: {e:#}"
        )))
        .await;
      let _ = socket.close().await;
      None
    }
  }
}

enum LoginMessage {
  /// The text message
  Ok(String),
  /// The err message
  Err(String),
}

#[instrument(level = "debug")]
async fn check_user_valid(user_id: &str) -> anyhow::Result<User> {
  let user = get_user(user_id).await?;
  if !user.enabled {
    return Err(anyhow!("user not enabled"));
  }
  Ok(user)
}

async fn handle_container_exec_terminal(
  mut client_socket: WebSocket,
  server: &Server,
  container: String,
  shell: String,
  recreate: TerminalRecreateMode,
) {
  let periphery = match crate::helpers::periphery_client(server).await
  {
    Ok(periphery) => periphery,
    Err(e) => {
      debug!("couldn't get periphery | {e:#}");
      let _ = client_socket
        .send(ws::Message::text(format!("ERROR: {e:#}")))
        .await;
      let _ = client_socket.close().await;
      return;
    }
  };

  trace!("connecting to periphery container exec websocket");

  let (periphery_connection_id, periphery_sender, periphery_receiver) =
    match periphery
      .connect_container_exec(container, shell, recreate)
      .await
    {
      Ok(ws) => ws,
      Err(e) => {
        debug!(
          "Failed connect to periphery container exec websocket | {e:#}"
        );
        let _ = client_socket
          .send(ws::Message::text(format!("ERROR: {e:#}")))
          .await;
        let _ = client_socket.close().await;
        return;
      }
    };

  trace!("connected to periphery container exec websocket");

  forward_ws_channel(
    periphery,
    client_socket,
    periphery_connection_id,
    periphery_sender,
    periphery_receiver,
  )
  .await
}

async fn handle_container_attach_terminal(
  mut client_socket: WebSocket,
  server: &Server,
  container: String,
  recreate: TerminalRecreateMode,
) {
  let periphery = match crate::helpers::periphery_client(server).await
  {
    Ok(periphery) => periphery,
    Err(e) => {
      debug!("couldn't get periphery | {e:#}");
      let _ = client_socket
        .send(ws::Message::text(format!("ERROR: {e:#}")))
        .await;
      let _ = client_socket.close().await;
      return;
    }
  };

  trace!("connecting to periphery container exec websocket");

  let (periphery_connection_id, periphery_sender, periphery_receiver) =
    match periphery
      .connect_container_attach(container, recreate)
      .await
    {
      Ok(ws) => ws,
      Err(e) => {
        debug!(
          "Failed connect to periphery container attach websocket | {e:#}"
        );
        let _ = client_socket
          .send(ws::Message::text(format!("ERROR: {e:#}")))
          .await;
        let _ = client_socket.close().await;
        return;
      }
    };

  trace!("connected to periphery container attach websocket");

  forward_ws_channel(
    periphery,
    client_socket,
    periphery_connection_id,
    periphery_sender,
    periphery_receiver,
  )
  .await
}

async fn forward_ws_channel(
  periphery: PeripheryClient,
  client_socket: axum::extract::ws::WebSocket,
  periphery_connection_id: Uuid,
  periphery_sender: Sender<EncodedTransportMessage>,
  mut periphery_receiver: Receiver<Vec<u8>>,
) {
  let (mut client_send, mut client_receive) = client_socket.split();
  let cancel = CancellationToken::new();

  periphery_receiver.set_cancel(cancel.clone());

  trace!("starting ws exchange");

  let core_to_periphery = async {
    loop {
      let client_recv_res = tokio::select! {
        res = client_receive.next() => res,
        _ = cancel.cancelled() => {
          trace!("core to periphery read: cancelled from inside");
          break;
        }
      };
      match client_recv_res {
        Some(Ok(ws::Message::Binary(bytes))) => {
          if let Err(e) = periphery_sender
            .send_terminal(periphery_connection_id, bytes)
            .await
          {
            debug!("Failed to send terminal message | {e:?}",);
            cancel.cancel();
            break;
          };
        }
        Some(Ok(ws::Message::Text(text))) => {
          let bytes: Bytes = text.into();
          if let Err(e) = periphery_sender
            .send_terminal(periphery_connection_id, bytes)
            .await
          {
            debug!("Failed to send terminal message | {e:?}",);
            cancel.cancel();
            break;
          };
        }
        // TODO: Disconnect from periphery when client disconnects
        Some(Ok(ws::Message::Close(_frame))) => {
          cancel.cancel();
          break;
        }
        // Ignore
        Some(Ok(_)) => {}
        Some(Err(_e)) => {
          cancel.cancel();
          break;
        }
        None => {
          cancel.cancel();
          break;
        }
      }
    }
  };

  let periphery_to_core = async {
    loop {
      // Already adheres to cancellation token
      match periphery_receiver.recv().await {
        Ok(bytes) => {
          if let Err(e) =
            client_send.send(ws::Message::Binary(bytes.into())).await
          {
            debug!("{e:?}");
            cancel.cancel();
            break;
          };
        }
        Err(_) => {
          let _ =
            client_send.send(ws::Message::text("STREAM EOF")).await;
          cancel.cancel();
          break;
        }
      }
    }
  };

  tokio::join!(core_to_periphery, periphery_to_core);

  // Cleanup
  if let Err(e) = periphery
    .request(DisconnectTerminal {
      id: periphery_connection_id,
    })
    .await
  {
    warn!(
      "Failed to disconnect Periphery terminal forwarding | {e:#}",
    )
  }
  if let Some(connection) =
    periphery_connections().get(&periphery.id).await
  {
    connection.terminals.remove(&periphery_connection_id).await;
  }
}
