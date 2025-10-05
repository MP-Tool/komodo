use komodo_client::entities::NoData;
use resolver_api::Resolve;
use serde::{Deserialize, Serialize};

//

#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[response(RotatePrivateKeyResponse)]
#[error(serror::Error)]
pub struct RotatePrivateKey {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotatePrivateKeyResponse {
  /// The new public key
  pub public_key: String,
}

//

#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[response(NoData)]
#[error(serror::Error)]
pub struct RotateCorePublicKey {
  /// The new Core public key.
  pub public_key: String,
}
