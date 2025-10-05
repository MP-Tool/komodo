use anyhow::{Context, anyhow};
use database::mungos::mongodb::bson::{Document, doc};
use komodo_client::{
  api::write::{
    CreateOnboardingKey, CreateOnboardingKeyResponse,
    DeleteOnboardingKey, DeleteOnboardingKeyResponse,
    UpdateOnboardingKey, UpdateOnboardingKeyResponse,
  },
  entities::{komodo_timestamp, onboarding_key::OnboardingKey},
};
use noise::key::EncodedKeyPair;
use reqwest::StatusCode;
use resolver_api::Resolve;
use serror::{AddStatusCode, AddStatusCodeError};

use crate::{api::write::WriteArgs, state::db_client};

//

impl Resolve<WriteArgs> for CreateOnboardingKey {
  #[instrument(name = "CreateServerOnboardingKey", skip(self, admin))]
  async fn resolve(
    self,
    WriteArgs { user: admin }: &WriteArgs,
  ) -> serror::Result<CreateOnboardingKeyResponse> {
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
    let onboarding_key = OnboardingKey {
      public_key: keys.public.into_inner(),
      name: self.name,
      enabled: true,
      onboarded: Default::default(),
      created_at: komodo_timestamp(),
      expires: self.expires,
      tags: self.tags,
      copy_server: self.copy_server,
    };
    let db = db_client();
    // Create the key
    db.onboarding_keys
      .insert_one(&onboarding_key)
      .await
      .context(
        "Failed to create Server onboarding key on database",
      )?;
    let created = db
      .onboarding_keys
      .find_one(doc! { "public_key": &onboarding_key.public_key })
      .await
      .context("Failed to query database for Server onboarding keys")?
      .context(
        "No Server onboarding key found on database after create",
      )?;
    Ok(CreateOnboardingKeyResponse {
      private_key: keys.private.into_inner(),
      created,
    })
  }
}

//

impl Resolve<WriteArgs> for UpdateOnboardingKey {
  async fn resolve(
    self,
    WriteArgs { user: admin }: &WriteArgs,
  ) -> serror::Result<UpdateOnboardingKeyResponse> {
    if !admin.admin {
      return Err(
        anyhow!("This call is admin only")
          .status_code(StatusCode::FORBIDDEN),
      );
    }

    let query = doc! { "public_key": &self.public_key };

    // No changes
    if self.enabled.is_none()
      && self.name.is_none()
      && self.expires.is_none()
      && self.tags.is_none()
      && self.copy_server.is_none()
    {
      return db_client()
        .onboarding_keys
        .find_one(query)
        .await
        .context("Failed to query database for onboarding key")?
        .context("No matching onboarding key found")
        .status_code(StatusCode::NOT_FOUND);
    }

    let mut update = Document::new();

    if let Some(enabled) = self.enabled {
      update.insert("enabled", enabled);
    }

    if let Some(name) = self.name {
      update.insert("name", name);
    }

    if let Some(expires) = self.expires {
      update.insert("expires", expires);
    }

    if let Some(tags) = self.tags {
      update.insert("tags", tags);
    }

    if let Some(copy_server) = self.copy_server {
      update.insert("copy_server", copy_server);
    }

    db_client()
      .onboarding_keys
      .update_one(query.clone(), doc! { "$set": update })
      .await
      .context("Failed to update onboarding key on database")?;

    db_client()
      .onboarding_keys
      .find_one(query)
      .await
      .context("Failed to query database for onboarding key")?
      .context("No matching onboarding key found")
      .status_code(StatusCode::NOT_FOUND)
  }
}

//

impl Resolve<WriteArgs> for DeleteOnboardingKey {
  #[instrument(name = "DeleteServerOnboardingKey", skip(admin))]
  async fn resolve(
    self,
    WriteArgs { user: admin }: &WriteArgs,
  ) -> serror::Result<DeleteOnboardingKeyResponse> {
    if !admin.admin {
      return Err(
        anyhow!("This call is admin only")
          .status_code(StatusCode::FORBIDDEN),
      );
    }
    let db = db_client();
    let query = doc! { "public_key": &self.public_key };
    let creation_key = db
      .onboarding_keys
      .find_one(query.clone())
      .await
      .context("Failed to query database for Server onboarding keys")?
      .context("Server onboarding key matching provided public key not found")
      .status_code(StatusCode::NOT_FOUND)?;
    db.onboarding_keys.delete_one(query).await.context(
      "Failed to delete Server onboarding key from database",
    )?;
    Ok(creation_key)
  }
}
