use anyhow::{Context as _, anyhow};
use derive_variants::{EnumVariants, ExtractVariant as _};

use crate::message::{
  CastBytes, Decode, Encode,
  json::JsonMessageBytes,
  login::LoginMessageBytes,
  wrappers::{ChannelWrapper, OptionWrapper, ResultWrapper},
};

#[derive(Debug, Clone)]
pub struct MessageBytes(Vec<u8>);

impl CastBytes for MessageBytes {
  fn from_vec(vec: Vec<u8>) -> Self {
    Self(vec)
  }
  fn into_vec(self) -> Vec<u8> {
    self.0
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
  Terminal(ChannelWrapper<Vec<u8>>),
}

impl<T: Into<Message> + Send> Encode<MessageBytes> for T {
  fn encode(self) -> MessageBytes {
    let message = self.into();
    let variant_byte = message.extract_variant().as_byte();
    let mut bytes = match message {
      Message::Login(data) => data.into_vec(),
      Message::Request(data) => data.into_vec(),
      Message::Response(data) => data.into_vec(),
      Message::Terminal(data) => data.into_vec(),
    };
    bytes.push(variant_byte);
    MessageBytes(bytes)
  }
}

impl<T: From<Message>> Decode<T> for MessageBytes {
  fn decode(self) -> anyhow::Result<T> {
    let mut bytes = self.0;
    let variant_byte = bytes
      .pop()
      .context("Failed to decode message | bytes are empty")?;
    use MessageVariant::*;
    let message = match MessageVariant::from_byte(variant_byte)? {
      Login => Message::Login(ResultWrapper::from_vec(bytes)),
      Request => Message::Request(ChannelWrapper::from_vec(bytes)),
      Response => Message::Response(ChannelWrapper::from_vec(bytes)),
      Terminal => Message::Terminal(ChannelWrapper::from_vec(bytes)),
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
