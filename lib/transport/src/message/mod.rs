use anyhow::{Context, anyhow};
use axum::extract::ws::Utf8Bytes as AxumUtf8Bytes;
use bytes::Bytes;
use derive_variants::EnumVariants;
use serror::serialize_error_bytes;
use tokio_tungstenite::tungstenite::Utf8Bytes as TungsteniteUtf8Bytes;
use uuid::Uuid;

use crate::message::{
  json::JsonMessage,
  login::LoginMessage,
  wrappers::{ChannelWrapper, ResultWrapper},
};

pub mod json;
pub mod login;
pub mod wrappers;

pub trait Encode<Target>: Sized {
  fn encode(self) -> anyhow::Result<Target>;
  fn encode_into<T>(self) -> anyhow::Result<T>
  where
    Target: Encode<T>,
  {
    self.encode()?.encode()
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

/// Helps cast between the top level message types.
pub trait CastBytes {
  fn from_bytes(bytes: Bytes) -> Self;
  fn into_bytes(self) -> Bytes;
}

impl CastBytes for Bytes {
  fn from_bytes(bytes: Bytes) -> Self {
    bytes
  }
  fn into_bytes(self) -> Bytes {
    self
  }
}

#[derive(Default, Clone, Debug)]
pub struct Message(Bytes);

impl CastBytes for Message {
  fn from_bytes(bytes: Bytes) -> Self {
    Self(bytes)
  }
  fn into_bytes(self) -> Bytes {
    self.0
  }
}

impl Decode<ParsedMessage> for Message {
  fn decode(self) -> anyhow::Result<ParsedMessage> {
    let mut bytes: Vec<u8> = self.0.into();
    let variant_byte = bytes
      .pop()
      .context("Failed to parse message | bytes are empty")?;
    let variant = ParsedMessageVariant::from_byte(variant_byte)?;
    use ParsedMessageVariant::*;
    match variant {
      Login => Ok(ParsedMessage::Login(ResultWrapper::from_bytes(
        bytes.into(),
      ))),
      Request => Ok(ParsedMessage::Request(
        ChannelWrapper::from_bytes(bytes.into()),
      )),
      Response => Ok(ParsedMessage::Response(
        ChannelWrapper::from_bytes(bytes.into()),
      )),
      Terminal => Ok(ParsedMessage::Terminal(
        ChannelWrapper::from_bytes(bytes.into()),
      )),
    }
  }
}

#[derive(EnumVariants)]
#[variant_derive(Debug, Clone, Copy)]
pub enum ParsedMessage {
  Login(ResultWrapper<LoginMessage>),
  Request(ChannelWrapper<JsonMessage>),
  Response(ChannelWrapper<ResultWrapper<JsonMessage>>),
  Terminal(ChannelWrapper<ResultWrapper<Bytes>>),
}

impl ParsedMessageVariant {
  pub fn from_byte(byte: u8) -> anyhow::Result<Self> {
    use ParsedMessageVariant::*;
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
    use ParsedMessageVariant::*;
    match self {
      Login => 0,
      Request => 1,
      Response => 2,
      Terminal => 3,
    }
  }
}

impl Message {
  pub fn from_bytes(bytes: Bytes) -> Self {
    Self(bytes)
  }

  pub fn from_axum_utf8(utf8: AxumUtf8Bytes) -> Self {
    Self(utf8.into())
  }

  pub fn from_tungstenite_utf8(utf8: TungsteniteUtf8Bytes) -> Self {
    Self(utf8.into())
  }

  pub fn into_inner(self) -> Bytes {
    self.0
  }

  pub fn is_empty(&self) -> bool {
    self.0.is_empty()
  }

  pub fn state(&self) -> anyhow::Result<MessageState> {
    self
      .0
      .last()
      .map(|byte| MessageState::from_byte(*byte))
      .context("Message is empty and has no state")
  }

  pub fn channel(&self) -> anyhow::Result<Uuid> {
    let len = self.0.len();
    if len < 17 {
      return Err(anyhow!(
        "Transport bytes too short to include uuid + state"
      ));
    }
    Uuid::from_slice(&self.0[(len - 17)..(len - 1)])
      .context("Invalid channel bytes")
  }

  pub fn channel_and_state(
    &self,
  ) -> anyhow::Result<(Uuid, MessageState)> {
    let len = self.0.len();
    if len < 17 {
      return Err(anyhow!(
        "Transport bytes too short to include uuid + state"
      ));
    }
    let uuid = Uuid::from_slice(&self.0[(len - 17)..(len - 1)])
      .context("Invalid Uuid bytes")?;
    let state = MessageState::from_byte(self.0[len - 1]);
    Ok((uuid, state))
  }

  pub fn data(&self) -> anyhow::Result<&[u8]> {
    // Does not work with .borrow() due to lifetime issue
    let len = self.0.len();
    if len < 17 {
      return Err(anyhow!(
        "Transport bytes too short to include uuid + state + data"
      ));
    }
    Ok(&self.0[..(len - 17)])
  }

  pub fn into_data(self) -> anyhow::Result<Bytes> {
    let len = self.0.len();
    if len < 17 {
      return Err(anyhow!(
        "Transport bytes too short to include uuid + state + data"
      ));
    }
    let mut res: Vec<u8> = self.0.into();
    res.drain((len - 17)..);
    Ok(res.into())
  }

  pub fn into_parts(
    self,
  ) -> anyhow::Result<(Bytes, Uuid, MessageState)> {
    let (channel, state) = self.channel_and_state()?;
    let data = self.into_data()?;
    Ok((data, channel, state))
  }
}

#[derive(Debug, Clone, Copy)]
pub enum MessageState {
  Successful = 0,
  Failed = 1,
  Terminal = 2,
  Request = 3,
  InProgress = 4,
}

impl MessageState {
  pub fn from_byte(byte: u8) -> MessageState {
    match byte {
      0 => MessageState::Successful,
      1 => MessageState::Failed,
      2 => MessageState::Terminal,
      3 => MessageState::Request,
      _ => MessageState::InProgress,
    }
  }

  pub fn as_byte(&self) -> u8 {
    match self {
      MessageState::Successful => 0,
      MessageState::Failed => 1,
      MessageState::Terminal => 2,
      MessageState::Request => 3,
      MessageState::InProgress => 4,
    }
  }
}

impl<T> From<(T, Uuid, MessageState)> for Message
where
  T: Into<Vec<u8>>,
{
  fn from((data, channel, state): (T, Uuid, MessageState)) -> Self {
    let mut data = data.into();
    data.extend(channel.into_bytes());
    data.push(state.as_byte());
    Self(data.into())
  }
}

impl From<(Vec<u8>, Uuid)> for Message {
  fn from((data, channel): (Vec<u8>, Uuid)) -> Self {
    (data, channel, MessageState::Successful).into()
  }
}

impl From<(Vec<u8>, MessageState)> for Message {
  fn from((data, state): (Vec<u8>, MessageState)) -> Self {
    (data, Uuid::nil(), state).into()
  }
}

impl From<(Uuid, MessageState)> for Message {
  fn from((channel, state): (Uuid, MessageState)) -> Self {
    let mut res = channel.into_bytes().to_vec();
    res.push(state.as_byte());
    Self(res.into())
  }
}

impl From<(&anyhow::Error, Uuid)> for Message {
  fn from((e, channel): (&anyhow::Error, Uuid)) -> Self {
    (serialize_error_bytes(e), channel, MessageState::Failed).into()
  }
}

impl From<Vec<u8>> for Message {
  fn from(data: Vec<u8>) -> Self {
    (data, Uuid::nil(), MessageState::Successful).into()
  }
}

impl From<&[u8]> for Message {
  fn from(data: &[u8]) -> Self {
    Vec::from(data).into()
  }
}

impl<const N: usize> From<[u8; N]> for Message {
  fn from(data: [u8; N]) -> Self {
    Vec::from(data).into()
  }
}

impl From<Uuid> for Message {
  fn from(value: Uuid) -> Self {
    (value, MessageState::Successful).into()
  }
}

impl From<MessageState> for Message {
  fn from(value: MessageState) -> Self {
    let mut res = [0u8; 17];
    res[16] = value.as_byte();
    Self(Bytes::from_owner(res))
  }
}

impl From<&anyhow::Error> for Message {
  fn from(value: &anyhow::Error) -> Self {
    (value, Uuid::nil()).into()
  }
}
