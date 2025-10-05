use std::sync::Arc;

use anyhow::Context;
use axum::http::HeaderValue;
use futures_util::{
  SinkExt, Stream, StreamExt, TryStreamExt,
  stream::{SplitSink, SplitStream},
};
use rustls::{ClientConfig, client::danger::ServerCertVerifier};
use serror::AddStatusCodeError;
use tokio::net::TcpStream;
use tokio_tungstenite::{
  Connector, MaybeTlsStream, WebSocketStream,
  tungstenite::{
    self, handshake::client::Response, protocol::CloseFrame,
  },
};

use super::{
  MaybeWithTimeout, Websocket, WebsocketMessage, WebsocketReceiver,
  WebsocketSender,
};

pub type InnerWebsocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct TungsteniteWebsocket(pub InnerWebsocket);

impl Websocket for TungsteniteWebsocket {
  type CloseFrame = CloseFrame;
  type Error = tungstenite::Error;

  fn split(self) -> (impl WebsocketSender, impl WebsocketReceiver) {
    let (tx, rx) = self.0.split();
    (
      TungsteniteWebsocketSender(tx),
      TungsteniteWebsocketReceiver(rx),
    )
  }

  fn recv(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<
      Output = Result<
        WebsocketMessage<Self::CloseFrame>,
        Self::Error,
      >,
    >,
  > {
    MaybeWithTimeout {
      inner: try_next(&mut self.0),
    }
  }

  async fn send(
    &mut self,
    bytes: bytes::Bytes,
  ) -> Result<(), Self::Error> {
    self.0.send(tungstenite::Message::Binary(bytes)).await
  }

  async fn close(
    &mut self,
    frame: Option<Self::CloseFrame>,
  ) -> Result<(), Self::Error> {
    self.0.close(frame).await
  }
}

pub type InnerWebsocketReceiver =
  SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

pub struct TungsteniteWebsocketReceiver(pub InnerWebsocketReceiver);

impl WebsocketReceiver for TungsteniteWebsocketReceiver {
  type CloseFrame = CloseFrame;
  type Error = tungstenite::Error;

  async fn recv(
    &mut self,
  ) -> Result<WebsocketMessage<Self::CloseFrame>, Self::Error> {
    try_next(&mut self.0).await
  }
}

pub type InnerWebsocketSender = SplitSink<
  WebSocketStream<MaybeTlsStream<TcpStream>>,
  tungstenite::Message,
>;

pub struct TungsteniteWebsocketSender(pub InnerWebsocketSender);

impl WebsocketSender for TungsteniteWebsocketSender {
  type CloseFrame = CloseFrame;
  type Error = tungstenite::Error;

  async fn send(
    &mut self,
    bytes: bytes::Bytes,
  ) -> Result<(), Self::Error> {
    self.0.send(tungstenite::Message::Binary(bytes)).await
  }

  async fn close(
    &mut self,
    frame: Option<Self::CloseFrame>,
  ) -> Result<(), Self::Error> {
    self.0.send(tungstenite::Message::Close(frame)).await
  }
}

async fn try_next<S>(
  stream: &mut S,
) -> Result<WebsocketMessage<CloseFrame>, tungstenite::Error>
where
  S: Stream<Item = Result<tungstenite::Message, tungstenite::Error>>
    + Unpin,
{
  loop {
    match stream.try_next().await? {
      Some(tungstenite::Message::Binary(bytes)) => {
        return Ok(WebsocketMessage::Binary(bytes));
      }
      Some(tungstenite::Message::Text(text)) => {
        return Ok(WebsocketMessage::Binary(text.into()));
      }
      Some(tungstenite::Message::Close(frame)) => {
        return Ok(WebsocketMessage::Close(frame));
      }
      None => return Ok(WebsocketMessage::Closed),
      // Ignored messages
      Some(tungstenite::Message::Ping(_))
      | Some(tungstenite::Message::Pong(_))
      | Some(tungstenite::Message::Frame(_)) => continue,
    }
  }
}

impl TungsteniteWebsocket {
  pub async fn connect_maybe_tls_insecure(
    url: &str,
    insecure: bool,
  ) -> serror::Result<(Self, HeaderValue)> {
    if insecure {
      Self::connect(url).await
    } else {
      Self::connect_tls_insecure(url).await
    }
  }

  pub async fn connect(
    url: &str,
  ) -> serror::Result<(Self, HeaderValue)> {
    let res = tokio_tungstenite::connect_async(url).await;
    Self::handle_connection_result(url, res)
  }

  pub async fn connect_tls_insecure(
    url: &str,
  ) -> serror::Result<(Self, HeaderValue)> {
    let res = tokio_tungstenite::connect_async_tls_with_config(
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
    .await;
    Self::handle_connection_result(url, res)
  }

  fn handle_connection_result(
    url: &str,
    res: Result<
      (WebSocketStream<MaybeTlsStream<TcpStream>>, Response),
      tungstenite::Error,
    >,
  ) -> serror::Result<(Self, HeaderValue)> {
    let (ws, mut response) = res
      .map_err(|e| {
        let status = if let tungstenite::Error::Http(response) = &e {
          response.status()
        } else {
          return anyhow::Error::from(e).into();
        };
        e.status_code(status)
      })
      .map_err(|mut e| {
        e.error = e.error.context({
          format!("Failed to connect to websocket | url: {url}")
        });
        e
      })?;

    let accept = response
      .headers_mut()
      .remove("sec-websocket-accept")
      .context("Headers do not contain Sec-Websocket-Accept")?;

    Ok((Self(ws), accept))
  }
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
