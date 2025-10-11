use std::option::Option as StdOption;

use anyhow::Context;
use bytes::Bytes;

use crate::{CastBytes, Decode, Encode, impl_wrapper};

/// Message wrapper to handle Option unwrapping
/// anywhere in the en/decoding chain.
/// ```markdown
/// | -- u8[] -- | ------- u8 ------- |
/// | <CONTENTS> | 0: Some or _: None |
/// ```
#[derive(Clone, Debug)]
pub struct EncodedOption<T>(T);

impl_wrapper!(EncodedOption);

impl<B: CastBytes + Send> EncodedOption<B> {
  /// Will only produce None if really None, with confirmed None byte.
  /// Any error in decoding will lead to Some(Err(e))
  pub fn decode_map<T>(self) -> StdOption<anyhow::Result<T>>
  where
    B: Decode<T>,
  {
    self.decode().and_then(|data| data.map_decode()).transpose()
  }
}

/// Just std Option,
/// but can implement on it.
pub enum Option<T> {
  Some(T),
  None,
}

impl<T> Option<T> {
  pub fn into_std(self) -> StdOption<T> {
    self.into()
  }

  pub fn map<R>(self, map: impl FnOnce(T) -> R) -> Option<R> {
    use Option::*;
    match self {
      Some(t) => Some(map(t)),
      None => None,
    }
  }

  pub fn map_encode<B: CastBytes + Send>(self) -> EncodedOption<B>
  where
    T: Encode<B>,
  {
    self.map(Encode::encode).encode()
  }

  pub fn map_decode<D>(self) -> anyhow::Result<StdOption<D>>
  where
    T: CastBytes + Send + Decode<D>,
  {
    match self.map(Decode::decode) {
      Option::Some(res) => res.map(Some),
      Option::None => Ok(None),
    }
  }
}

impl<T: CastBytes + Send> Encode<EncodedOption<T>> for Option<T> {
  fn encode(self) -> EncodedOption<T> {
    use Option::*;
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

impl<T: CastBytes + Send> Encode<EncodedOption<T>> for StdOption<T> {
  fn encode(self) -> EncodedOption<T> {
    Option::from(self).encode()
  }
}

impl<T: CastBytes> Decode<Option<T>> for EncodedOption<T> {
  fn decode(self) -> anyhow::Result<Option<T>> {
    use Option::*;
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

impl<T> From<StdOption<T>> for Option<T> {
  fn from(value: StdOption<T>) -> Self {
    match value {
      Some(t) => Self::Some(t),
      None => Self::None,
    }
  }
}

impl<T> From<Option<T>> for StdOption<T> {
  fn from(value: Option<T>) -> Self {
    match value {
      Option::Some(t) => Some(t),
      Option::None => None,
    }
  }
}
