use anyhow::{Context as _, anyhow};
use derive_variants::{EnumVariants, ExtractVariant as _};
use encoding::{
  CastBytes, Decode, Encode, EncodedChannel, EncodedJsonMessage,
  EncodedOption, EncodedResult, WithChannel,
};

mod login;
pub use login::*;
use serde::de::DeserializeOwned;

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

// ===================
//  TRANSPORT MESSAGE
// ===================

/// When an EncodedTransportMessage is received,
/// it is decoded into this type.
///
/// Note that inner bytes for top level message variants are left as is,
/// as their decoding is left to specific handler.
/// The main receiving hot loop should do minimal parsing.
#[derive(Debug, EnumVariants)]
#[variant_derive(Debug, Clone, Copy)]
pub enum TransportMessage {
  Login(EncodedLoginMessage),
  Request(EncodedRequestMessage),
  Response(EncodedResponseMessage),
  Terminal(EncodedTerminalMessage),
}

impl Encode<EncodedTransportMessage> for TransportMessage {
  fn encode(self) -> EncodedTransportMessage {
    let variant_byte = self.extract_variant().as_byte();
    let mut bytes = match self {
      TransportMessage::Login(data) => data.into_vec(),
      TransportMessage::Request(data) => data.0.into_vec(),
      TransportMessage::Response(data) => data.0.into_vec(),
      TransportMessage::Terminal(data) => data.0.into_vec(),
    };
    bytes.push(variant_byte);
    EncodedTransportMessage(bytes)
  }
}

impl Decode<TransportMessage> for EncodedTransportMessage {
  fn decode(self) -> anyhow::Result<TransportMessage> {
    let mut bytes = self.0;
    let variant_byte = bytes
      .pop()
      .context("Failed to decode message | bytes are empty")?;
    use TransportMessageVariant::*;
    let message =
      match TransportMessageVariant::from_byte(variant_byte)? {
        Login => TransportMessage::Login(
          EncodedLoginMessage::from_vec(bytes),
        ),
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

// ===================
//  DECODED TRANSPORT
// ===================

pub enum DecodedTransportMessage {
  Login(anyhow::Result<LoginMessage>),
  Request(WithChannel<EncodedJsonMessage>),
  Response(
    WithChannel<Option<anyhow::Result<EncodedJsonMessage>>>, // EncodedChannel<EncodedOption<EncodedResult<EncodedJsonMessage>>>,
  ),
  Terminal(WithChannel<Vec<u8>>),
}

impl Encode<TransportMessage> for DecodedTransportMessage {
  fn encode(self) -> TransportMessage {
    use DecodedTransportMessage::*;
    match self {
      Login(res) => TransportMessage::Login(res.encode()),
      Request(data) => TransportMessage::Request(
        EncodedRequestMessage(data.encode()),
      ),
      Response(data) => {
        TransportMessage::Response(EncodedResponseMessage(
          data
            .map(|data| encoding::Option::from(data).map_encode())
            .encode(),
        ))
      }
      Terminal(data) => TransportMessage::Terminal(
        EncodedTerminalMessage(data.encode()),
      ),
    }
  }
}

impl Encode<EncodedTransportMessage> for DecodedTransportMessage {
  fn encode(self) -> EncodedTransportMessage {
    let res: TransportMessage = self.encode();
    res.encode()
  }
}

impl<T: From<DecodedTransportMessage>> Decode<T>
  for TransportMessage
{
  fn decode(self) -> anyhow::Result<T> {
    let message = match self {
      TransportMessage::Login(encoded_result) => {
        DecodedTransportMessage::Login(encoded_result.decode())
      }
      TransportMessage::Request(encoded_channel) => {
        DecodedTransportMessage::Request(encoded_channel.0.decode()?)
      }
      TransportMessage::Response(encoded_channel) => {
        DecodedTransportMessage::Response(
          encoded_channel.0.decode()?.map(|data| data.decode_map()),
        )
      }
      TransportMessage::Terminal(encoded_channel) => {
        DecodedTransportMessage::Terminal(encoded_channel.0.decode()?)
      }
    };
    Ok(message.into())
  }
}

// =================
//  REQUEST MESSAGE
// =================

#[derive(Debug)]
pub struct EncodedRequestMessage(EncodedChannel<EncodedJsonMessage>);

impl CastBytes for EncodedRequestMessage {
  fn from_vec(bytes: Vec<u8>) -> Self {
    Self(EncodedChannel::from_vec(bytes))
  }
  fn into_vec(self) -> Vec<u8> {
    self.0.into_vec()
  }
}

impl Encode<EncodedRequestMessage>
  for WithChannel<EncodedJsonMessage>
{
  fn encode(self) -> EncodedRequestMessage {
    EncodedRequestMessage(self.encode())
  }
}

impl<T: DeserializeOwned> Decode<WithChannel<T>>
  for EncodedRequestMessage
{
  fn decode(self) -> anyhow::Result<WithChannel<T>> {
    let WithChannel { channel, data } = self.0.decode()?;
    let data = data.decode()?;
    Ok(WithChannel { channel, data })
  }
}

// ==================
//  RESPONSE MESSAGE
// ==================

#[derive(Debug)]
pub struct EncodedResponseMessage(
  EncodedChannel<EncodedOption<EncodedResult<EncodedJsonMessage>>>,
);

impl CastBytes for EncodedResponseMessage {
  fn from_vec(bytes: Vec<u8>) -> Self {
    Self(EncodedChannel::from_vec(bytes))
  }
  fn into_vec(self) -> Vec<u8> {
    self.0.into_vec()
  }
}

impl Encode<EncodedResponseMessage>
  for WithChannel<
    encoding::Option<encoding::Result<EncodedJsonMessage>>,
  >
{
  fn encode(self) -> EncodedResponseMessage {
    EncodedResponseMessage(
      self.map(|data| data.map_encode()).encode(),
    )
  }
}

impl Encode<EncodedResponseMessage>
  for WithChannel<EncodedResult<EncodedJsonMessage>>
{
  fn encode(self) -> EncodedResponseMessage {
    EncodedResponseMessage(self.map(Some).map_encode())
  }
}

impl<T: DeserializeOwned>
  Decode<WithChannel<Option<anyhow::Result<T>>>>
  for EncodedResponseMessage
{
  fn decode(
    self,
  ) -> anyhow::Result<WithChannel<Option<anyhow::Result<T>>>> {
    let WithChannel { channel, data } = self.0.decode()?;
    let data = data.decode()?.into_std();
    // Does it this way to make sure the inner result is preserved
    // for later handling.
    let Some(data) = data else {
      return Ok(WithChannel {
        channel,
        data: None,
      });
    };
    let data = data.decode().and_then(|data| data.decode());
    Ok(WithChannel {
      channel,
      data: Some(data),
    })
  }
}

impl
  Decode<
    WithChannel<EncodedOption<EncodedResult<EncodedJsonMessage>>>,
  > for EncodedResponseMessage
{
  fn decode(
    self,
  ) -> anyhow::Result<
    WithChannel<EncodedOption<EncodedResult<EncodedJsonMessage>>>,
  > {
    self.0.decode()
  }
}

// ==================
//  TERMINAL MESSAGE
// ==================

#[derive(Debug)]
pub struct EncodedTerminalMessage(EncodedChannel<Vec<u8>>);

impl CastBytes for EncodedTerminalMessage {
  fn from_vec(bytes: Vec<u8>) -> Self {
    Self(EncodedChannel::from_vec(bytes))
  }
  fn into_vec(self) -> Vec<u8> {
    self.0.into_vec()
  }
}

impl Encode<EncodedTerminalMessage> for WithChannel<Vec<u8>> {
  fn encode(self) -> EncodedTerminalMessage {
    EncodedTerminalMessage(self.encode())
  }
}

impl Decode<WithChannel<Vec<u8>>> for EncodedTerminalMessage {
  fn decode(self) -> anyhow::Result<WithChannel<Vec<u8>>> {
    self.0.decode_map()
  }
}
