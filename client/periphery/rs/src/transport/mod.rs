use anyhow::{Context as _, anyhow};
use derive_variants::{EnumVariants, ExtractVariant as _};
use encoding::{
  CastBytes, Decode, Encode, EncodedChannel, EncodedJsonMessage,
  EncodedOption, EncodedResult, WithChannel, impl_cast_bytes_vec,
};

mod login;
pub use login::*;
use serde::de::DeserializeOwned;
use uuid::Uuid;

// ===================
//  TRANSPORT MESSAGE
// ===================

#[derive(Debug, Clone)]
pub struct EncodedTransportMessage(Vec<u8>);

impl_cast_bytes_vec!(EncodedTransportMessage, Vec);

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
    Ok(message)
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

// =================
//  REQUEST MESSAGE
// =================

#[derive(Debug)]
pub struct EncodedRequestMessage(EncodedChannel<EncodedJsonMessage>);

impl_cast_bytes_vec!(EncodedRequestMessage, EncodedChannel);

pub struct RequestMessage(WithChannel<EncodedJsonMessage>);

impl RequestMessage {
  pub fn map_decode<T: DeserializeOwned>(
    self,
  ) -> anyhow::Result<WithChannel<T>> {
    self.0.map_decode()
  }
}

impl RequestMessage {
  pub fn new(channel: Uuid, json: EncodedJsonMessage) -> Self {
    Self(WithChannel {
      channel,
      data: json,
    })
  }
}

impl Encode<EncodedTransportMessage> for RequestMessage {
  fn encode(self) -> EncodedTransportMessage {
    TransportMessage::Request(EncodedRequestMessage(self.0.encode()))
      .encode()
  }
}

impl Decode<RequestMessage> for EncodedRequestMessage {
  fn decode(self) -> anyhow::Result<RequestMessage> {
    self.0.decode().map(RequestMessage)
  }
}

// ==================
//  RESPONSE MESSAGE
// ==================

#[derive(Debug)]
pub struct EncodedResponseMessage(
  EncodedChannel<InnerEncodedResponseMessage>,
);

impl From<EncodedChannel<InnerEncodedResponseMessage>>
  for EncodedResponseMessage
{
  fn from(
    value: EncodedChannel<InnerEncodedResponseMessage>,
  ) -> Self {
    Self(value)
  }
}

impl_cast_bytes_vec!(EncodedResponseMessage, EncodedChannel);

/// This is handled as one bundle in main handler,
/// and passed to response handler for parsing.
#[derive(Debug)]
pub struct InnerEncodedResponseMessage(
  EncodedOption<EncodedResult<EncodedJsonMessage>>,
);

impl_cast_bytes_vec!(InnerEncodedResponseMessage, EncodedOption);

pub struct ResponseMessage(WithChannel<InnerEncodedResponseMessage>);

impl ResponseMessage {
  pub fn new(
    channel: Uuid,
    response: Option<EncodedResult<EncodedJsonMessage>>,
  ) -> Self {
    Self(WithChannel {
      channel,
      data: InnerEncodedResponseMessage(response.encode()),
    })
  }

  pub fn extract(
    self,
  ) -> WithChannel<EncodedOption<EncodedResult<EncodedJsonMessage>>>
  {
    self.0.map(|data| data.0)
  }
}

impl Encode<EncodedTransportMessage> for ResponseMessage {
  fn encode(self) -> EncodedTransportMessage {
    TransportMessage::Response(EncodedResponseMessage(
      self.0.encode(),
    ))
    .encode()
  }
}

impl Decode<ResponseMessage> for EncodedResponseMessage {
  fn decode(self) -> anyhow::Result<ResponseMessage> {
    self.0.decode().map(ResponseMessage)
  }
}

// ==================
//  TERMINAL MESSAGE
// ==================

#[derive(Debug)]
pub struct EncodedTerminalMessage(EncodedChannel<Vec<u8>>);

impl TerminalMessage {
  pub fn new(channel: Uuid, bytes: Vec<u8>) -> Self {
    Self(WithChannel {
      channel,
      data: bytes,
    })
  }
}

impl_cast_bytes_vec!(EncodedTerminalMessage, EncodedChannel);

pub struct TerminalMessage(WithChannel<Vec<u8>>);

impl Encode<EncodedTransportMessage> for TerminalMessage {
  fn encode(self) -> EncodedTransportMessage {
    TransportMessage::Terminal(EncodedTerminalMessage(
      self.0.encode(),
    ))
    .encode()
  }
}

impl Decode<WithChannel<Vec<u8>>> for EncodedTerminalMessage {
  fn decode(self) -> anyhow::Result<WithChannel<Vec<u8>>> {
    self.0.decode_map()
  }
}
