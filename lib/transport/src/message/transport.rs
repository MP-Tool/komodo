use anyhow::{Context as _, anyhow};
use derive_variants::{EnumVariants, ExtractVariant as _};

use crate::message::{
  CastBytes, Decode, Encode,
  json::EncodedJsonMessage,
  login::{InnerEncodedLoginMessage, LoginMessage},
  wrappers::{
    EncodedChannel, EncodedOption, EncodedResult, WithChannel,
  },
};

#[derive(Debug, Clone)]
pub struct EncodedTransportMessage(Vec<u8>);

impl CastBytes for EncodedTransportMessage {
  fn from_vec(vec: Vec<u8>) -> Self {
    Self(vec)
  }
  fn into_vec(self) -> Vec<u8> {
    self.0
  }
}

#[derive(Debug)]
pub struct EncodedLoginMessage(
  pub EncodedResult<InnerEncodedLoginMessage>,
);

#[derive(Debug)]
pub struct EncodedRequestMessage(
  pub EncodedChannel<EncodedJsonMessage>,
);

#[derive(Debug)]
pub struct EncodedResponseMessage(
  pub EncodedChannel<EncodedOption<EncodedResult<EncodedJsonMessage>>>,
);

#[derive(Debug)]
pub struct EncodedTerminalMessage(pub EncodedChannel<Vec<u8>>);

#[derive(Debug, EnumVariants)]
#[variant_derive(Debug, Clone, Copy)]
pub enum TransportMessage {
  Login(EncodedLoginMessage),
  Request(EncodedRequestMessage),
  Response(EncodedResponseMessage),
  Terminal(EncodedTerminalMessage),
}

impl<T: Into<TransportMessage> + Send> Encode<EncodedTransportMessage>
  for T
{
  fn encode(self) -> EncodedTransportMessage {
    let message = self.into();
    let variant_byte = message.extract_variant().as_byte();
    let mut bytes = match message {
      TransportMessage::Login(data) => data.0.into_vec(),
      TransportMessage::Request(data) => data.0.into_vec(),
      TransportMessage::Response(data) => data.0.into_vec(),
      TransportMessage::Terminal(data) => data.0.into_vec(),
    };
    bytes.push(variant_byte);
    EncodedTransportMessage(bytes)
  }
}

impl<T: From<TransportMessage>> Decode<T>
  for EncodedTransportMessage
{
  fn decode(self) -> anyhow::Result<T> {
    let mut bytes = self.0;
    let variant_byte = bytes
      .pop()
      .context("Failed to decode message | bytes are empty")?;
    use TransportMessageVariant::*;
    let message =
      match TransportMessageVariant::from_byte(variant_byte)? {
        Login => TransportMessage::Login(EncodedLoginMessage(
          EncodedResult::from_vec(bytes),
        )),
        Request => TransportMessage::Request(EncodedRequestMessage(
          EncodedChannel::from_vec(bytes),
        )),
        Response => TransportMessage::Response(
          EncodedResponseMessage(EncodedChannel::from_vec(bytes)),
        ),
        Terminal => TransportMessage::Terminal(
          EncodedTerminalMessage(EncodedChannel::from_vec(bytes)),
        ),
      };
    Ok(message.into())
  }
}

pub enum DecodedTransportMessage {
  Login(anyhow::Result<LoginMessage>),
  Request(WithChannel<EncodedJsonMessage>),
  Response(
    WithChannel<Option<anyhow::Result<EncodedJsonMessage>>>, // EncodedChannel<EncodedOption<EncodedResult<EncodedJsonMessage>>>,
  ),
  Terminal(WithChannel<Vec<u8>>),
}

impl<T: Into<DecodedTransportMessage> + Send> Encode<TransportMessage>
  for T
{
  fn encode(self) -> TransportMessage {
    use DecodedTransportMessage::*;
    match self.into() {
      Login(res) => TransportMessage::Login(EncodedLoginMessage(
        res.map(LoginMessage::encode).encode(),
      )),
      Request(data) => TransportMessage::Request(
        EncodedRequestMessage(data.encode()),
      ),
      Response(data) => {
        TransportMessage::Response(EncodedResponseMessage(
          data
            .map(|data| data.map(|data| data.encode()).encode())
            .encode(),
        ))
      }
      Terminal(data) => TransportMessage::Terminal(
        EncodedTerminalMessage(data.encode()),
      ),
    }
  }
}

impl Into<TransportMessage> for DecodedTransportMessage {
  fn into(self) -> TransportMessage {
    self.encode()
  }
}

impl<T: From<DecodedTransportMessage>> Decode<T>
  for TransportMessage
{
  fn decode(self) -> anyhow::Result<T> {
    let message = match self {
      TransportMessage::Login(encoded_result) => {
        let res =
          encoded_result.0.decode().and_then(|msg| msg.decode());
        DecodedTransportMessage::Login(res)
      }
      TransportMessage::Request(encoded_channel) => {
        DecodedTransportMessage::Request(encoded_channel.0.decode()?)
      }
      TransportMessage::Response(encoded_channel) => {
        let WithChannel { channel, data } =
          encoded_channel.0.decode()?;
        let data = data.decode()?.map(|data| data.decode());
        DecodedTransportMessage::Response(WithChannel {
          channel,
          data,
        })
      }
      TransportMessage::Terminal(encoded_channel) => {
        DecodedTransportMessage::Terminal(encoded_channel.0.decode()?)
      }
    };
    Ok(message.into())
  }
}

impl TransportMessageVariant {
  pub fn from_byte(byte: u8) -> anyhow::Result<Self> {
    use TransportMessageVariant::*;
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
    use TransportMessageVariant::*;
    match self {
      Login => 0,
      Request => 1,
      Response => 2,
      Terminal => 3,
    }
  }
}
