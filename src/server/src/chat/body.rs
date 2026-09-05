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

/// Composes a message body into its segment list plus every inline image
/// source `sanitize` collected across every `Text` chunk (see
/// `Sanitized.image_urls`'s doc -- the fast path's single whole-body sanitize
/// call and the per-chunk loop's per-`Text`-chunk calls both contribute),
/// per this module's doc comment. Moved verbatim from `handle_send_message`'s
/// Normal/Emote arm.
pub(crate) async fn compose_message(
    body: &str,
    deps: ComposeDeps<'_>,
    mode: ScanMode,
) -> Result<(Vec<Segment>, Vec<sanitize::ImageSource>), ComposeError> {
    let chunks =
        rolls::scan_body_capped(body, rolls::MAX_INLINE_ROLLS).map_err(ComposeError::Roll)?;
    if let [rolls::BodyChunk::Text(_)] = chunks.as_slice() {
        let sanitized = sanitize::sanitize(body, deps.policy);
        return Ok((sanitized.segments, sanitized.image_urls));
    }
    let mut dice_ctx: Option<crate::dice::ParseContext> = None;
    let mut roll_host: Option<Option<Document>> = None;
    let mut segments = Vec::with_capacity(chunks.len());
    let mut image_urls: Vec<sanitize::ImageSource> = Vec::new();
    for chunk in chunks {
        match chunk {
            rolls::BodyChunk::Text(t) => {
                let sanitized = sanitize::sanitize(t, deps.policy);
                segments.extend(sanitized.segments);
                // Each `sanitize` call dedups WITHIN its own chunk only; a
                // URL repeated across two different Text chunks (e.g. the
                // same image before and after an inline roll) must not queue
                // two identical enrichment jobs / `Segment::Image`s.
                for src in sanitized.image_urls {
                    if !image_urls
                        .iter()
                        .any(|s: &sanitize::ImageSource| s.url == src.url)
                    {
                        image_urls.push(src);
                    }
                }
            }
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
            rolls::BodyChunk::Image { asset_id, alt } => {
                if !deps.policy.images() {
                    return Err(ComposeError::Roll(rolls::RollError::ImagesDisabled));
                }
                let alt = alt.unwrap_or("");
                if alt.chars().count() > super::MAX_IMAGE_ALT_CHARS {
                    return Err(ComposeError::Roll(rolls::RollError::AltTooLong));
                }
                let asset = deps
                    .repo
                    .get_asset(asset_id)
                    .await
                    .map_err(ComposeError::Data)?;
                let in_world = matches!(&asset, Some(a) if a.world_id == deps.world_id);
                if !in_world {
                    return Err(ComposeError::Roll(rolls::RollError::UnknownAsset));
                }
                segments.push(Segment::Image {
                    asset_id,
                    alt: alt.to_string(),
                });
            }
        }
    }
    Ok((segments, image_urls))
}

/// Composes a body into segments synchronously, with no repository or
/// network access: a `Text` chunk sanitizes under `policy`; an `Inline` OR
/// `Button` chunk validates (never executes, since a static body has no
/// per-send host to resolve a reference against) and becomes an unexecuted
/// `Segment::RollButton` -- an inline `[[formula]]` span is NOT a
/// `Segment::RollEmbed` here, because a static body composes once at
/// document-write time and must never roll dice as a side effect of that
/// write; a `DocLink` chunk passes its parsed target/label straight through;
/// an `Image` chunk becomes a `Segment::Image` with NO existence check -- the
/// referenced asset id is not confirmed to exist, because no outbound fetch
/// or repository lookup exists on the document-write path this composes for.
/// `Sanitized.image_urls` is deliberately DISCARDED for the same reason: a
/// markdown image in a document body renders as its alt text (the sanitizer
/// already replaced the image event with it), and only an explicit
/// `[[asset:...]]` span produces an image. Used for a document body derived
/// synchronously at ingress (a note's `body`, a table row's text), never for
/// a chat message's own asynchronous send/edit pipeline (`compose_message`).
pub(crate) fn compose_static(
    body: &str,
    policy: &ChatContentPolicy,
    max_spans: usize,
) -> Result<Vec<Segment>, rolls::RollError> {
    let chunks = rolls::scan_body_capped(body, max_spans)?;
    if let [rolls::BodyChunk::Text(_)] = chunks.as_slice() {
        return Ok(sanitize::sanitize(body, policy).segments);
    }
    let mut segments = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        match chunk {
            rolls::BodyChunk::Text(t) => {
                segments.extend(sanitize::sanitize(t, policy).segments);
            }
            rolls::BodyChunk::Inline(formula) => {
                rolls::validate_formula(formula, crate::dice::ParseContext::default())?;
                segments.push(Segment::RollButton {
                    formula: formula.to_string(),
                    label: None,
                });
            }
            rolls::BodyChunk::Button { formula, label } => {
                let formula = formula.trim();
                rolls::validate_formula(formula, crate::dice::ParseContext::default())?;
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
            rolls::BodyChunk::Image { asset_id, alt } => {
                let alt = alt.unwrap_or("");
                if alt.chars().count() > super::MAX_IMAGE_ALT_CHARS {
                    return Err(rolls::RollError::AltTooLong);
                }
                segments.push(Segment::Image {
                    asset_id,
                    alt: alt.to_string(),
                });
            }
        }
    }
    Ok(segments)
}

#[cfg(test)]
mod tests;
