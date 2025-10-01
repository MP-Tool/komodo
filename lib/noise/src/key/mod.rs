use anyhow::Context;
use der::AnyRef;

pub mod command;

mod private;
mod public;

pub use private::Pkcs8PrivateKey;
pub use public::SpkiPublicKey;

const OID_X25519: spki::ObjectIdentifier =
  spki::ObjectIdentifier::new_unwrap("1.3.101.110");

fn algorithm() -> spki::AlgorithmIdentifier<AnyRef<'static>> {
  spki::AlgorithmIdentifier {
    oid: OID_X25519,
    parameters: None,
  }
}

pub struct EncodedKeyPair {
  /// pkcs8 encoded private key
  pub private: String,
  /// spki encoded public key
  pub public: String,
}

impl EncodedKeyPair {
  pub fn generate() -> anyhow::Result<EncodedKeyPair> {
    let builder = snow::Builder::new(crate::NOISE_XX_PARAMS.parse()?);
    let keypair = builder
      .generate_keypair()
      .context("Failed to generate keypair")?;
    let private =
      Pkcs8PrivateKey::from_raw_bytes(&keypair.private)?.into_inner();
    let public =
      SpkiPublicKey::from_raw_bytes(&keypair.public)?.into_inner();
    Ok(EncodedKeyPair { private, public })
  }
}
