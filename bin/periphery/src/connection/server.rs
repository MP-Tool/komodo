use std::{
  net::{IpAddr, SocketAddr},
  str::FromStr,
  sync::{
    Arc, OnceLock,
    atomic::{self, AtomicBool},
  },
};

use anyhow::{Context, anyhow};
use axum::{
  Router,
  body::Body,
  extract::{ConnectInfo, Query, WebSocketUpgrade},
  http::{HeaderMap, Request, StatusCode},
  middleware::{self, Next},
  response::Response,
  routing::get,
};
use axum_server::tls_rustls::RustlsConfig;
use bytes::Bytes;
use serror::{
  AddStatusCode, AddStatusCodeError, deserialize_error_bytes,
  serialize_error_bytes,
};
use transport::{
  CoreConnectionQuery, MessageState,
  auth::{
    ConnectionIdentifiers, HeaderConnectionIdentifiers,
    ServerLoginFlow,
  },
  websocket::{Websocket, axum::AxumWebsocket},
};

use crate::{
  api::Args,
  config::{core_public_keys, periphery_config},
  connection::core_channels,
};

pub async fn run() -> anyhow::Result<()> {
  let config = periphery_config();

  let addr = format!("{}:{}", config.bind_ip, config.port);

  let socket_addr = SocketAddr::from_str(&addr)
    .context("failed to parse listen address")?;

  let app = Router::new()
    .route("/", get(crate::connection::server::handler))
    .layer(middleware::from_fn(guard_request_by_ip))
    .into_make_service_with_connect_info::<SocketAddr>();

  if config.ssl_enabled {
    info!("🔒 Periphery SSL Enabled");
    crate::helpers::ensure_ssl_certs().await;
    info!("Komodo Periphery starting on wss://{}", socket_addr);
    let ssl_config = RustlsConfig::from_pem_file(
      config.ssl_cert_file(),
      config.ssl_key_file(),
    )
    .await
    .context("Invalid ssl cert / key")?;
    axum_server::bind_rustls(socket_addr, ssl_config)
      .serve(app)
      .await?
  } else {
    info!("🔓 Periphery SSL Disabled");
    info!("Komodo Periphery starting on ws://{}", socket_addr);
    axum_server::bind(socket_addr).serve(app).await?
  }

  Ok(())
}

fn already_logged_login_error() -> &'static AtomicBool {
  static ALREADY_LOGGED: OnceLock<AtomicBool> = OnceLock::new();
  ALREADY_LOGGED.get_or_init(|| AtomicBool::new(false))
}

async fn handler(
  Query(CoreConnectionQuery { core }): Query<CoreConnectionQuery>,
  mut headers: HeaderMap,
  ws: WebSocketUpgrade,
) -> serror::Result<Response> {
  let identifiers =
    HeaderConnectionIdentifiers::extract(&mut headers)
      .status_code(StatusCode::UNAUTHORIZED)?;

  let args = Arc::new(Args { core });

  let channel =
    core_channels().get_or_insert_default(&args.core).await;

  // Ensure the receiver is free before upgrading connection.
  // Due to ownership, it needs to be re-locked inside the ws handler,
  // opening up a possible timing edge case, but should be good enough.
  drop(
    channel
      .receiver()
      .with_context(|| {
        format!("Connection for {} is already connected", args.core)
      })
      .inspect_err(|e| warn!("{e:#}"))?,
  );

  Ok(ws.on_upgrade(|socket| async move {
    let mut socket = AxumWebsocket(socket);

    let query = format!("core={}", urlencoding::encode(&args.core));

    if let Err(e) =
      handle_login(&mut socket, identifiers.build(query.as_bytes()))
        .await
    {
      let already_logged = already_logged_login_error();
      if !already_logged.load(atomic::Ordering::Relaxed) {
        warn!("Core failed to login to connection | {e:#}");
        already_logged.store(true, atomic::Ordering::Relaxed);
      }
      // End the connection
      return;
    }

    already_logged_login_error()
      .store(false, atomic::Ordering::Relaxed);

    let mut receiver = match channel.receiver() {
      Ok(receiver) => receiver,
      Err(e) => {
        warn!("Failed to forward connection | {e:#}");

        let mut bytes = serialize_error_bytes(&e);
        bytes.push(MessageState::Failed.as_byte());

        if let Err(e) = socket
          .send(bytes.into())
          .await
          .context("Failed to send forward failed to client")
        {
          // Log additional error
          warn!("{e:#}");
        }

        // Close socket
        let _ = socket.close(None).await;

        return;
      }
    };

    super::handle_socket(
      socket,
      &args,
      &channel.sender,
      &mut receiver,
    )
    .await
  }))
}

/// Custom Core -> Periphery side only login wrapper
/// to implement passkey support for backward compatibility
async fn handle_login(
  socket: &mut AxumWebsocket,
  identifiers: ConnectionIdentifiers<'_>,
) -> anyhow::Result<()> {
  match (core_public_keys(), &periphery_config().passkeys) {
    (Some(_), _) | (_, None) => {
      // Send login type [0] (Noise auth)
      socket
        .send(Bytes::from_owner([0]))
        .await
        .context("Failed to send login type indicator")?;
      super::handle_login::<_, ServerLoginFlow>(socket, identifiers)
        .await
    }
    (None, Some(passkeys)) => {
      handle_passkey_login(socket, passkeys).await
    }
  }
}

async fn handle_passkey_login(
  socket: &mut AxumWebsocket,
  passkeys: &[String],
) -> anyhow::Result<()> {
  warn!(
    "Authenticating using Passkeys. Set 'core_public_key' (PERIPHERY_CORE_PUBLIC_KEY) instead to enhance security."
  );
  let res = async {
    // Send login type
    socket
      // Passkey auth: [1]
      .send(Bytes::from_owner([1]))
      .await
      .context("Failed to send login type indicator")?;

    // Receieve passkey
    let bytes = socket
      .recv_bytes()
      .await
      .context("Failed to receive passkey from Core")?;
    let passkey = match MessageState::from_byte(
      *bytes.last().context("passkey message is empty")?,
    ) {
      MessageState::Successful => &bytes[..(bytes.len() - 1)],
      _ => {
        return Err(deserialize_error_bytes(
          &bytes[..(bytes.len() - 1)],
        ));
      }
    };

    if passkeys
      .iter()
      .any(|expected_passkey| expected_passkey.as_bytes() == passkey)
    {
      socket
        .send(MessageState::Successful.into())
        .await
        .context("Failed to send login type indicator")?;
      Ok(())
    } else {
      let e = anyhow!("Invalid passkey");
      let mut bytes = serialize_error_bytes(&e);
      bytes.push(MessageState::Failed.as_byte());
      if let Err(e) = socket
        .send(bytes.into())
        .await
        .context("Failed to send login failed")
      {
        // Log additional error
        warn!("{e:#}");
        // Close socket
        let _ = socket.close(None).await;
      }
      // Return the original error
      Err(e)
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

async fn guard_request_by_ip(
  req: Request<Body>,
  next: Next,
) -> serror::Result<Response> {
  if periphery_config().allowed_ips.is_empty() {
    return Ok(next.run(req).await);
  }
  let ConnectInfo(socket_addr) = req
    .extensions()
    .get::<ConnectInfo<SocketAddr>>()
    .context("could not get ConnectionInfo of request")
    .status_code(StatusCode::UNAUTHORIZED)?;
  let ip = socket_addr.ip();

  let ip_match = periphery_config().allowed_ips.iter().any(|net| {
    net.contains(ip)
      || match ip {
        IpAddr::V4(ipv4) => {
          net.contains(IpAddr::V6(ipv4.to_ipv6_mapped()))
        }
        IpAddr::V6(_) => net.contains(ip.to_canonical()),
      }
  });

  if ip_match {
    Ok(next.run(req).await)
  } else {
    Err(
      anyhow!("requesting ip {ip} not allowed")
        .status_code(StatusCode::UNAUTHORIZED),
    )
  }
}
