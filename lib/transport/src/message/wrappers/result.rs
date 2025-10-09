use anyhow::Context;
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

impl<T: CastBytes> CastBytes for ResultWrapper<T> {
  fn from_bytes(bytes: bytes::Bytes) -> Self {
    Self(T::from_bytes(bytes))
  }
  fn into_bytes(self) -> bytes::Bytes {
    self.0.into_bytes()
  }
}

impl<T: CastBytes> Encode<ResultWrapper<T>> for anyhow::Result<T> {
  fn encode(self) -> anyhow::Result<ResultWrapper<T>> {
    let bytes = match self {
      Ok(t) => {
        let mut bytes: Vec<u8> = t.into_bytes().into();
        bytes.push(0);
        bytes
      }
      Err(e) => {
        let mut bytes = serialize_error_bytes(&e);
        bytes.push(1);
        bytes
      }
    };
    Ok(ResultWrapper(T::from_bytes(bytes.into())))
  }
}

impl<T: CastBytes> Decode<T> for ResultWrapper<T> {
  fn decode(self) -> anyhow::Result<T> {
    let mut bytes: Vec<u8> = self.0.into_bytes().into();
    let result_byte =
      bytes.pop().context("ResultMessage bytes cannot be empty")?;
    if result_byte == 0 {
      Ok(T::from_bytes(bytes.into()))
    } else {
      Err(deserialize_error_bytes(&bytes))
    }
  }
}
