use anyhow::Context;
use bytes::Bytes;
use serde::{Serialize, de::DeserializeOwned};

use crate::message::{
  CastBytes, Decode, Encode, wrappers::ResultWrapper,
};

/// ```markdown
/// | --- u8[] --- |
/// | <JSON BYTES> |
/// ```
#[derive(Clone, Debug)]
pub struct JsonMessageBytes(Bytes);

impl CastBytes for JsonMessageBytes {
  fn from_bytes(bytes: Bytes) -> Self {
    Self(bytes.into())
  }
  fn into_bytes(self) -> Bytes {
    self.0.into()
  }
}

pub struct JsonMessage<'a, T>(pub &'a T);

impl<'a, T: Serialize + Send> Encode<anyhow::Result<JsonMessageBytes>>
  for JsonMessage<'a, T>
where
  &'a T: Send,
{
  fn encode(self) -> anyhow::Result<JsonMessageBytes> {
    let bytes = serde_json::to_vec(self.0)
      .context("Failed to serialize data to bytes")?;
    Ok(JsonMessageBytes(bytes.into()))
  }
}

impl<T: DeserializeOwned> Decode<T> for JsonMessageBytes {
  fn decode(self) -> anyhow::Result<T> {
    serde_json::from_slice(&self.0)
      .context("Failed to parse JSON bytes")
  }
}

impl<T: Serialize + Send> From<T>
  for ResultWrapper<JsonMessageBytes>
{
  fn from(value: T) -> Self {
    serde_json::to_vec(&value)
      .map(JsonMessageBytes::from_vec)
      .context("Failed to serialize data to bytes")
      .encode()
  }
}
