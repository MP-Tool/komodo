use anyhow::{Context, anyhow};
use database::mungos::find::find_collect;
use komodo_client::api::read::{
  ListOnboardingKeys, ListOnboardingKeysResponse,
};
use reqwest::StatusCode;
use resolver_api::Resolve;
use serror::AddStatusCodeError;

use crate::{api::read::ReadArgs, state::db_client};

//

impl Resolve<ReadArgs> for ListOnboardingKeys {
  async fn resolve(
    self,
    ReadArgs { user: admin }: &ReadArgs,
  ) -> serror::Result<ListOnboardingKeysResponse> {
    if !admin.admin {
      return Err(
        anyhow!("This call is admin only")
          .status_code(StatusCode::FORBIDDEN),
      );
    }
    find_collect(&db_client().onboarding_keys, None, None)
      .await
      .context("Failed to query database for Server onboarding keys")
      .map_err(Into::into)
  }
}
