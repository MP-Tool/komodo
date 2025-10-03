use std::{sync::Arc, time::Duration};

use anyhow::{Context, anyhow};
use axum::http::{HeaderValue, StatusCode};
use periphery_client::CONNECTION_RETRY_SECONDS;
use tokio_tungstenite::tungstenite;
use transport::{
  auth::{AddressConnectionIdentifiers, ClientLoginFlow},
  fix_ws_address,
  websocket::tungstenite::TungsteniteWebsocket,
};

use crate::{api::Args, connection::core_channels};

pub async fn handler(
  address: &str,
  connect_as: &str,
) -> anyhow::Result<()> {
  let address = fix_ws_address(address);
  let identifiers = AddressConnectionIdentifiers::extract(&address)?;
  let query = format!("server={}", urlencoding::encode(connect_as));
  let endpoint = format!("{address}/ws/periphery?{query}");

  info!("Initiating outbound connection to {endpoint}");

  let mut already_logged_connection_error = false;
  let mut already_logged_login_error = false;

  let args = Arc::new(Args {
    core: identifiers.host().to_string(),
  });

  let channel =
    core_channels().get_or_insert_default(&args.core).await;

  let mut receiver = channel.receiver()?;

  loop {
    let (mut socket, accept) =
      match connect_websocket(&endpoint, connect_as).await {
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

    if let Err(e) = super::handle_login::<_, ClientLoginFlow>(
      &mut socket,
      identifiers.build(accept.as_bytes(), query.as_bytes()),
    )
    .await
    {
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

    super::handle_socket(
      socket,
      &args,
      &channel.sender,
      &mut receiver,
    )
    .await
  }
}

async fn connect_websocket(
  url: &str,
  connect_as: &str,
) -> anyhow::Result<(TungsteniteWebsocket, HeaderValue)> {
  let (ws, mut response) = tokio_tungstenite::connect_async(url)
    .await
    .map_err(|e| match e {
      tungstenite::Error::Http(response) => match response.status() {
        StatusCode::NOT_FOUND => anyhow!("404 Not Found: Server '{connect_as}' does not exist."),
        StatusCode::BAD_REQUEST => anyhow!("400 Bad Request: Server '{connect_as}' is disabled or configured to make Core → Periphery connection"),
        StatusCode::UNAUTHORIZED => anyhow!("401 Unauthorized: Only one Server connected as '{connect_as}' is allowed. Or the Core reverse proxy needs to forward host and websocket headers."),
        _ => anyhow::Error::from(tungstenite::Error::Http(response)),
      },
      e => anyhow::Error::from(e),
    })
    .with_context(|| format!("Failed to connect to {url}"))?;
  let accept = response
    .headers_mut()
    .remove("sec-websocket-accept")
    .context("sec-websocket-accept")
    .context("Headers do not contain Sec-Websocket-Accept")?;
  Ok((TungsteniteWebsocket(ws), accept))
}
