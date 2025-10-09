use anyhow::{Context, anyhow};
use derive_variants::{EnumVariants, ExtractVariant};
use noise::key::SpkiPublicKey;

use crate::{
  auth::AUTH_TIMEOUT,
  message::{CastBytes, Decode, Encode, Message},
  websocket::{Websocket, WebsocketExt},
};

pub trait LoginWebsocketExt: WebsocketExt {
  fn send_login_error(
    &mut self,
    e: &anyhow::Error,
  ) -> impl Future<Output = anyhow::Result<()>> + Send {
    let message = Message::Login(e.encode());
    self.send(message)
  }

  fn recv_login_message(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<LoginMessage>> + Send {
    async {
      let Message::Login(message) =
        self.recv().with_timeout(AUTH_TIMEOUT).await?
      else {
        return Err(anyhow!(
          "Expected Login message, got other message type"
        ));
      };
      message.decode_into()
    }
  }

  fn recv_login_success(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<()>> + Send {
    async {
      let LoginMessage::Success = self.recv_login_message().await?
      else {
        return Err(anyhow!(
          "Expected Login Success message, got other message type"
        ));
      };
      Ok(())
    }
  }

  fn recv_login_nonce(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<[u8; 32]>> + Send {
    async {
      let LoginMessage::Nonce(nonce) =
        self.recv_login_message().await?
      else {
        return Err(anyhow!(
          "Expected Login Nonce message, got other message type"
        ));
      };
      Ok(nonce)
    }
  }

  fn recv_login_handshake_bytes(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<Vec<u8>>> + Send {
    async {
      let LoginMessage::Handshake(bytes) =
        self.recv_login_message().await?
      else {
        return Err(anyhow!(
          "Expected Login Handshake message, got other message type"
        ));
      };
      Ok(bytes)
    }
  }

  fn recv_login_onboarding_flow(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<bool>> + Send {
    async {
      let LoginMessage::OnboardingFlow(onboarding_flow) =
        self.recv_login_message().await?
      else {
        return Err(anyhow!(
          "Expected Login OnboardingFlow message, got other message type"
        ));
      };
      Ok(onboarding_flow)
    }
  }

  fn recv_login_public_key(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<SpkiPublicKey>> + Send {
    async {
      let LoginMessage::PublicKey(public_key) =
        self.recv_login_message().await?
      else {
        return Err(anyhow!(
          "Expected Login PublicKey message, got other message type"
        ));
      };
      Ok(public_key)
    }
  }

  fn recv_login_v1_passkey_flow(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<bool>> + Send {
    async {
      let LoginMessage::V1PasskeyFlow(v1_passkey_flow) =
        self.recv_login_message().await?
      else {
        return Err(anyhow!(
          "Expected Login V1PasskeyFlow message, got other message type"
        ));
      };
      Ok(v1_passkey_flow)
    }
  }

  fn recv_login_v1_passkey(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<Vec<u8>>> + Send {
    async {
      let LoginMessage::V1Passkey(bytes) =
        self.recv_login_message().await?
      else {
        return Err(anyhow!(
          "Expected Login V1Passkey message, got other message type"
        ));
      };
      Ok(bytes)
    }
  }
}

impl<W: Websocket> LoginWebsocketExt for W {}

impl From<LoginMessage> for Message {
  fn from(value: LoginMessage) -> Self {
    Self::Login(Ok(value.encode()).encode())
  }
}

/// ```markdown
/// | -- u8[] -- | --------- u8 ------------ |
/// | <CONTENTS> | LoginMessageVariant |
/// ```
#[derive(Clone, Debug)]
pub struct LoginMessageBytes(Vec<u8>);

impl CastBytes for LoginMessageBytes {
  fn from_vec(vec: Vec<u8>) -> Self {
    Self(vec)
  }
  fn into_vec(self) -> Vec<u8> {
    self.0
  }
}

#[derive(EnumVariants, Clone)]
#[variant_derive(Debug, Clone, Copy)]
pub enum LoginMessage {
  /// At the end of every login flow,
  /// Send a success message
  Success,
  /// Every handshake includes a random 32 byte nonce
  /// to identify the connection.
  Nonce([u8; 32]),
  /// Bytes that are part of the noise handshake.
  Handshake(Vec<u8>),
  /// Used during Periphery -> Core connections.
  /// Core must let Periphery know which flow to use
  /// before the handshake is started, so it can use
  /// the onboarding key.
  OnboardingFlow(bool),
  /// The onboarding flow requires Periphery to send
  /// over its public key to initialize the Server with
  /// allowed connection.
  PublicKey(SpkiPublicKey),
  /// Used during Core -> Periphery connections.
  /// If Periphery hasn't set `core_public_keys`,
  /// will fall back to passkey auth
  /// for backward compatability with v1
  V1PasskeyFlow(bool),
  /// Core will send the passkey to Periphery to validate
  /// in the V1PasskeyLogin flow.
  V1Passkey(Vec<u8>),
}

impl Encode<LoginMessageBytes> for LoginMessage {
  fn encode(self) -> LoginMessageBytes {
    let variant_byte = self.extract_variant().as_byte();
    let mut bytes = match self {
      LoginMessage::Success => Vec::new(),
      LoginMessage::Nonce(nonce) => nonce.to_vec(),
      LoginMessage::Handshake(bytes) => bytes,
      LoginMessage::OnboardingFlow(onboarding_flow) => {
        let byte = if onboarding_flow { 1 } else { 0 };
        vec![byte]
      }
      LoginMessage::PublicKey(spki_public_key) => {
        spki_public_key.into_inner().into()
      }
      LoginMessage::V1PasskeyFlow(passkey_flow) => {
        let byte = if passkey_flow { 1 } else { 0 };
        vec![byte]
      }
      LoginMessage::V1Passkey(bytes) => bytes,
    };
    bytes.push(variant_byte);
    LoginMessageBytes(bytes)
  }
}

impl Decode<LoginMessage> for LoginMessageBytes {
  /// Parses login messages, performing various validations.
  fn decode(self) -> anyhow::Result<LoginMessage> {
    let mut bytes = self.0;
    let variant_byte = bytes
      .pop()
      .context("Failed to parse login message | bytes are empty")?;
    let variant = LoginMessageVariant::from_byte(variant_byte)?;
    use LoginMessageVariant::*;
    match variant {
      Success => Ok(LoginMessage::Success),

      Nonce => Ok(LoginMessage::Nonce(
        bytes
          .try_into()
          .map_err(|_| anyhow!("Invalid connection nonce"))?,
      )),

      Handshake => Ok(LoginMessage::Handshake(bytes)),

      OnboardingFlow => {
        let onboarding_flow = match bytes.as_slice() {
          &[0] => false,
          &[1] => true,
          other => {
            return Err(anyhow!(
              "Got unrecognized LoginMessage OnboardingFlow bytes: {other:?}"
            ));
          }
        };
        Ok(LoginMessage::OnboardingFlow(onboarding_flow))
      }

      PublicKey => {
        if bytes.is_empty() {
          return Err(anyhow!(
            "Got empty LoginMessage OnboardingFlow PublicKey bytes"
          ));
        }
        let public_key = String::from_utf8(bytes)
          .context("Public key is not valid utf-8")?;
        Ok(LoginMessage::PublicKey(SpkiPublicKey::from(public_key)))
      }

      // V1
      V1PasskeyFlow => {
        let passkey_login = match bytes.as_slice() {
          &[0] => false,
          &[1] => true,
          other => {
            return Err(anyhow!(
              "Got unrecognized LoginMessage V1PasskeyLogin bytes: {other:?}"
            ));
          }
        };
        Ok(LoginMessage::V1PasskeyFlow(passkey_login))
      }

      V1Passkey => {
        if bytes.is_empty() {
          return Err(anyhow!(
            "Got empty LoginMessage V1Passkey bytes"
          ));
        }
        Ok(LoginMessage::V1Passkey(bytes))
      }
    }
  }
}

impl LoginMessageVariant {
  pub fn from_byte(byte: u8) -> anyhow::Result<Self> {
    use LoginMessageVariant::*;
    let variant = match byte {
      0 => Success,
      1 => Nonce,
      2 => Handshake,
      3 => OnboardingFlow,
      4 => PublicKey,
      // V1
      5 => V1PasskeyFlow,
      6 => V1Passkey,
      other => {
        return Err(anyhow!(
          "Got unrecognized LoginMessageVariant byte: {other}"
        ));
      }
    };
    Ok(variant)
  }

  pub fn as_byte(self) -> u8 {
    use LoginMessageVariant::*;
    match self {
      Success => 0,
      Nonce => 1,
      Handshake => 2,
      OnboardingFlow => 3,
      PublicKey => 4,
      // V1
      V1PasskeyFlow => 5,
      V1Passkey => 6,
    }
  }
}
