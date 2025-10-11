use anyhow::{Context, Result as AnyhowResult};
use bytes::Bytes;
use serror::{deserialize_error_bytes, serialize_error_bytes};

use crate::{CastBytes, Decode, Encode, impl_wrapper};

/// Message wrapper to handle Error unwrapping
/// anywhere in the en/decoding chain.
/// ```markdown
/// | -- u8[] -- | ----- u8 ------ |
/// | <CONTENTS> | 0: Ok or _: Err |
/// ```
#[derive(Clone, Debug)]
pub struct EncodedResult<T>(T);

impl_wrapper!(EncodedResult);

/// Just anyhow's Result,
/// but can implement on it.
pub enum Result<T> {
  Ok(T),
  Err(anyhow::Error),
}

impl<T> Result<T> {
  pub fn into_anyhow(self) -> AnyhowResult<T> {
    self.into()
  }

  pub fn map<R>(self, map: impl FnOnce(T) -> R) -> Result<R> {
    use Result::*;
    match self {
      Ok(t) => Ok(map(t)),
      Err(e) => Err(e),
    }
  }

  pub fn map_encode<B: CastBytes + Send>(self) -> EncodedResult<B>
  where
    T: Encode<B>,
  {
    self.map(Encode::encode).encode()
  }

  pub fn map_decode<D>(self) -> anyhow::Result<D>
  where
    T: CastBytes + Send + Decode<D>,
  {
    match self.map(Decode::decode) {
      Result::Ok(res) => res,
      Result::Err(e) => Err(e),
    }
  }
}

impl<T: CastBytes + Send> Encode<EncodedResult<T>> for Result<T> {
  fn encode(self) -> EncodedResult<T> {
    use Result::*;
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
  for AnyhowResult<T>
{
  fn encode(self) -> EncodedResult<T> {
    Result::from(self).encode()
  }
}

impl<T: CastBytes> Encode<EncodedResult<T>> for &anyhow::Error {
  fn encode(self) -> EncodedResult<T> {
    let mut bytes = serialize_error_bytes(self);
    bytes.push(1);
    EncodedResult::from_vec(bytes)
  }
}

impl<T: CastBytes> Decode<T> for EncodedResult<T> {
  fn decode(self) -> AnyhowResult<T> {
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

impl<T> From<AnyhowResult<T>> for Result<T> {
  fn from(value: AnyhowResult<T>) -> Self {
    match value {
      Ok(t) => Self::Ok(t),
      Err(e) => Self::Err(e),
    }
  }
}

impl<T> From<Result<T>> for AnyhowResult<T> {
  fn from(value: Result<T>) -> Self {
    match value {
      Result::Ok(t) => Ok(t),
      Result::Err(e) => Err(e),
    }
  }
}
