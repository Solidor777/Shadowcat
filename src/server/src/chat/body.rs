//! Chunk-to-segment composer shared by the send and edit ingest paths.
//!
//! `compose_message` turns `rolls::scan_body_capped`'s per-chunk grammar into
//! `Segment`s: a `Text` chunk sanitizes independently, an `Inline` chunk
//! executes a roll (`ScanMode::Execute` only -- refused under
//! `ScanMode::NoExecute`, since a roll's outcome is immutable once sent), a
//! `Button` chunk validates a formula without rolling it, and a `DocLink`
//! chunk passes its parsed target/label straight through. A body that scans
//! to exactly one `Text` chunk is the fast path: `sanitize(body)` runs once
//! over the WHOLE body, byte-identical to a body with no spans at all.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use uuid::Uuid;

use crate::data::document::Document;
use crate::data::repository::Repository;
use crate::data::DataError;

use super::settings::ChatContentPolicy;
use super::{host, resolve_dice_context, rolls, sanitize, ActorOwnerRef, Segment};

/// Whether an inline `[[formula]]` span executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanMode {
    /// Execute inline rolls (the send path).
    Execute,
    /// Refuse any inline roll span with `ComposeError::Inline` (the edit
    /// path -- a roll's outcome is immutable once sent).
    NoExecute,
}

/// Borrowed dependencies `compose_message` needs to resolve dice context and
/// the roll's actor binding, grouped the same way `LinkPreviewDeps` groups
/// its own borrowed bundle.
pub(crate) struct ComposeDeps<'a> {
    /// The document repository.
    pub repo: &'a dyn Repository,
    /// The sending/editing room's world.
    pub world_id: Uuid,
    /// The message's channel (selects the dice `ParseContext`).
    pub channel: &'a str,
    /// The message's validated actor attribution, if any.
    pub actor_owner: Option<&'a ActorOwnerRef>,
    /// The world's resolved chat content policy.
    pub policy: &'a ChatContentPolicy,
}

/// Why `compose_message` could not produce a segment list.
#[derive(Debug)]
pub(crate) enum ComposeError {
    /// A scan/roll failure from `rolls::scan_body_capped`/`rolls::execute_roll`/
    /// `rolls::validate_formula`.
    Roll(rolls::RollError),
    /// An inline `[[formula]]` span under `ScanMode::NoExecute`. Cannot occur
    /// under `ScanMode::Execute`.
    Inline,
    /// A repository error resolving dice context or the roll's host.
    Data(DataError),
}

/// Composes a message body into its segment list, per this module's doc
/// comment. Moved verbatim from `handle_send_message`'s Normal/Emote arm.
pub(crate) async fn compose_message(
    body: &str,
    deps: ComposeDeps<'_>,
    mode: ScanMode,
) -> Result<Vec<Segment>, ComposeError> {
    let chunks =
        rolls::scan_body_capped(body, rolls::MAX_INLINE_ROLLS).map_err(ComposeError::Roll)?;
    if let [rolls::BodyChunk::Text(_)] = chunks.as_slice() {
        return Ok(sanitize::sanitize(body, deps.policy));
    }
    let mut dice_ctx: Option<crate::dice::ParseContext> = None;
    let mut roll_host: Option<Option<Document>> = None;
    let mut segments = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        match chunk {
            rolls::BodyChunk::Text(t) => segments.extend(sanitize::sanitize(t, deps.policy)),
            rolls::BodyChunk::Inline(formula) => match mode {
                ScanMode::NoExecute => return Err(ComposeError::Inline),
                ScanMode::Execute => {
                    if dice_ctx.is_none() {
                        dice_ctx = Some(
                            resolve_dice_context(deps.repo, deps.world_id, deps.channel).await,
                        );
                    }
                    if roll_host.is_none() {
                        roll_host = Some(match deps.actor_owner {
                            Some(owner_ref) => host::host_for_actor_owner(deps.repo, owner_ref)
                                .await
                                .map_err(ComposeError::Data)?,
                            None => None,
                        });
                    }
                    let host_ref = roll_host.as_ref().expect("roll host computed").as_ref();
                    let (formula, outcome, spec, raw) =
                        rolls::execute_roll(formula, dice_ctx.unwrap(), host_ref)
                            .map_err(ComposeError::Roll)?;
                    segments.push(Segment::RollEmbed {
                        formula,
                        outcome,
                        roll_id: Uuid::new_v4(),
                        spec: Some(Box::new(spec)),
                        raw: Some(Box::new(raw)),
                        recalc_history: None,
                    });
                }
            },
            rolls::BodyChunk::Button { formula, label } => {
                if dice_ctx.is_none() {
                    dice_ctx =
                        Some(resolve_dice_context(deps.repo, deps.world_id, deps.channel).await);
                }
                // Stored/validated formula is trimmed -- the `roll:`/`|` split
                // leaves incidental whitespace (e.g. "[[roll: 1d20|Attack]]")
                // that must not survive into the button's stored formula.
                let formula = formula.trim();
                rolls::validate_formula(formula, dice_ctx.unwrap()).map_err(ComposeError::Roll)?;
                segments.push(Segment::RollButton {
                    formula: formula.to_string(),
                    label: label.map(|s| s.to_string()),
                });
            }
            rolls::BodyChunk::DocLink { target, label } => {
                segments.push(Segment::DocLink {
                    target,
                    label: label.to_string(),
                });
            }
        }
    }
    Ok(segments)
}

#[cfg(test)]
mod tests;
