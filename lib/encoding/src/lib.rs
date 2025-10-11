//! Utilities for type-safe byte encoding and decoding.

use bytes::Bytes;

mod channel;
mod json;
mod option;
mod result;

pub use channel::*;
pub use json::*;
pub use option::*;
pub use result::*;

pub trait Encode<Target>: Sized + Send {
  fn encode(self) -> Target;
  fn encode_into<T>(self) -> T
  where
    Target: Encode<T>,
  {
    self.encode().encode()
  }
}

pub trait Decode<Target>: Sized {
  fn decode(self) -> anyhow::Result<Target>;
  fn decode_into<T>(self) -> anyhow::Result<T>
  where
    Target: Decode<T>,
  {
    self.decode()?.decode()
  }
}

impl_identity!(Bytes);
impl_identity!(Vec<u8>);

impl_identity_into!(Bytes, Vec<u8>);
impl_identity_into!(Vec<u8>, Bytes);

/// Helps cast between the top level message types.
/// Implement whichever ones are most convenient for the source type.
pub trait CastBytes: Sized {
  fn from_bytes(bytes: Bytes) -> Self {
    Self::from_vec(bytes.into())
  }
  fn into_bytes(self) -> Bytes {
    self.into_vec().into()
  }
  fn from_vec(bytes: Vec<u8>) -> Self {
    Self::from_bytes(bytes.into())
  }
  fn into_vec(self) -> Vec<u8> {
    self.into_bytes().into()
  }
}

impl CastBytes for Bytes {
  fn from_bytes(bytes: Bytes) -> Self {
    bytes
  }
  fn into_bytes(self) -> Bytes {
    self
  }
}

impl CastBytes for Vec<u8> {
  fn from_vec(vec: Vec<u8>) -> Self {
    vec
  }
  fn into_vec(self) -> Vec<u8> {
    self
  }
}

// Encode is basically 'Into',
// while decode is 'TryInto'.
#[macro_export]
macro_rules! impl_identity {
  ($typ:ty) => {
    impl Encode<$typ> for $typ {
      fn encode(self) -> $typ {
        self
      }
    }
    impl Decode<$typ> for $typ {
      fn decode(self) -> anyhow::Result<$typ> {
        Ok(self)
      }
    }
  };
}

/// impl where a: Into<b>
#[macro_export]
macro_rules! impl_identity_into {
  ($a:ty, $b:ty) => {
    impl Encode<$b> for $a {
      fn encode(self) -> $b {
        self.into()
      }
    }
    impl Decode<$b> for $a {
      fn decode(self) -> anyhow::Result<$b> {
        Ok(self.into())
      }
    }
  };
}

#[macro_export]
macro_rules! impl_wrapper {
  ($struct:ident) => {
    impl<T> From<T> for $struct<T> {
      fn from(value: T) -> Self {
        Self(value)
      }
    }

    impl<T: CastBytes> CastBytes for $struct<T> {
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
  };
}
