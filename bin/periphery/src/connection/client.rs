use std::time::Duration;

use anyhow::Context;
use axum::http::HeaderValue;
use periphery_client::CONNECTION_RETRY_SECONDS;
use transport::{
  auth::{AddressConnectionIdentifiers, ClientLoginFlow},
  fix_ws_address,
  websocket::tungstenite::TungsteniteWebsocket,
};

use crate::connection::ws_receiver;

pub async fn handler(
  address: &str,
  connect_as: &str,
) -> anyhow::Result<()> {
  if let Err(e) =
    rustls::crypto::aws_lc_rs::default_provider().install_default()
  {
    error!("Failed to install default crypto provider | {e:?}");
    std::process::exit(1);
  };

  let mut write_receiver = ws_receiver()
    .try_lock()
    .context("Websocket handler called more than once.")?;

  let address = fix_ws_address(address);
  let identifiers = AddressConnectionIdentifiers::extract(&address)?;
  let query = format!("server={}", urlencoding::encode(connect_as));
  let endpoint = format!("{address}/ws/periphery?{query}");

  info!("Initiating outbound connection to {endpoint}");

  let mut already_logged_connection_error = false;
  let mut already_logged_login_error = false;

  loop {
    let (socket, accept) = match connect_websocket(&endpoint).await {
      Ok(res) => res,
      Err(e) => {
        if !already_logged_connection_error {
          warn!("{e:#}");
          already_logged_connection_error = true;
          // If error transitions from login to connection,
          // set to false to see login error after reconnect.
          already_logged_login_error = false;
        }
        tokio::time::sleep(Duration::from_secs(
          CONNECTION_RETRY_SECONDS,
        ))
        .await;
        continue;
      }
    };

    already_logged_connection_error = false;

    if !already_logged_login_error {
      info!("Connected to core connection websocket");
    }

    let mut handler = super::WebsocketHandler {
      socket,
      connection_identifiers: identifiers
        .build(accept.as_bytes(), query.as_bytes()),
      write_receiver: &mut write_receiver,
    };

    if let Err(e) = handler.login::<ClientLoginFlow>().await {
      if !already_logged_login_error {
        warn!("Failed to login | {e:#}");
        already_logged_login_error = true;
      }
      tokio::time::sleep(Duration::from_secs(
        CONNECTION_RETRY_SECONDS,
      ))
      .await;
      continue;
    }

    already_logged_login_error = false;

    handler.handle().await
  }
}

async fn connect_websocket(
  url: &str,
) -> anyhow::Result<(TungsteniteWebsocket, HeaderValue)> {
  let (ws, mut response) = tokio_tungstenite::connect_async(url)
    .await
    .with_context(|| format!("Failed to connect to {url}"))?;
  let accept = response
    .headers_mut()
    .remove("sec-websocket-accept")
    .context("sec-websocket-accept")
    .context("Headers do not contain Sec-Websocket-Accept")?;
  Ok((TungsteniteWebsocket(ws), accept))
}
