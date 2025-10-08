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
  pub fn generate() -> anyhow::Result<Self> {
    let builder = snow::Builder::new(crate::NOISE_XX_PARAMS.parse()?);
    let keypair = builder
      .generate_keypair()
      .context("Failed to generate keypair")?;
    let private = Pkcs8PrivateKey::from_raw_bytes(&keypair.private)?;
    let public = SpkiPublicKey::from_raw_bytes(&keypair.public)?;
    Ok(Self { private, public })
  }

  pub fn generate_write_sync(
    path: impl AsRef<Path>,
  ) -> anyhow::Result<Self> {
    let path = path.as_ref();
    // Generate and write pems to path
    let keys = Self::generate()?;
    keys.private.write_pem_sync(path)?;
    keys.public.write_pem_sync(path.with_extension("pub"))?;
    Ok(keys)
  }

  pub async fn generate_write_async(
    path: impl AsRef<Path>,
  ) -> anyhow::Result<Self> {
    let path = path.as_ref();
    // Generate and write pems to path
    let keys = Self::generate()?;
    keys.private.write_pem_async(path).await?;
    keys
      .public
      .write_pem_async(path.with_extension("pub"))
      .await?;
    Ok(keys)
  }

  pub fn load_maybe_generate(
    private_key_path: impl AsRef<Path>,
  ) -> anyhow::Result<Self> {
    let path = private_key_path.as_ref();

    let exists = path.try_exists().with_context(|| {
      format!("Invalid private key path: {path:?}")
    })?;

    if !exists {
      return Self::generate_write_sync(path);
    }

    let private = Pkcs8PrivateKey::from_file(private_key_path)?;
    let public = private.compute_public_key()?;

    Ok(Self { private, public })
  }

  pub fn from_private_key(
    maybe_pkcs8_private_key: &str,
  ) -> anyhow::Result<Self> {
    let private =
      Pkcs8PrivateKey::from_maybe_raw_bytes(maybe_pkcs8_private_key)?;
    let public = private.compute_public_key()?;
    Ok(Self { private, public })
  }
}
