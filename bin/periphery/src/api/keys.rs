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
  config::{
    periphery_config, periphery_private_key, periphery_public_key,
  },
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
        None => {
          // If the private key is static, just return the public key.
          return Ok(RotatePrivateKeyResponse {
            public_key: EncodedKeyPair::from_private_key(
              private_key,
            )?
            .public
            .into_inner(),
          });
        }
        Some(path) => generate_write_keys(path)?,
      },
      None => generate_write_keys(
        config.root_directory.join("keys/periphery.key"),
      )?,
    };

    info!("New Public Key: {}", keys.public);

    periphery_private_key()
      .store(Arc::new(keys.private.into_inner()));
    periphery_public_key().store(Arc::new(keys.public.clone()));

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

    let Some(path) = core_public_keys_spec
      .iter()
      // Finds the first Core Public Key in spec with `file` prefix.
      .find_map(|public_keys| public_keys.strip_prefix("file:"))
    else {
      return Ok(NoData {});
    };

    SpkiPublicKey::from(self.public_key)
      .write_pem_async(path)
      .await?;

    core_public_keys().refresh();

    Ok(NoData {})
  }
}
