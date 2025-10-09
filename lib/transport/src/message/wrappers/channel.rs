use anyhow::anyhow;
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

impl<T: CastBytes> CastBytes for ChannelWrapper<T> {
  fn from_bytes(bytes: bytes::Bytes) -> Self {
    Self(T::from_bytes(bytes))
  }
  fn into_bytes(self) -> bytes::Bytes {
    self.0.into_bytes()
  }
}

impl<T: CastBytes> Encode<ChannelWrapper<T>> for (Uuid, T) {
  fn encode(self) -> anyhow::Result<ChannelWrapper<T>> {
    let (channel, t) = self;
    let mut bytes: Vec<u8> = t.into_bytes().into();
    bytes.extend(channel.into_bytes());
    Ok(ChannelWrapper(T::from_bytes(bytes.into())))
  }
}

impl<T: CastBytes> Decode<(Uuid, T)> for ChannelWrapper<T> {
  fn decode(self) -> anyhow::Result<(Uuid, T)> {
    let mut bytes: Vec<u8> = self.0.into_bytes().into();
    let len = bytes.len();
    if bytes.len() < 16 {
      return Err(anyhow!(
        "ChannelMessage bytes too short to include uuid"
      ));
    }
    let mut buf = [0u8; 16];
    for (i, byte) in bytes.drain(len - 16..).enumerate() {
      buf[i] = byte;
    }
    let uuid = Uuid::from_bytes(buf);
    Ok((uuid, T::from_bytes(bytes.into())))
  }
}
