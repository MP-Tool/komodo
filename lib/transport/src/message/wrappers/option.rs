use anyhow::Context;
use bytes::Bytes;

use crate::message::{CastBytes, Decode, Encode};

/// Message wrapper to handle Error unwrapping
/// anywhere in the en/decoding chain.
/// ```markdown
/// | -- u8[] -- | ----- u8 ------ |
/// | <CONTENTS> | 0: Ok or _: Err |
/// ```
#[derive(Clone, Debug)]
pub struct EncodedOption<T>(T);

impl<T> From<T> for EncodedOption<T> {
  fn from(value: T) -> Self {
    Self(value)
  }
}

impl<T: CastBytes> CastBytes for EncodedOption<T> {
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

impl<T: CastBytes + Send> Encode<EncodedOption<T>> for Option<T> {
  fn encode(self) -> EncodedOption<T> {
    match self {
      Some(data) => {
        let mut bytes = data.into_vec();
        bytes.push(0);
        EncodedOption(T::from_vec(bytes))
      }
      None => EncodedOption(T::from_vec(vec![1])),
    }
  }
}

impl<T: CastBytes> Decode<Option<T>> for EncodedOption<T> {
  fn decode(self) -> anyhow::Result<Option<T>> {
    let mut bytes = self.0.into_vec();
    let option_byte =
      bytes.pop().context("OptionWrapper bytes cannot be empty")?;
    if option_byte == 0 {
      Ok(Some(T::from_vec(bytes)))
    } else {
      Ok(None)
    }
  }
}
