use std::sync::Arc;

use komodo_client::entities::NoData;
use noise::key::{
  EncodedKeyPair, SpkiPublicKey, generate_write_keys,
};
use periphery_client::api::keys::{
  RotateCorePublicKey, RotatePrivateKey, RotatePrivateKeyResponse,
};
use resolver_api::Resolve;

use crate::{
  config::{periphery_config, periphery_private_key},
  connection::core_public_keys,
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

    let Some(core_public_keys_spec) =
      config.core_public_keys.as_ref()
    else {
      return Ok(NoData {});
    };

    let Some(core_public_key_path) = core_public_keys_spec
      .first()
      .and_then(|key| key.strip_prefix("file:"))
    else {
      return Ok(NoData {});
    };

    SpkiPublicKey::from(self.public_key)
      .write_pem_sync(core_public_key_path)?;

    core_public_keys().refresh();

    Ok(NoData {})
  }
}
