use anyhow::Context;
use bytes::Bytes;
use serror::{deserialize_error_bytes, serialize_error_bytes};

use crate::{CastBytes, Decode, Encode};

/// Message wrapper to handle Error unwrapping
/// anywhere in the en/decoding chain.
/// ```markdown
/// | -- u8[] -- | ----- u8 ------ |
/// | <CONTENTS> | 0: Ok or _: Err |
/// ```
#[derive(Clone, Debug)]
pub struct EncodedResult<T>(T);

impl<T> From<T> for EncodedResult<T> {
  fn from(value: T) -> Self {
    Self(value)
  }
}

impl<T: CastBytes> CastBytes for EncodedResult<T> {
  fn from_bytes(bytes: Bytes) -> Self {
    Self(T::from_bytes(bytes))
  }
  fn into_bytes(self) -> Bytes {
    self.0.into_bytes()
  }
  fn from_vec(vec: Vec<u8>) -> Self {
    Self(T::from_vec(vec))
  }
  fn into_vec(self) -> Vec<u8> {
    self.0.into_vec()
  }
}

impl<T: CastBytes + Send> Encode<EncodedResult<T>>
  for anyhow::Result<T>
{
  fn encode(self) -> EncodedResult<T> {
    let bytes = match self {
      Ok(data) => {
        let mut bytes = data.into_vec();
        bytes.push(0);
        bytes
      }
      Err(e) => {
        let mut bytes = serialize_error_bytes(&e);
        bytes.push(1);
        bytes
      }
    };
    EncodedResult(T::from_vec(bytes))
  }
}

impl<T: CastBytes + Send> Encode<EncodedResult<T>>
  for &anyhow::Error
{
  fn encode(self) -> EncodedResult<T> {
    let mut bytes = serialize_error_bytes(self);
    bytes.push(1);
    EncodedResult::from_vec(bytes)
  }
}

impl<T: CastBytes> Decode<T> for EncodedResult<T> {
  fn decode(self) -> anyhow::Result<T> {
    let mut bytes = self.0.into_vec();
    let result_byte =
      bytes.pop().context("ResultWrapper bytes cannot be empty")?;
    if result_byte == 0 {
      Ok(T::from_vec(bytes))
    } else {
      Err(deserialize_error_bytes(&bytes))
    }
  }
}
