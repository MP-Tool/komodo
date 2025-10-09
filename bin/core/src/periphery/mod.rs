use std::{sync::Arc, time::Duration};

use anyhow::{Context, anyhow};
use periphery_client::api;
use resolver_api::HasResponse;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use transport::{
  channel::channel,
  message::{
    Decode, Encode, Message, json::JsonMessage, wrappers::WithChannel,
  },
};
use uuid::Uuid;

use crate::{
  connection::{
    PeripheryConnection, PeripheryConnectionArgs, ResponseChannels,
    TerminalChannels,
  },
  state::periphery_connections,
};

pub mod terminal;

#[derive(Debug)]
pub struct PeripheryClient {
  pub id: String,
  pub responses: Arc<ResponseChannels>,
  pub terminals: Arc<TerminalChannels>,
}

impl PeripheryClient {
  pub async fn new(
    args: PeripheryConnectionArgs<'_>,
    insecure_tls: bool,
  ) -> anyhow::Result<PeripheryClient> {
    let connections = periphery_connections();

    let id = args.id.to_string();

    // Spawn client side connection if one doesn't exist.
    let Some(connection) = connections.get(&id).await else {
      if args.address.is_none() {
        return Err(anyhow!("Server {id} is not connected"));
      }
      return args
        .spawn_client_connection(id.clone(), insecure_tls)
        .await;
    };

    // Ensure the connection args are unchanged.
    if args.matches(&connection.args) {
      return Ok(PeripheryClient {
        id,
        responses: connection.responses.clone(),
        terminals: connection.terminals.clone(),
      });
    }

    // The args have changed.
    if args.address.is_none() {
      // Periphery -> Core connection
      // Remove this connection, wait and see if client reconnects
      connections.remove(&id).await;
      tokio::time::sleep(Duration::from_millis(500)).await;
      let connection = connections
        .get(&id)
        .await
        .with_context(|| format!("Server {id} is not connected"))?;
      Ok(PeripheryClient {
        id,
        responses: connection.responses.clone(),
        terminals: connection.terminals.clone(),
      })
    } else {
      // Core -> Periphery connection
      args.spawn_client_connection(id.clone(), insecure_tls).await
    }
  }

  pub async fn cleanup(self) -> Option<Arc<PeripheryConnection>> {
    periphery_connections().remove(&self.id).await
  }

  #[tracing::instrument(level = "debug", skip(self))]
  pub async fn health_check(&self) -> anyhow::Result<()> {
    self.request(api::GetHealth {}).await?;
    Ok(())
  }

  #[tracing::instrument(
    name = "PeripheryRequest",
    skip(self),
    level = "debug"
  )]
  pub async fn request<T>(
    &self,
    request: T,
  ) -> anyhow::Result<T::Response>
  where
    T: std::fmt::Debug + Serialize + HasResponse,
    T::Response: DeserializeOwned,
  {
    let connection =
      periphery_connections().get(&self.id).await.with_context(
        || format!("No connection found for server {}", self.id),
      )?;

    // Polls connected 3 times before bailing
    connection.bail_if_not_connected().await?;

    let channel_id = Uuid::new_v4();
    let (response_sender, mut response_receiever) = channel();
    self.responses.insert(channel_id, response_sender).await;

    let req_type = T::req_type();
    let data = JsonMessage(&json!({
      "type": req_type,
      "params": request
    }))
    .encode()?;

    if let Err(e) = connection
      .send(Message::Request(
        WithChannel {
          channel: channel_id,
          data,
        }
        .encode(),
      ))
      .await
      .context("Failed to send request over channel")
    {
      // cleanup
      self.terminals.remove(&channel_id).await;
      return Err(e);
    }

    // Poll for the associated response
    loop {
      let message = response_receiever
        .recv()
        .with_timeout(Duration::from_secs(10))
        .await?;

      // Still in progress, sent to avoid timeout.
      let Some(message) = message.decode()? else {
        continue;
      };

      return message.decode_into();
    }
  }
}
