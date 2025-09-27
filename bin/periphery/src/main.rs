use futures::{StreamExt, stream::FuturesUnordered};

use crate::config::periphery_public_key;

#[macro_use]
extern crate tracing;

mod api;
mod config;
mod connection;
mod docker;
mod helpers;
mod stats;
mod terminal;

async fn app() -> anyhow::Result<()> {
  dotenvy::dotenv().ok();
  let config = config::periphery_config();
  logger::init(&config.logging)?;

  info!("Komodo Periphery version: v{}", env!("CARGO_PKG_VERSION"));
  // Init public key to crash on failure
  info!("Periphery Public Key: {}", periphery_public_key());

  if config.pretty_startup_config {
    info!("{:#?}", config.sanitized());
  } else {
    info!("{:?}", config.sanitized());
  }

  install_crypto_provider();
  stats::spawn_polling_thread();
  docker::stats::spawn_polling_thread();

  let mut handles = FuturesUnordered::new();

  // Spawn client side connections
  match (&config.core_addresses, &config.connect_as) {
    (Some(addresses), Some(connect_as)) => {
      for address in addresses {
        handles.push(tokio::spawn(connection::client::handler(
          address, connect_as,
        )));
      }
    }
    (Some(_), None) => {
      warn!(
        "'core_addresses' are defined for outbound connection, but missing 'connect_as' (PERIPHERY_CONNECT_AS)"
      );
    }
    _ => {}
  }

  // Spawn server connection handler
  if config.server_enabled {
    handles.push(tokio::spawn(connection::server::run()));
  }

  // Watch the threads
  while let Some(res) = handles.next().await {
    match res {
      Ok(Err(e)) => {
        error!("CONNECTION ERROR: {e:#}");
      }
      Err(e) => {
        error!("SPAWN ERROR: {e:#}");
      }
      Ok(Ok(())) => {}
    }
  }

  Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let mut term_signal = tokio::signal::unix::signal(
    tokio::signal::unix::SignalKind::terminate(),
  )?;

  let app = tokio::spawn(app());

  tokio::select! {
    res = app => return res?,
    _ = term_signal.recv() => {
      info!("Exiting all active Terminals for shutdown");
      terminal::delete_all_terminals().await;
    },
  }

  Ok(())
}

fn install_crypto_provider() {
  if let Err(e) =
    rustls::crypto::aws_lc_rs::default_provider().install_default()
  {
    error!("Failed to install default crypto provider | {e:?}");
    std::process::exit(1);
  };
}
