use std::{sync::Arc, time::Duration};

use anyhow::{Context, anyhow};
use axum::http::{HeaderValue, StatusCode};
use bytes::Bytes;
use periphery_client::CONNECTION_RETRY_SECONDS;
use serror::deserialize_error_bytes;
use tokio_tungstenite::tungstenite;
use transport::{
  MessageState,
  auth::{
    AddressConnectionIdentifiers, ClientLoginFlow,
    ConnectionIdentifiers, LoginFlow, LoginFlowArgs,
  },
  fix_ws_address,
  websocket::{Websocket, tungstenite::TungsteniteWebsocket},
};

use crate::{
  api::Args,
  config::{periphery_config, periphery_public_key},
  connection::{CorePublicKeyValidator, core_channels},
};

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
  let mut already_logged_onboarding_error = false;

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
            already_logged_onboarding_error = false;
          }
          tokio::time::sleep(Duration::from_secs(
            CONNECTION_RETRY_SECONDS,
          ))
          .await;
          continue;
        }
      };

    // Receive whether to use Server connection flow vs Server onboarding flow.

    let flow_bytes = match socket
      .recv_bytes()
      .with_timeout(Duration::from_secs(2))
      .await?
      .context("Failed to receive login flow indicator")
    {
      Ok(flow_bytes) => flow_bytes,
      Err(e) => {
        if !already_logged_connection_error {
          warn!("{e:#}");
          already_logged_connection_error = true;
          // If error transitions from login to connection,
          // set to false to see login error after reconnect.
          already_logged_login_error = false;
          already_logged_onboarding_error = false;
        }
        tokio::time::sleep(Duration::from_secs(
          CONNECTION_RETRY_SECONDS,
        ))
        .await;
        continue;
      }
    };

    already_logged_connection_error = false;

    let identifiers =
      identifiers.build(accept.as_bytes(), query.as_bytes());

    match flow_bytes.iter().as_slice() {
      // Connection (standard) flow
      &[0] => {
        if let Err(e) = super::handle_login::<_, ClientLoginFlow>(
          &mut socket,
          identifiers,
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
      // Creation
      &[1] => {
        if let Err(e) =
          handle_onboarding(connect_as, socket, identifiers).await
        {
          if !already_logged_onboarding_error {
            error!("{e:#}");
            already_logged_onboarding_error = true;
          }
          tokio::time::sleep(Duration::from_secs(
            CONNECTION_RETRY_SECONDS,
          ))
          .await;
          continue;
        };
      }
      // Other (error)
      other => {
        if !already_logged_connection_error {
          warn!("Receieved invalid login flow pattern: {other:?}");
          already_logged_connection_error = true;
          // If error transitions from login to connection,
          // set to false to see login error after reconnect.
          already_logged_login_error = false;
        }
        tokio::time::sleep(Duration::from_secs(
          CONNECTION_RETRY_SECONDS,
        ))
        .await;
      }
    };
  }
}

async fn handle_onboarding(
  connect_as: &str,
  mut socket: TungsteniteWebsocket,
  identifiers: ConnectionIdentifiers<'_>,
) -> anyhow::Result<()> {
  let onboarding_key = periphery_config()
    .onboarding_key
    .as_deref()
    .with_context(|| format!("Server {connect_as} does not exist, and no PERIPHERY_ONBOARDING_KEY is provided."))?;

  ClientLoginFlow::login(LoginFlowArgs {
    private_key: onboarding_key,
    identifiers,
    public_key_validator: CorePublicKeyValidator,
    socket: &mut socket,
  })
  .await?;

  // Post onboarding login 1: Send public key
  socket
    .send(Bytes::copy_from_slice(
      periphery_public_key().load().as_bytes(),
    ))
    .await
    .context("Failed to send public key bytes")?;

  let res = socket
    .recv_bytes()
    .with_timeout(Duration::from_secs(2))
    .await?
    .context("Failed to receive Server creation result")?;

  match res.last().map(|byte| MessageState::from_byte(*byte)) {
    Some(MessageState::Successful) => {
      info!(
        "Server onboarding flow for '{connect_as}' successful ✅"
      );
      Ok(())
    }
    Some(MessageState::Failed) => {
      Err(deserialize_error_bytes(&res[..(res.len() - 1)]))
    }
    other => Err(anyhow!(
      "Got unrecognized onboarding flow response: {other:?}"
    )),
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
