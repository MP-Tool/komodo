use futures::{StreamExt, stream::FuturesUnordered};
use komodo_client::entities::config::periphery::Command;

use crate::config::{periphery_args, periphery_public_key};

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

  rustls::crypto::aws_lc_rs::default_provider()
    .install_default()
    .expect("Failed to install default crypto provider");

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
  // Handle `periphery key gen` and `periphery key compute <private-key>`
  if let Some(Command::Key { command }) = &periphery_args().command {
    return keygen::handle_key_command(command).await;
  }

  dotenvy::dotenv().ok();

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
