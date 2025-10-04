use derive_empty_traits::EmptyTraits;
use resolver_api::Resolve;
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

use crate::entities::server_onboarding_key::ServerOnboardingKey;

use super::KomodoReadRequest;

/// **Admin only.** Gets list of creation keys.
/// Response: [ListServerOnboardingKeysResponse]
#[typeshare]
#[derive(
  Debug, Clone, Serialize, Deserialize, Resolve, EmptyTraits,
)]
#[empty_traits(KomodoReadRequest)]
#[response(ListServerOnboardingKeysResponse)]
#[error(serror::Error)]
pub struct ListServerOnboardingKeys {}

#[typeshare]
pub type ListServerOnboardingKeysResponse = Vec<ServerOnboardingKey>;
