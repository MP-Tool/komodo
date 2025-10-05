use std::sync::Arc;

use komodo_client::entities::NoData;
use noise::key::{
  EncodedKeyPair, SpkiPublicKey, generate_write_keys,
};
use periphery_client::api::keys::{
  RotateCorePublicKey, RotatePrivateKey, RotatePrivateKeyResponse,
};
use resolver_api::Resolve;

use crate::config::{
  core_public_keys, periphery_config, periphery_private_key,
};

//

impl Resolve<super::Args> for RotatePrivateKey {
  async fn resolve(
    self,
    _: &super::Args,
  ) -> serror::Result<RotatePrivateKeyResponse> {
    let config = periphery_config();
    let keys = match config.private_key.as_ref() {
      Some(private_key) => match private_key.strip_prefix("file:") {
        None => EncodedKeyPair::from_private_key(private_key)?,
        Some(path) => generate_write_keys(path)?,
      },
      None => generate_write_keys(
        config.root_directory.join("keys/periphery.key"),
      )?,
    };
    // Store new private key for next auth
    periphery_private_key()
      .store(Arc::new(keys.private.into_inner()));
    Ok(RotatePrivateKeyResponse {
      public_key: keys.public.into_inner(),
    })
  }
}

//

impl Resolve<super::Args> for RotateCorePublicKey {
  async fn resolve(self, _: &super::Args) -> serror::Result<NoData> {
    let config = periphery_config();
    let (Some(core_public_keys_spec), Some(core_public_keys)) =
      (config.core_public_keys.as_ref(), core_public_keys())
    else {
      return Ok(NoData {});
    };
    let Some(core_public_key_path) = core_public_keys_spec
      .first()
      .and_then(|key| key.strip_prefix("file:"))
    else {
      return Ok(NoData {});
    };
    let public_key = SpkiPublicKey::from(self.public_key);
    public_key.write_pem(core_public_key_path)?;
    let mut new_core_public_keys = vec![public_key];
    // This only replaces the first, extend the rest.
    new_core_public_keys
      .extend_from_slice(&core_public_keys.load().as_slice()[1..]);
    // Store the new keys for the next auth
    core_public_keys.store(Arc::new(new_core_public_keys));
    Ok(NoData {})
  }
}
