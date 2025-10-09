use anyhow::anyhow;
use bytes::Bytes;
use uuid::Uuid;

use crate::message::{CastBytes, Decode, Encode};

/// Message wrapper to handle Error unwrapping
/// anywhere in the en/decoding chain.
/// ```markdown
/// | -- u8[] -- | -- [u8; 16] -- |
/// | <CONTENTS> |  Channel Uuid  |
/// ```
#[derive(Clone, Debug)]
pub struct ChannelWrapper<T>(T);

impl<T> From<T> for ChannelWrapper<T> {
  fn from(value: T) -> Self {
    Self(value)
  }
}

impl<T: CastBytes> CastBytes for ChannelWrapper<T> {
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

pub struct WithChannel<T> {
  pub channel: Uuid,
  pub data: T,
}

impl<T: CastBytes + Send> Encode<ChannelWrapper<T>>
  for WithChannel<T>
{
  fn encode(self) -> ChannelWrapper<T> {
    let mut bytes = self.data.into_vec();
    bytes.extend(self.channel.into_bytes());
    ChannelWrapper(T::from_vec(bytes))
  }
}

impl<T: CastBytes> Decode<WithChannel<T>> for ChannelWrapper<T> {
  fn decode(self) -> anyhow::Result<WithChannel<T>> {
    let mut bytes = self.0.into_vec();
    let len = bytes.len();
    if bytes.len() < 16 {
      return Err(anyhow!(
        "ChannelMessage bytes too short to include uuid"
      ));
    }
    let mut channel = [0u8; 16];
    for (i, byte) in bytes.drain(len - 16..).enumerate() {
      channel[i] = byte;
    }
    Ok(WithChannel {
      channel: Uuid::from_bytes(channel),
      data: T::from_vec(bytes),
    })
  }
}
