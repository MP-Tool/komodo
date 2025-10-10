use anyhow::Context;
use serde::{Serialize, de::DeserializeOwned};

use crate::message::{
  CastBytes, Decode, Encode, wrappers::EncodedResult,
};

/// ```markdown
/// | --- u8[] --- |
/// | <JSON BYTES> |
/// ```
#[derive(Clone, Debug)]
pub struct EncodedJsonMessage(Vec<u8>);

impl CastBytes for EncodedJsonMessage {
  fn from_vec(vec: Vec<u8>) -> Self {
    Self(vec)
  }
  fn into_vec(self) -> Vec<u8> {
    self.0
  }
}

pub struct JsonMessage<'a, T>(pub &'a T);

impl<'a, T: Serialize + Send>
  Encode<anyhow::Result<EncodedJsonMessage>> for JsonMessage<'a, T>
where
  &'a T: Send,
{
  fn encode(self) -> anyhow::Result<EncodedJsonMessage> {
    let bytes = serde_json::to_vec(self.0)
      .context("Failed to serialize data to bytes")?;
    Ok(EncodedJsonMessage(bytes))
  }
}

impl<T: DeserializeOwned> Decode<T> for EncodedJsonMessage {
  fn decode(self) -> anyhow::Result<T> {
    serde_json::from_slice(&self.0)
      .context("Failed to parse JSON bytes")
  }
}

impl<T: Serialize + Send> From<T>
  for EncodedResult<EncodedJsonMessage>
{
  fn from(value: T) -> Self {
    serde_json::to_vec(&value)
      .map(EncodedJsonMessage::from_vec)
      .context("Failed to serialize data to bytes")
      .encode()
  }
}
