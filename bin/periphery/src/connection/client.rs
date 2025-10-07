use std::{sync::Arc, time::Duration};

use anyhow::{Context, anyhow};
use axum::http::{HeaderValue, StatusCode};
use periphery_client::CONNECTION_RETRY_SECONDS;
use transport::{
  auth::{
    AUTH_TIMEOUT, AddressConnectionIdentifiers, ClientLoginFlow,
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

pub async fn handler(address: &str) -> anyhow::Result<()> {
  let address = fix_ws_address(address);
  let identifiers = AddressConnectionIdentifiers::extract(&address)?;
  let query = format!(
    "server={}",
    urlencoding::encode(&periphery_config().connect_as)
  );
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
      match connect_websocket(&endpoint).await {
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
      .recv_result()
      .with_timeout(AUTH_TIMEOUT)
      .await
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
        if let Err(e) = handle_onboarding(socket, identifiers).await {
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
  mut socket: TungsteniteWebsocket,
  identifiers: ConnectionIdentifiers<'_>,
) -> anyhow::Result<()> {
  let config = periphery_config();
  let onboarding_key = config
    .onboarding_key
    .as_deref()
    .with_context(|| format!("Server {} does not exist, and no PERIPHERY_ONBOARDING_KEY is provided.", config.connect_as))?;

  ClientLoginFlow::login(LoginFlowArgs {
    private_key: onboarding_key,
    identifiers,
    public_key_validator: CorePublicKeyValidator,
    socket: &mut socket,
  })
  .await?;

  // Post onboarding login 1: Send public key
  socket
    .send(periphery_public_key().load().as_bytes())
    .await
    .context("Failed to send public key bytes")?;

  socket
    .recv_result()
    .with_timeout(AUTH_TIMEOUT)
    .await
    .context("Failed to receive Server creation result")?;

  info!(
    "Server onboarding flow for '{}' successful ✅",
    config.connect_as
  );

  Ok(())
}

async fn connect_websocket(
  url: &str,
) -> anyhow::Result<(TungsteniteWebsocket, HeaderValue)> {
  let config = periphery_config();
  TungsteniteWebsocket::connect_maybe_tls_insecure(url, config.core_tls_insecure_skip_verify)
    .await
    .map_err(|e| match e.status {
      StatusCode::NOT_FOUND => anyhow!("404 Not Found: Server '{}' does not exist.", config.connect_as),
      StatusCode::BAD_REQUEST => anyhow!("400 Bad Request: Server '{}' is disabled or configured to make Core → Periphery connection", config.connect_as),
      StatusCode::UNAUTHORIZED => anyhow!("401 Unauthorized: Only one Server connected as '{}' is allowed. Or the Core reverse proxy needs to forward host and websocket headers.", config.connect_as),
      _ => e.error,
    })
}
