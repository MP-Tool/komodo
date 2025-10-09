use komodo_client::entities::NoData;
use noise::key::SpkiPublicKey;
use periphery_client::api::keys::{
  RotateCorePublicKey, RotatePrivateKey, RotatePrivateKeyResponse,
};
use resolver_api::Resolve;

use crate::{
  config::{periphery_config, periphery_keys},
  connection::core_public_keys,
};

//

impl Resolve<super::Args> for RotatePrivateKey {
  async fn resolve(
    self,
    _: &super::Args,
  ) -> serror::Result<RotatePrivateKeyResponse> {
    let public_key = periphery_keys().rotate().await?.into_inner();
    info!("New Public Key: {public_key}");
    Ok(RotatePrivateKeyResponse { public_key })
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

    let public_key = SpkiPublicKey::from(self.public_key);

    // Check equality at path before trying to rewrite.
    match SpkiPublicKey::from_file(path) {
      Ok(existing) if existing == public_key => {}
      _ => public_key.write_pem_async(path).await?,
    }

    core_public_keys().refresh();

    Ok(NoData {})
  }
}
