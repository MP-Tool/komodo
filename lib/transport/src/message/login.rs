use anyhow::{Context, anyhow};
use bytes::Bytes;
use derive_variants::EnumVariants;
use noise::key::SpkiPublicKey;

use crate::message::{CastBytes, Decode};

/// ```markdown
/// | -- u8[] -- | --------- u8 ------------ |
/// | <CONTENTS> | ParsedLoginMessageVariant |
/// ```
#[derive(Clone, Debug)]
pub struct LoginMessage(Bytes);

impl CastBytes for LoginMessage {
  fn from_bytes(bytes: Bytes) -> Self {
    Self(bytes)
  }
  fn into_bytes(self) -> Bytes {
    self.0
  }
}

impl Decode<ParsedLoginMessage> for LoginMessage {
  /// Parses login messages, performing various validations.
  fn decode(self) -> anyhow::Result<ParsedLoginMessage> {
    let mut bytes: Vec<u8> = self.0.into();
    let variant_byte = bytes
      .pop()
      .context("Failed to parse login message | bytes are empty")?;
    let variant = ParsedLoginMessageVariant::from_byte(variant_byte)?;
    use ParsedLoginMessageVariant::*;
    match variant {
      LoginSuccessful => Ok(ParsedLoginMessage::LoginSuccessful),
      Handshake => Ok(ParsedLoginMessage::Handshake(bytes.into())),
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
        Ok(ParsedLoginMessage::OnboardingFlow(onboarding_flow))
      }
      PublicKey => {
        if bytes.is_empty() {
          return Err(anyhow!(
            "Got empty LoginMessage OnboardingFlow PublicKey bytes"
          ));
        }
        let public_key = String::from_utf8(bytes)
          .context("Public key is not valid utf-8")?;
        Ok(ParsedLoginMessage::PublicKey(SpkiPublicKey::from(
          public_key,
        )))
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
        Ok(ParsedLoginMessage::V1PasskeyFlow(passkey_login))
      }
      V1Passkey => {
        if bytes.is_empty() {
          return Err(anyhow!(
            "Got empty LoginMessage V1Passkey bytes"
          ));
        }
        Ok(ParsedLoginMessage::V1Passkey(bytes.into()))
      }
    }
  }
}

#[derive(EnumVariants)]
#[variant_derive(Debug, Clone, Copy)]
pub enum ParsedLoginMessage {
  /// At the end of every login flow,
  /// Send a success message
  LoginSuccessful,
  /// Bytes that are part of the noise handshake.
  Handshake(Bytes),
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
  V1Passkey(Bytes),
}

impl ParsedLoginMessageVariant {
  pub fn from_byte(byte: u8) -> anyhow::Result<Self> {
    use ParsedLoginMessageVariant::*;
    let variant = match byte {
      0 => LoginSuccessful,
      1 => Handshake,
      2 => OnboardingFlow,
      3 => PublicKey,
      // V1
      4 => V1PasskeyFlow,
      5 => V1Passkey,
      other => {
        return Err(anyhow!(
          "Got unrecognized LoginMessageVariant byte: {other}"
        ));
      }
    };
    Ok(variant)
  }

  pub fn as_byte(self) -> u8 {
    use ParsedLoginMessageVariant::*;
    match self {
      LoginSuccessful => 0,
      Handshake => 1,
      OnboardingFlow => 2,
      PublicKey => 3,
      // V1
      V1PasskeyFlow => 4,
      V1Passkey => 5,
    }
  }
}
