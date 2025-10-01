use anyhow::{Context, anyhow};
use base64::{Engine as _, prelude::BASE64_STANDARD};
use der::{Decode as _, Encode as _, asn1::OctetStringRef};

#[derive(PartialEq)]
pub struct Pkcs8PrivateKey(String);

impl From<String> for Pkcs8PrivateKey {
  fn from(value: String) -> Self {
    Self(value)
  }
}

impl std::fmt::Display for Pkcs8PrivateKey {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.0)
  }
}

impl Pkcs8PrivateKey {
  pub fn into_inner(self) -> String {
    self.0
  }

  pub fn from_raw_bytes(private_key: &[u8]) -> anyhow::Result<Self> {
    if private_key.len() > 32 {
      return Err(anyhow!(
        "Private key bytes too long, expected 32 bytes or less."
      ));
    }

    let mut buf = [0u8; 32];
    buf[..private_key.len()].copy_from_slice(private_key);

    let octet = OctetStringRef::new(&buf)
      .map_err(anyhow::Error::msg)
      .context("Failed to parse private key bytes into octet")?;

    let mut buf = [0u8; 128];
    let octet_der = octet
      .encode_to_slice(&mut buf)
      .map_err(anyhow::Error::msg)
      .context("Failed to write private key octet into der")?;

    let pki = pkcs8::PrivateKeyInfo {
      algorithm: super::algorithm(),
      private_key: octet_der,
      public_key: None,
    };

    let mut buf = [0u8; 128];
    let private_key = pki
      .encode_to_slice(&mut buf)
      .map_err(anyhow::Error::msg)
      .context("Failed to write private key info into der")?;

    Ok(Self(BASE64_STANDARD.encode(private_key)))
  }

  pub fn as_raw_bytes(&self) -> anyhow::Result<Vec<u8>> {
    Self::raw_bytes(self.0.as_bytes())
  }

  pub fn raw_bytes(
    pkcs8_private_key: &[u8],
  ) -> anyhow::Result<Vec<u8>> {
    let decoded = BASE64_STANDARD
      .decode(pkcs8_private_key)
      .context("Private key is not valid base64 encoding")?;
    Self::raw_bytes_after_decode(&decoded)
  }

  fn raw_bytes_after_decode(
    decoded: &[u8],
  ) -> anyhow::Result<Vec<u8>> {
    let pki = pkcs8::PrivateKeyInfo::from_der(decoded)
      .map_err(anyhow::Error::msg)
      .context("Failed to parse pki from der")?;
    if pki.algorithm.oid != super::OID_X25519 {
      return Err(anyhow!("Private key is not X25519"));
    }
    let octet = OctetStringRef::from_der(pki.private_key)
      .map_err(anyhow::Error::msg)
      .context("Failed to get octet string ref from private key")?;
    Ok(octet.as_bytes().to_vec())
  }

  pub fn maybe_raw_bytes(
    maybe_pkcs8_private_key: &str,
  ) -> anyhow::Result<Vec<u8>> {
    // pem
    if maybe_pkcs8_private_key.starts_with("-----BEGIN") {
      let (_label, private_key_der) =
        pem_rfc7468::decode_vec(maybe_pkcs8_private_key.as_bytes())
          .map_err(anyhow::Error::msg)
          .context("Failed to get der from pem")?;
      return Self::raw_bytes_after_decode(&private_key_der);
    }
    let len = maybe_pkcs8_private_key.len();
    if len == 64 {
      // base64 der
      Self::raw_bytes(maybe_pkcs8_private_key.as_bytes())
    } else if len <= 32 {
      Ok(maybe_pkcs8_private_key.as_bytes().to_vec())
    } else {
      Err(anyhow!(
        "Private key must be less than 32 characters, or pkcs8 encoded."
      ))
    }
  }

  pub fn compute_public_key(
    &self,
  ) -> anyhow::Result<super::public::SpkiPublicKey> {
    super::public::SpkiPublicKey::from_private_key(&self.0)
  }
}
