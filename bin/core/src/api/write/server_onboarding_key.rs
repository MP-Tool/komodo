use anyhow::{Context, anyhow};
use database::mungos::mongodb::bson::doc;
use komodo_client::{
  api::write::{
    CreateServerOnboardingKey, CreateServerOnboardingKeyResponse,
    DeleteServerOnboardingKey, DeleteServerOnboardingKeyResponse,
  },
  entities::{
    komodo_timestamp, server_onboarding_key::ServerOnboardingKey,
  },
};
use noise::key::EncodedKeyPair;
use reqwest::StatusCode;
use resolver_api::Resolve;
use serror::{AddStatusCode, AddStatusCodeError};

use crate::{api::write::WriteArgs, state::db_client};

//

impl Resolve<WriteArgs> for CreateServerOnboardingKey {
  #[instrument(name = "CreateServerOnboardingKey", skip(self, admin))]
  async fn resolve(
    self,
    WriteArgs { user: admin }: &WriteArgs,
  ) -> serror::Result<CreateServerOnboardingKeyResponse> {
    if !admin.admin {
      return Err(
        anyhow!("This call is admin only")
          .status_code(StatusCode::FORBIDDEN),
      );
    }
    let keys = if let Some(private_key) = self.private_key {
      EncodedKeyPair::from_private_key(&private_key)?
    } else {
      EncodedKeyPair::generate()?
    };
    let creation_key = ServerOnboardingKey {
      public_key: keys.public.into_inner(),
      name: self.name,
      onboarded: Default::default(),
      created_at: komodo_timestamp(),
      expires: self.expires,
      default_tags: self.default_tags,
      default_config: self.default_config.into(),
    };
    let db = db_client();
    // Create the key
    db.server_onboarding_keys
      .insert_one(&creation_key)
      .await
      .context(
        "Failed to create Server onboarding key on database",
      )?;
    let created = db
      .server_onboarding_keys
      .find_one(doc! { "public_key": &creation_key.public_key })
      .await
      .context("Failed to query database for Server onboarding keys")?
      .context(
        "No Server onboarding key found on database after create",
      )?;
    Ok(CreateServerOnboardingKeyResponse {
      private_key: keys.private.into_inner(),
      created,
    })
  }
}

//

impl Resolve<WriteArgs> for DeleteServerOnboardingKey {
  #[instrument(name = "DeleteServerOnboardingKey", skip(admin))]
  async fn resolve(
    self,
    WriteArgs { user: admin }: &WriteArgs,
  ) -> serror::Result<DeleteServerOnboardingKeyResponse> {
    if !admin.admin {
      return Err(
        anyhow!("This call is admin only")
          .status_code(StatusCode::FORBIDDEN),
      );
    }
    let db = db_client();
    let query = doc! { "public_key": &self.public_key };
    let creation_key = db
      .server_onboarding_keys
      .find_one(query.clone())
      .await
      .context("Failed to query database for Server onboarding keys")?
      .context("Server onboarding key matching provided public key not found")
      .status_code(StatusCode::NOT_FOUND)?;
    db.server_onboarding_keys.delete_one(query).await.context(
      "Failed to delete Server onboarding key from database",
    )?;
    Ok(creation_key)
  }
}
