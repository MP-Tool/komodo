use serde::{Deserialize, Serialize};
use typeshare::typeshare;

use crate::entities::server::ServerConfig;

use super::I64;

/// An public key used to authenticate new Periphery -> Core connections
/// to join Komodo as a newly created Server.
///
/// Server onboarding keys correspond to private / public key pairs.
/// While the public key is stored, the private key will only be returned to the user,
/// The private key will not be stored or available afterwards, just like the api key "secret".
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[cfg_attr(
  feature = "mongo",
  derive(mongo_indexed::derive::MongoIndexed)
)]
pub struct ServerOnboardingKey {
  /// Unique public key associated the creation private key.
  #[cfg_attr(feature = "mongo", unique_index)]
  pub public_key: String,

  /// Name associated with the api key for management
  pub name: String,

  /// The [Server](crate::entities::server::Server) ids onboarded by this Creation Key
  pub onboarded: Vec<String>,

  /// Timestamp of key creation
  pub created_at: I64,

  /// Expiry of key, or 0 if never expires
  pub expires: I64,

  /// Default tags to give to Servers created with this key.
  pub default_tags: Vec<String>,

  /// The default [ServerConfig] to give to these Servers.
  pub default_config: ServerConfig,
}
