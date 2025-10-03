use std::{sync::Arc, time::Duration};

use anyhow::{Context, anyhow};
use bytes::Bytes;
use cache::CloneCache;
use periphery_client::api;
use resolver_api::HasResponse;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use serror::deserialize_error_bytes;
use tokio::sync::mpsc::{self, Sender};
use tracing::warn;
use transport::{
  MessageState,
  bytes::{from_transport_bytes, to_transport_bytes},
};
use uuid::Uuid;

use crate::{
  connection::{PeripheryConnection, PeripheryConnectionArgs},
  state::periphery_connections,
};

pub mod terminal;

pub type ConnectionChannels = CloneCache<Uuid, Sender<Bytes>>;

#[derive(Debug)]
pub struct PeripheryClient {
  pub id: String,
  channels: Arc<ConnectionChannels>,
}

impl PeripheryClient {
  pub async fn new(
    args: PeripheryConnectionArgs<'_>,
    // deprecated.
    passkey: &str,
  ) -> anyhow::Result<PeripheryClient> {
    let connections = periphery_connections();

    let id = args.id.to_string();

    // Spawn client side connection if one doesn't exist.
    let Some(connection) = connections.get(&id).await else {
      if args.address.is_none() {
        return Err(anyhow!("Server {id} is not connected"));
      }
      let channels = args
        .spawn_client_connection(id.clone(), passkey.to_string())
        .await?;
      return Ok(PeripheryClient { id, channels });
    };

    // Ensure the connection args are unchanged.
    if args.matches(&connection.args) {
      return Ok(PeripheryClient {
        id,
        channels: connection.channels.clone(),
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
        channels: connection.channels.clone(),
      })
    } else {
      // Core -> Periphery connection
      let channels = args
        .spawn_client_connection(id.clone(), passkey.to_string())
        .await?;
      Ok(PeripheryClient { id, channels })
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

    let id = Uuid::new_v4();
    let (response_sender, mut response_receiever) =
      mpsc::channel(1000);
    self.channels.insert(id, response_sender).await;

    let req_type = T::req_type();
    let data = serde_json::to_vec(&json!({
      "type": req_type,
      "params": request
    }))
    .context("Failed to serialize request to bytes")?;

    if let Err(e) = connection
      .send(to_transport_bytes(data, id, MessageState::Request))
      .await
      .context("Failed to send request over channel")
    {
      // cleanup
      self.channels.remove(&id).await;
      return Err(e);
    }

    // Poll for the associated response
    loop {
      let next = tokio::select! {
        msg = response_receiever.recv() => msg,
        // Periphery will send InProgress every 5s to avoid timeout
        _ = tokio::time::sleep(Duration::from_secs(10)) => {
          return Err(anyhow!("Response timed out"));
        }
      };

      let bytes = match next {
        Some(bytes) => bytes,
        None => {
          return Err(anyhow!(
            "Sender dropped before response was recieved"
          ));
        }
      };

      let (state, data) = match from_transport_bytes(bytes) {
        Ok((data, _, state)) if !data.is_empty() => (state, data),
        // Ignore no data cases
        Ok(_) => continue,
        Err(e) => {
          warn!(
            "Server {} | Received invalid message | {e:#}",
            self.id
          );
          continue;
        }
      };
      match state {
        // TODO: improve the allocation in .to_vec
        MessageState::Successful => {
          // cleanup
          self.channels.remove(&id).await;
          return serde_json::from_slice(&data)
            .context("Failed to parse successful response");
        }
        MessageState::Failed => {
          // cleanup
          self.channels.remove(&id).await;
          return Err(deserialize_error_bytes(&data));
        }
        MessageState::InProgress => continue,
        // Shouldn't be received by this receiver
        other => {
          // TODO: delete log
          warn!(
            "Server {} | Got other message over over response channel: {other:?}",
            self.id
          );
          continue;
        }
      }
    }
  }
}
