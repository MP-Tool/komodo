use anyhow::Context;
use bytes::Bytes;
use serde::{Serialize, de::DeserializeOwned};

use crate::message::{CastBytes, Decode, Encode};

/// ```markdown
/// | --- u8[] --- |
/// | <JSON BYTES> |
/// ```
#[derive(Clone, Debug)]
pub struct JsonMessage(Bytes);

impl CastBytes for JsonMessage {
  fn from_bytes(bytes: Bytes) -> Self {
    Self(bytes)
  }
  fn into_bytes(self) -> Bytes {
    self.0
  }
}

impl<T: Serialize> Encode<JsonMessage> for &T {
  fn encode(self) -> anyhow::Result<JsonMessage> {
    let bytes = serde_json::to_vec(self)
      .context("Failed to serialize data to bytes")?;
    Ok(JsonMessage(bytes.into()))
  }
}

impl<T: DeserializeOwned> Decode<T> for JsonMessage {
  fn decode(self) -> anyhow::Result<T> {
    serde_json::from_slice(&self.0)
      .context("Failed to parse JSON bytes")
  }
}
