use clap::Parser;
use derive_empty_traits::EmptyTraits;
use resolver_api::Resolve;
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

use crate::entities::update::Update;

use super::KomodoExecuteRequest;

/// **Admin only.** Clears all repos from the Core repo cache.
/// Response: [Update]
#[typeshare]
#[derive(
  Debug,
  Clone,
  PartialEq,
  Serialize,
  Deserialize,
  Resolve,
  EmptyTraits,
  Parser,
)]
#[empty_traits(KomodoExecuteRequest)]
#[response(Update)]
#[error(serror::Error)]
pub struct ClearRepoCache {}

//

/// **Admin only.** Backs up the Komodo Core database to compressed jsonl files.
/// Response: [Update]
///
/// Mount a folder to `/backups`, and Core will use it to create
/// timestamped database dumps, which can be restored using
/// the Komodo CLI.
///
/// https://komo.do/docs/setup/backup
#[typeshare]
#[derive(
  Debug,
  Clone,
  PartialEq,
  Serialize,
  Deserialize,
  Resolve,
  EmptyTraits,
  Parser,
)]
#[empty_traits(KomodoExecuteRequest)]
#[response(Update)]
#[error(serror::Error)]
pub struct BackupCoreDatabase {}

//

/// **Admin only.** Trigger a global poll for image updates on Stacks and Deployments
/// with `poll_for_updates` or `auto_update` enabled.
/// Response: [Update]
///
/// 1. `docker compose pull` any Stacks / Deployments with `poll_for_updates` or `auto_update` enabled. This will pick up any available updates.
/// 2. Redeploy Stacks / Deployments that have updates found and 'auto_update' enabled.
#[typeshare]
#[derive(
  Debug,
  Clone,
  PartialEq,
  Serialize,
  Deserialize,
  Resolve,
  EmptyTraits,
  Parser,
)]
#[empty_traits(KomodoExecuteRequest)]
#[response(Update)]
#[error(serror::Error)]
pub struct GlobalAutoUpdate {}

//

/// **Admin only.** Rotates all connected Server keys.
/// Response: [Update]
#[typeshare]
#[derive(
  Debug,
  Clone,
  PartialEq,
  Serialize,
  Deserialize,
  Resolve,
  EmptyTraits,
  Parser,
)]
#[empty_traits(KomodoExecuteRequest)]
#[response(Update)]
#[error(serror::Error)]
pub struct RotateAllServerKeys {}
