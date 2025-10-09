use anyhow::{Context, anyhow};
use bytes::Bytes;
use derive_variants::{EnumVariants, ExtractVariant};

use crate::message::{
  json::JsonMessageBytes,
  login::LoginMessageBytes,
  wrappers::{ChannelWrapper, OptionWrapper, ResultWrapper},
};

pub mod json;
pub mod login;
pub mod wrappers;

pub trait Encode<Target>: Sized + Send {
  fn encode(self) -> Target;
  fn encode_into<T>(self) -> T
  where
    Target: Encode<T>,
  {
    self.encode().encode()
  }
}

impl Decode<Bytes> for Bytes {
  fn decode(self) -> anyhow::Result<Bytes> {
    Ok(self)
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

impl Encode<Bytes> for Bytes {
  fn encode(self) -> Bytes {
    self
  }
}

/// Helps cast between the top level message types.
pub trait CastBytes: Sized {
  fn from_bytes(bytes: Bytes) -> Self;
  fn into_bytes(self) -> Bytes;
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

#[derive(Debug, Clone)]
pub struct MessageBytes(Bytes);

impl CastBytes for MessageBytes {
  fn from_bytes(bytes: Bytes) -> Self {
    Self(bytes.into())
  }
  fn into_bytes(self) -> Bytes {
    self.0.into()
  }
}

#[derive(Debug, EnumVariants)]
#[variant_derive(Debug, Clone, Copy)]
pub enum Message {
  Login(ResultWrapper<LoginMessageBytes>),
  Request(ChannelWrapper<JsonMessageBytes>),
  Response(
    ChannelWrapper<OptionWrapper<ResultWrapper<JsonMessageBytes>>>,
  ),
  Terminal(ChannelWrapper<Bytes>),
}

impl<T: Into<Message> + Send> Encode<MessageBytes> for T {
  fn encode(self) -> MessageBytes {
    let message = self.into();
    let variant_byte = message.extract_variant().as_byte();
    let mut bytes: Vec<u8> = match message {
      Message::Login(data) => data.into_vec(),
      Message::Request(data) => data.into_vec(),
      Message::Response(data) => data.into_vec(),
      Message::Terminal(data) => data.into_vec(),
    }
    .into();
    bytes.push(variant_byte);
    MessageBytes(bytes.into())
  }
}

impl<T: From<Message>> Decode<T> for MessageBytes {
  fn decode(self) -> anyhow::Result<T> {
    let mut bytes: Vec<u8> = self.0.into();
    let variant_byte = bytes
      .pop()
      .context("Failed to decode message | bytes are empty")?;
    use MessageVariant::*;
    let message = match MessageVariant::from_byte(variant_byte)? {
      Login => Message::Login(ResultWrapper::from_vec(bytes.into())),
      Request => {
        Message::Request(ChannelWrapper::from_vec(bytes.into()))
      }
      Response => {
        Message::Response(ChannelWrapper::from_vec(bytes.into()))
      }
      Terminal => {
        Message::Terminal(ChannelWrapper::from_vec(bytes.into()))
      }
    };
    Ok(message.into())
  }
}

impl MessageVariant {
  pub fn from_byte(byte: u8) -> anyhow::Result<Self> {
    use MessageVariant::*;
    let variant = match byte {
      0 => Login,
      1 => Request,
      2 => Response,
      3 => Terminal,
      other => {
        return Err(anyhow!(
          "Got unrecognized MessageVariant byte: {other}"
        ));
      }
    };
    Ok(variant)
  }

  pub fn as_byte(self) -> u8 {
    use MessageVariant::*;
    match self {
      Login => 0,
      Request => 1,
      Response => 2,
      Terminal => 3,
    }
  }
}
