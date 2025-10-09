use anyhow::Context;
use bytes::Bytes;
use serror::{deserialize_error_bytes, serialize_error_bytes};

use crate::message::{CastBytes, Decode, Encode};

/// Message wrapper to handle Error unwrapping
/// anywhere in the en/decoding chain.
/// ```markdown
/// | -- u8[] -- | ----- u8 ------ |
/// | <CONTENTS> | 0: Ok or _: Err |
/// ```
#[derive(Clone, Debug)]
pub struct ResultWrapper<T>(T);

impl<T> From<T> for ResultWrapper<T> {
  fn from(value: T) -> Self {
    Self(value)
  }
}

impl<T: CastBytes> CastBytes for ResultWrapper<T> {
  fn from_bytes(bytes: Bytes) -> Self {
    Self(T::from_bytes(bytes))
  }
  fn into_bytes(self) -> Bytes {
    self.0.into_bytes()
  }
}

impl<T: CastBytes + Send> Encode<ResultWrapper<T>>
  for anyhow::Result<T>
{
  fn encode(self) -> ResultWrapper<T> {
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
    ResultWrapper(T::from_vec(bytes))
  }
}

impl<T: CastBytes + Send> Encode<ResultWrapper<T>>
  for &anyhow::Error
{
  fn encode(self) -> ResultWrapper<T> {
    let mut bytes = serialize_error_bytes(self);
    bytes.push(1);
    ResultWrapper::from_vec(bytes)
  }
}

impl<T: CastBytes> Decode<T> for ResultWrapper<T> {
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
