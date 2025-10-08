use std::path::Path;

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
  pub private: Pkcs8PrivateKey,
  /// spki encoded public key
  pub public: SpkiPublicKey,
}

impl EncodedKeyPair {
  pub fn generate() -> anyhow::Result<EncodedKeyPair> {
    let builder = snow::Builder::new(crate::NOISE_XX_PARAMS.parse()?);
    let keypair = builder
      .generate_keypair()
      .context("Failed to generate keypair")?;
    let private = Pkcs8PrivateKey::from_raw_bytes(&keypair.private)?;
    let public = SpkiPublicKey::from_raw_bytes(&keypair.public)?;
    Ok(EncodedKeyPair { private, public })
  }

  pub fn from_private_key(
    maybe_pkcs8_private_key: &str,
  ) -> anyhow::Result<EncodedKeyPair> {
    let private =
      Pkcs8PrivateKey::from_maybe_raw_bytes(maybe_pkcs8_private_key)?;
    let public = private.compute_public_key()?;
    Ok(Self { private, public })
  }
}

pub fn load_maybe_generate_private_key(
  path: impl AsRef<Path>,
) -> anyhow::Result<String> {
  let path = path.as_ref();
  if path
    .try_exists()
    .with_context(|| format!("Invalid private key path: {path:?}"))?
  {
    // Already exists, load it
    std::fs::read_to_string(path).with_context(|| {
      format!("Failed to read private key at {path:?}")
    })
  } else {
    let keys = generate_write_keys(path)?;
    Ok(keys.private.into_inner())
  }
}

pub fn generate_write_keys(
  path: impl AsRef<Path>,
) -> anyhow::Result<EncodedKeyPair> {
  let path = path.as_ref();
  // Generate and write pems to path
  let keys = EncodedKeyPair::generate()?;
  keys.private.write_pem_sync(path)?;
  keys.public.write_pem_sync(path.with_extension("pub"))?;
  Ok(keys)
}
