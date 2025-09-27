use std::{sync::Arc, time::Duration};

use anyhow::{Context, anyhow};
use axum::http::HeaderValue;
use periphery_client::CONNECTION_RETRY_SECONDS;
use rustls::{ClientConfig, client::danger::ServerCertVerifier};
use serror::{deserialize_error_bytes, serialize_error_bytes};
use tokio_tungstenite::Connector;
use transport::{
  MessageState,
  auth::{
    AddressConnectionIdentifiers, ClientLoginFlow,
    ConnectionIdentifiers,
  },
  fix_ws_address,
  websocket::{Websocket, tungstenite::TungsteniteWebsocket},
};

use crate::{
  config::{core_config, core_connection_query},
  connection::{PeripheryConnection, PeripheryConnectionArgs},
  periphery::ConnectionChannels,
  state::periphery_connections,
};

impl PeripheryConnectionArgs<'_> {
  pub async fn spawn_client_connection(
    self,
    server_id: String,
    passkey: String,
  ) -> anyhow::Result<Arc<ConnectionChannels>> {
    if self.address.is_empty() {
      return Err(anyhow!(
        "Cannot spawn client connection with empty address"
      ));
    }

    let address = fix_ws_address(self.address);
    let identifiers =
      AddressConnectionIdentifiers::extract(&address)?;
    let endpoint = format!("{address}/?{}", core_connection_query());

    let (connection, mut receiver) =
      periphery_connections().insert(server_id, self).await;

    let channels = connection.channels.clone();

    tokio::spawn(async move {
      loop {
        let ws = tokio::select! {
          ws = connect_websocket(&endpoint) => ws,
          _ = connection.cancel.cancelled() => {
            break
          }
        };

        let (mut socket, accept) = match ws {
          Ok(res) => res,
          Err(e) => {
            connection.set_error(e).await;
            tokio::time::sleep(Duration::from_secs(
              CONNECTION_RETRY_SECONDS,
            ))
            .await;
            continue;
          }
        };

        let identifiers = identifiers.build(
          accept.as_bytes(),
          core_connection_query().as_bytes(),
        );

        if let Err(e) = connection
          .client_login(&mut socket, identifiers, &passkey)
          .await
        {
          if connection.cancel.is_cancelled() {
            break;
          }
          connection.set_error(e).await;
          tokio::time::sleep(Duration::from_secs(
            CONNECTION_RETRY_SECONDS,
          ))
          .await;
          continue;
        };

        connection.handle_socket(socket, &mut receiver).await
      }
    });

    Ok(channels)
  }
}

impl PeripheryConnection {
  /// Custom Core -> Periphery side only login wrapper
  /// to implement passkey support for backward compatibility
  async fn client_login(
    &self,
    socket: &mut TungsteniteWebsocket,
    identifiers: ConnectionIdentifiers<'_>,
    // for legacy auth
    passkey: &str,
  ) -> anyhow::Result<()> {
    // Get the required auth type
    let bytes = socket
      .recv_bytes()
      .await
      .context("Failed to receive login type indicator")?;

    match bytes.iter().as_slice() {
      // Noise auth
      &[0] => {
        self
          .handle_login::<_, ClientLoginFlow>(socket, identifiers)
          .await
      }
      // Passkey auth
      &[1] => handle_passkey_login(socket, passkey).await,
      other => Err(anyhow!(
        "Receieved invalid login type pattern: {other:?}"
      )),
    }
  }
}

async fn handle_passkey_login(
  socket: &mut TungsteniteWebsocket,
  // for legacy auth
  passkey: &str,
) -> anyhow::Result<()> {
  let res = async {
    let mut passkey = if passkey.is_empty() {
      core_config()
        .passkey
        .as_deref()
        .context("Periphery requires passkey auth")?
        .as_bytes()
        .to_vec()
    } else {
      passkey.as_bytes().to_vec()
    };
    passkey.push(MessageState::Successful.as_byte());

    socket
      .send(passkey.into())
      .await
      .context("Failed to send passkey")?;

    // Receive login state message and return based on value
    let state_msg = socket
      .recv_bytes()
      .await
      .context("Failed to receive authentication state message")?;
    let state = state_msg.last().context(
      "Authentication state message did not contain state byte",
    )?;
    match MessageState::from_byte(*state) {
      MessageState::Successful => anyhow::Ok(()),
      _ => Err(deserialize_error_bytes(
        &state_msg[..(state_msg.len() - 1)],
      )),
    }
  }
  .await;
  if let Err(e) = res {
    let mut bytes = serialize_error_bytes(&e);
    bytes.push(MessageState::Failed.as_byte());
    if let Err(e) = socket
      .send(bytes.into())
      .await
      .context("Failed to send login failed to client")
    {
      // Log additional error
      warn!("{e:#}");
    }
    // Close socket
    let _ = socket.close(None).await;
    // Return the original error
    Err(e)
  } else {
    Ok(())
  }
}

async fn connect_websocket(
  url: &str,
) -> anyhow::Result<(TungsteniteWebsocket, HeaderValue)> {
  let (ws, mut response) = if url.starts_with("wss") {
    tokio_tungstenite::connect_async_tls_with_config(
      url,
      None,
      false,
      Some(Connector::Rustls(Arc::new(
        ClientConfig::builder()
          .dangerous()
          .with_custom_certificate_verifier(Arc::new(
            InsecureVerifier,
          ))
          .with_no_client_auth(),
      ))),
    )
    .await
    .with_context(|| {
      format!("failed to connect to websocket | url: {url}")
    })?
  } else {
    tokio_tungstenite::connect_async(url).await.with_context(
      || format!("failed to connect to websocket | url: {url}"),
    )?
  };

  let accept = response
    .headers_mut()
    .remove("sec-websocket-accept")
    .context("sec-websocket-accept")?;

  Ok((TungsteniteWebsocket(ws), accept))
}

#[derive(Debug)]
struct InsecureVerifier;

impl ServerCertVerifier for InsecureVerifier {
  fn verify_server_cert(
    &self,
    _end_entity: &rustls::pki_types::CertificateDer<'_>,
    _intermediates: &[rustls::pki_types::CertificateDer<'_>],
    _server_name: &rustls::pki_types::ServerName<'_>,
    _ocsp_response: &[u8],
    _now: rustls::pki_types::UnixTime,
  ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error>
  {
    Ok(rustls::client::danger::ServerCertVerified::assertion())
  }

  fn verify_tls12_signature(
    &self,
    _message: &[u8],
    _cert: &rustls::pki_types::CertificateDer<'_>,
    _dss: &rustls::DigitallySignedStruct,
  ) -> Result<
    rustls::client::danger::HandshakeSignatureValid,
    rustls::Error,
  > {
    Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
  }

  fn verify_tls13_signature(
    &self,
    _message: &[u8],
    _cert: &rustls::pki_types::CertificateDer<'_>,
    _dss: &rustls::DigitallySignedStruct,
  ) -> Result<
    rustls::client::danger::HandshakeSignatureValid,
    rustls::Error,
  > {
    Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
  }

  fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
    vec![
      rustls::SignatureScheme::RSA_PKCS1_SHA1,
      rustls::SignatureScheme::ECDSA_SHA1_Legacy,
      rustls::SignatureScheme::RSA_PKCS1_SHA256,
      rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
      rustls::SignatureScheme::RSA_PKCS1_SHA384,
      rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
      rustls::SignatureScheme::RSA_PKCS1_SHA512,
      rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
      rustls::SignatureScheme::RSA_PSS_SHA256,
      rustls::SignatureScheme::RSA_PSS_SHA384,
      rustls::SignatureScheme::RSA_PSS_SHA512,
      rustls::SignatureScheme::ED25519,
      rustls::SignatureScheme::ED448,
    ]
  }
}
