//! The roll host for a message's actor binding: which document a roll's
//! references resolve against when the send carries an `ActorOwnerRef`. The
//! precedence rule — a token's embedded actor copy beats its linked actor —
//! is the one `combat::eval::formula_host` declares for combatants; the two
//! share the `embedded_actor_copy` extraction so the token→copy step cannot
//! fork between the combatant walk and the roll binding.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::data::document::{embedded_actor_copy, Document};
use crate::data::engine::TokenEngine;
use crate::data::repository::Repository;
use crate::data::DataError;

use super::ActorOwnerRef;

/// The document a roll's references resolve against for this actor binding:
/// the actor itself for an `Actor` ref; for a `TokenInstance` ref, the token's
/// embedded actor copy, else its linked actor. `None` when the named document
/// (or the fallback) is absent — the caller resolves through the no-host
/// resolver, so a referencing roll then fails `unknown-ref` rather than
/// reading a guessed host.
///
/// CONTRACT: the caller has already validated the ref at ingest
/// (`handle_send_message`'s attribution gate: existence, doc type, world
/// pinning, ownership). This function re-reads the documents and performs no
/// authorization of its own — it answers "which document's `system` band do
/// this roll's references read", nothing more.
pub(crate) async fn host_for_actor_owner(
    repo: &dyn Repository,
    owner_ref: &ActorOwnerRef,
) -> Result<Option<Document>, DataError> {
    match owner_ref {
        ActorOwnerRef::Actor { actor_id } => repo.get_document(*actor_id).await,
        ActorOwnerRef::TokenInstance { token_id } => {
            let Some(token) = repo.get_document(*token_id).await? else {
                return Ok(None);
            };
            if let Some(copy) = embedded_actor_copy(&token) {
                return Ok(Some(copy.clone()));
            }
            let engine: TokenEngine =
                serde_json::from_value(token.engine.clone().unwrap_or_default())
                    .map_err(|e| DataError::BadEngine(format!("token: {e}")))?;
            match engine.actor_id {
                Some(actor_id) => repo.get_document(actor_id).await,
                None => Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests;
