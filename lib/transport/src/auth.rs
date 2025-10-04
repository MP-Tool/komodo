//! Implementes both sides of Noise handshake
//! using asymmetric private-public key authentication.
//!
//! TODO: Revisit
//! Note. Relies on Server being behind trusted TLS connection.
//! This is trivial for Periphery -> Core connection, but presents a challenge
//! for Core -> Periphery, where untrusted TLS certs are being used.

use std::time::Duration;

use anyhow::Context;
use axum::http::{HeaderMap, HeaderValue};
use base64::{Engine, prelude::BASE64_STANDARD};
use bytes::Bytes;
use noise::{NoiseHandshake, key::SpkiPublicKey};
use rand::RngCore;
use serror::{deserialize_error_bytes, serialize_error_bytes};
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::{MessageState, websocket::Websocket};

pub trait PublicKeyValidator {
  type ValidationResult;
  fn validate(
    &self,
    public_key: String,
  ) -> impl Future<Output = anyhow::Result<Self::ValidationResult>>;
}

pub struct LoginFlowArgs<'a, 's, V, W> {
  pub identifiers: ConnectionIdentifiers<'a>,
  pub private_key: &'a str,
  pub public_key_validator: V,
  pub socket: &'s mut W,
}

pub trait LoginFlow {
  fn login<'a, 's, V: PublicKeyValidator, W: Websocket>(
    args: LoginFlowArgs<'a, 's, V, W>,
  ) -> impl Future<Output = anyhow::Result<V::ValidationResult>>;
}

const AUTH_TIMEOUT: Duration = Duration::from_secs(2);

pub struct ServerLoginFlow;

impl LoginFlow for ServerLoginFlow {
  async fn login<'a, 's, V: PublicKeyValidator, W: Websocket>(
    LoginFlowArgs {
      identifiers,
      private_key,
      public_key_validator,
      socket,
    }: LoginFlowArgs<'a, 's, V, W>,
  ) -> anyhow::Result<V::ValidationResult> {
    let res = async {
      // Server generates random nonce and sends to client
      let nonce = nonce();
      socket
        .send(Bytes::from_owner(nonce))
        .await
        .context("Failed to send connection nonce")?;

      let mut handshake = NoiseHandshake::new_responder(
        private_key,
        // Builds the handshake using the connection-unique prologue hash.
        // The prologue must be the same on both sides of connection.
        &identifiers.hash(&nonce),
      )
      .context("Failed to inialize handshake")?;

      // Receive and read handshake_m1
      let handshake_m1 = socket
        .recv_bytes_with_timeout(AUTH_TIMEOUT)
        .await
        .context("Failed to get handshake_m1")?;
      match MessageState::from_byte(
        *handshake_m1.last().context("handshake_m1 is empty")?,
      ) {
        MessageState::Successful => handshake
          .read_message(&handshake_m1[..(handshake_m1.len() - 1)])
          .context("Failed to read handshake_m1")?,
        _ => {
          return Err(deserialize_error_bytes(
            &handshake_m1[..(handshake_m1.len() - 1)],
          ));
        }
      }

      // Send handshake_m2
      let mut handshake_m2 = handshake
        .next_message()
        .context("Failed to write handshake_m2")?;
      handshake_m2.push(MessageState::Successful.as_byte());
      socket
        .send(handshake_m2.into())
        .await
        .context("Failed to send handshake_m2")?;

      // Receive and read handshake_m3
      let handshake_m3 = socket
        .recv_bytes_with_timeout(AUTH_TIMEOUT)
        .await
        .context("Failed to get handshake_m3")?;
      match MessageState::from_byte(
        *handshake_m3.last().context("handshake_m3 is empty")?,
      ) {
        MessageState::Successful => handshake
          .read_message(&handshake_m3[..(handshake_m3.len() - 1)])
          .context("Failed to read handshake_m3")?,
        _ => {
          return Err(deserialize_error_bytes(
            &handshake_m3[..(handshake_m3.len() - 1)],
          ));
        }
      }

      // Server now has client public key
      let public_key =
        SpkiPublicKey::from_raw_bytes(handshake.remote_public_key()?)
          .context("Invalid public key")?
          .into_inner();

      public_key_validator.validate(public_key).await
    }
    .await;

    match res {
      Ok(res) => {
        socket
          .send(MessageState::Successful.into())
          .await
          .context("Failed to send login successful to client")?;
        Ok(res)
      }
      Err(e) => {
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
      }
    }
  }
}

pub struct ClientLoginFlow;

impl LoginFlow for ClientLoginFlow {
  async fn login<'a, 's, V: PublicKeyValidator, W: Websocket>(
    LoginFlowArgs {
      identifiers,
      private_key,
      public_key_validator,
      socket,
    }: LoginFlowArgs<'a, 's, V, W>,
  ) -> anyhow::Result<V::ValidationResult> {
    let res = async {
      // Receive nonce from server
      let nonce = socket
        .recv_bytes_with_timeout(AUTH_TIMEOUT)
        .await
        .context("Failed to receive connection nonce")?;

      let mut handshake = NoiseHandshake::new_initiator(
        private_key,
        // Builds the handshake using the connection-unique prologue hash.
        // The prologue must be the same on both sides of connection.
        &identifiers.hash(&nonce),
      )
      .context("Failed to inialize handshake")?;

      // Send handshake_m1
      let mut handshake_m1 = handshake
        .next_message()
        .context("Failed to write handshake m1")?;
      handshake_m1.push(MessageState::Successful.as_byte());
      socket
        .send(handshake_m1.into())
        .await
        .context("Failed to send handshake_m1")?;

      // Receive and read handshake_m2
      let handshake_m2 = socket
        .recv_bytes_with_timeout(AUTH_TIMEOUT)
        .await
        .context("Failed to get handshake_m2")?;
      match MessageState::from_byte(
        *handshake_m2.last().context("handshake_m2 is empty")?,
      ) {
        MessageState::Successful => handshake
          .read_message(&handshake_m2[..(handshake_m2.len() - 1)])
          .context("Failed to read handshake_m2")?,
        _ => {
          return Err(deserialize_error_bytes(
            &handshake_m2[..(handshake_m2.len() - 1)],
          ));
        }
      }

      // Client now has server public key.
      // Perform validation before proceeding.
      let public_key =
        SpkiPublicKey::from_raw_bytes(handshake.remote_public_key()?)
          .context("Invalid public key")?
          .into_inner();
      let validation_result =
        public_key_validator.validate(public_key).await?;

      // Send handshake_m3
      let mut handshake_m3 = handshake
        .next_message()
        .context("Failed to write handshake_m3")?;
      handshake_m3.push(MessageState::Successful.as_byte());
      socket
        .send(handshake_m3.into())
        .await
        .context("Failed to send handshake_m3")?;

      // Receive login state message and return based on value
      let state_msg = socket
        .recv_bytes_with_timeout(AUTH_TIMEOUT)
        .await
        .context("Failed to receive authentication state message")?;
      let state = state_msg.last().context(
        "Authentication state message did not contain state byte",
      )?;
      match MessageState::from_byte(*state) {
        MessageState::Successful => anyhow::Ok(validation_result),
        _ => Err(deserialize_error_bytes(
          &state_msg[..(state_msg.len() - 1)],
        )),
      }
    }
    .await;

    match res {
      Ok(res) => Ok(res),
      Err(e) => {
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
      }
    }
  }
}

fn nonce() -> [u8; 32] {
  let mut out = [0u8; 32];
  rand::rng().fill_bytes(&mut out);
  out
}

#[derive(Clone, Copy)]
pub struct ConnectionIdentifiers<'a> {
  /// Server hostname
  pub host: &'a [u8],
  /// Query: 'server=<SERVER>'
  pub query: &'a [u8],
  /// Sec-Websocket-Accept, unique for each connection
  pub accept: &'a [u8],
}

impl ConnectionIdentifiers<'_> {
  /// nonce: Server computed random connection nonce, sent to client before auth handshake
  pub fn hash(&self, nonce: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"noise-wss-v1|");
    hash.update(self.host);
    hash.update(b"|");
    hash.update(self.query);
    hash.update(b"|");
    hash.update(self.accept);
    hash.update(b"|");
    hash.update(nonce);
    hash.finalize().into()
  }
}

pub struct AddressConnectionIdentifiers {
  host: String,
}

impl AddressConnectionIdentifiers {
  pub fn extract(address: &str) -> anyhow::Result<Self> {
    let url = ::url::Url::parse(address)
      .context("Failed to parse server address")?;
    let mut host = url.host().context("url has no host")?.to_string();
    if let Some(port) = url.port() {
      host.push(':');
      host.push_str(&port.to_string());
    };
    Ok(Self { host })
  }

  pub fn host(&self) -> &String {
    &self.host
  }

  pub fn build<'a>(
    &'a self,
    accept: &'a [u8],
    query: &'a [u8],
  ) -> ConnectionIdentifiers<'a> {
    ConnectionIdentifiers {
      host: self.host.as_bytes(),
      query,
      accept,
    }
  }
}

/// Used to extract owned connection identifier
/// in server side connection handler.
pub struct HeaderConnectionIdentifiers {
  host: HeaderValue,
  accept: String,
}

impl HeaderConnectionIdentifiers {
  pub fn extract(
    headers: &mut HeaderMap,
  ) -> anyhow::Result<HeaderConnectionIdentifiers> {
    let host = headers
      .remove("x-forwarded-host")
      .or(headers.remove("host"))
      .context("Failed to get connection host")?;
    let key = headers
      .remove("sec-websocket-key")
      .context("Headers do not contain Sec-Websocket-Key")?;
    let accept = compute_accept(key.as_bytes());
    Ok(HeaderConnectionIdentifiers { host, accept })
  }

  pub fn host(&self) -> anyhow::Result<String> {
    self
      .host
      .to_str()
      .map(str::to_string)
      .context("Failed to parse header host to string")
  }

  pub fn build<'a>(
    &'a self,
    query: &'a [u8],
  ) -> ConnectionIdentifiers<'a> {
    ConnectionIdentifiers {
      host: self.host.as_bytes(),
      accept: self.accept.as_bytes(),
      query,
    }
  }
}

pub fn compute_accept(sec_websocket_key: &[u8]) -> String {
  // This is standard GUID to compute Sec-Websocket-Accept
  const GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
  let mut sha1 = sha1::Sha1::new();
  sha1.update(sec_websocket_key);
  sha1.update(GUID);
  let digest = sha1.finalize();
  BASE64_STANDARD.encode(digest)
}
