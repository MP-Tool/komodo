use anyhow::Context;
use base64::{Engine as _, prelude::BASE64_STANDARD};
use der::{Encode as _, asn1::BitStringRef};

#[derive(PartialEq)]
pub struct SpkiPublicKey(String);

impl From<String> for SpkiPublicKey {
  fn from(value: String) -> Self {
    Self(value)
  }
}

impl std::fmt::Display for SpkiPublicKey {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.0)
  }
}

impl SpkiPublicKey {
  pub fn into_inner(self) -> String {
    self.0
  }

  pub fn from_pem(public_key_pem: &str) -> anyhow::Result<Self> {
    let (_label, public_key_der) =
      pem_rfc7468::decode_vec(public_key_pem.as_bytes())
        .map_err(anyhow::Error::msg)
        .context("Failed to get der from pem")?;
    Ok(Self(BASE64_STANDARD.encode(public_key_der)))
  }

  pub fn from_raw_bytes(public_key: &[u8]) -> anyhow::Result<Self> {
    let bs = BitStringRef::new(0, public_key)
      .map_err(anyhow::Error::msg)
      .context("Failed to parse public key bytes into bit string")?;

    let spki = spki::SubjectPublicKeyInfo {
      algorithm: super::algorithm(),
      subject_public_key: bs,
    };

    let mut buf = [0u8; 128];
    let public_key = spki
      .encode_to_slice(&mut buf)
      .map_err(anyhow::Error::msg)
      .context("Failed to write subject public key info into der")?;

    Ok(Self(BASE64_STANDARD.encode(public_key)))
  }

  pub fn from_private_key(
    maybe_pkcs8_private_key: &str,
  ) -> anyhow::Result<Self> {
    // Create mock client handshake. The private key doesn't matter.
    // Trying to get the "server" public key.
    let mut client_handshake =
      crate::NoiseHandshake::new_initiator("0000", &[])
        .context("Failed to create client handshake")?;
    // Create mock server handshake.
    // Use the target private key with server handshake,
    // since its public key is the first available in the flow.
    let mut server_handshake = crate::NoiseHandshake::new_responder(
      maybe_pkcs8_private_key,
      &[],
    )
    .context("Failed to create server handshake")?;
    // write message 1
    let message_1 = client_handshake
      .next_message()
      .context("CLIENT: failed to write message 1")?;
    // read message 1
    server_handshake
      .read_message(&message_1)
      .context("SERVER: failed to read message 1")?;
    // write message 2
    let message_2 = server_handshake
      .next_message()
      .context("SERVER: failed to write message 2")?;
    // read message 2
    client_handshake
      .read_message(&message_2)
      .context("CLIENT: failed to read message 2")?;
    // client now has server public key
    let raw_public_key = client_handshake
      .remote_public_key()
      .map(Vec::from)
      .context("Failed to get public key")?;
    Self::from_raw_bytes(&raw_public_key)
  }
}
