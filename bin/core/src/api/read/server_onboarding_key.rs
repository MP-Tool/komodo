use anyhow::{Context, anyhow};
use database::mungos::find::find_collect;
use komodo_client::api::read::{
  ListServerOnboardingKeys, ListServerOnboardingKeysResponse,
};
use reqwest::StatusCode;
use resolver_api::Resolve;
use serror::AddStatusCodeError;

use crate::{api::read::ReadArgs, state::db_client};

//

impl Resolve<ReadArgs> for ListServerOnboardingKeys {
  async fn resolve(
    self,
    ReadArgs { user: admin }: &ReadArgs,
  ) -> serror::Result<ListServerOnboardingKeysResponse> {
    if !admin.admin {
      return Err(
        anyhow!("This call is admin only")
          .status_code(StatusCode::FORBIDDEN),
      );
    }
    find_collect(&db_client().server_onboarding_keys, None, None)
      .await
      .context("Failed to query database for Server onboarding keys")
      .map_err(Into::into)
  }
}
