//! Chunk-to-segment composer shared by the send and edit ingest paths.
//!
//! `compose_message` turns `rolls::scan_body_capped`'s per-chunk grammar into
//! `Segment`s: a `Text` chunk sanitizes independently, an `Inline` chunk
//! executes a roll (`ScanMode::Execute` only -- refused under
//! `ScanMode::NoExecute`, since a roll's outcome is immutable once sent), a
//! `Button` chunk validates a formula without rolling it, a `DocLink`
//! chunk passes its parsed target/label straight through, and a `Secret`
//! chunk composes its inner body recursively under the visibility its spec
//! names. A body that scans to exactly one `Text` chunk is the fast path:
//! `sanitize(body)` runs once over the WHOLE body, byte-identical to a body
//! with no spans at all.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::future::Future;
use std::pin::Pin;

use uuid::Uuid;

use crate::data::document::{Document, Visibility};
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
    /// The message's author. A `[[secret:@name|...]]` span lists the author among its
    /// recipients, so a writer always reads what they wrote.
    pub author: Uuid,
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

/// A boxed, recursive composer step. `Composer::compose_chunks` awaits itself for a secret's
/// inner body, so its future has to be boxed.
type ComposeFuture<'f> =
    Pin<Box<dyn Future<Output = Result<Vec<Segment>, ComposeError>> + Send + 'f>>;

/// State one `compose_message` call threads through every (possibly nested) body it composes.
struct Composer<'a, 'd> {
    /// The call's dependencies.
    deps: &'d ComposeDeps<'a>,
    /// Whether an inline roll executes.
    mode: ScanMode,
    /// The dice parse context, resolved on first use.
    dice_ctx: Option<crate::dice::ParseContext>,
    /// The roll's host document, resolved on first use.
    roll_host: Option<Option<Document>>,
    /// Every inline image source `sanitize` collected, deduplicated by URL across all chunks of
    /// all nested bodies.
    image_urls: Vec<sanitize::ImageSource>,
    /// Non-text spans still allowed across the whole body, nested secrets included. A scan
    /// charges its spans here before any of them is composed, so the total never exceeds
    /// `rolls::MAX_INLINE_ROLLS`.
    spans_left: usize,
}

impl Composer<'_, '_> {
    /// Sanitizes `text` and folds its image sources into `image_urls`.
    ///
    /// Each `sanitize` call dedups WITHIN its own text only; a URL repeated across two
    /// different Text chunks (e.g. the same image before and after an inline roll) must not
    /// queue two identical enrichment jobs / `Segment::Image`s.
    fn push_text(&mut self, text: &str, segments: &mut Vec<Segment>) {
        let sanitized = sanitize::sanitize(text, self.deps.policy);
        segments.extend(sanitized.segments);
        for src in sanitized.image_urls {
            if !self
                .image_urls
                .iter()
                .any(|s: &sanitize::ImageSource| s.url == src.url)
            {
                self.image_urls.push(src);
            }
        }
    }

    /// The visibility a secret span's `spec` names. `gm` is `GmOnly`; a `@name` list resolves
    /// each name against the world's members (fail-closed on an unknown name) and appends the
    /// author, deduplicated, since `Visibility::Recipients` matches by exact membership.
    async fn secret_visibility(&self, spec: &str) -> Result<Visibility, ComposeError> {
        match rolls::parse_secret_spec(spec).map_err(ComposeError::Roll)? {
            rolls::SecretSpec::Gm => Ok(Visibility::GmOnly),
            rolls::SecretSpec::Users(names) => {
                // Capped BEFORE the first lookup: resolving first would run one database
                // round-trip per attacker-supplied name.
                if names.len() > super::MAX_WHISPER_RECIPIENTS {
                    return Err(ComposeError::Roll(
                        rolls::RollError::TooManySecretRecipients,
                    ));
                }
                let mut recipients = vec![self.deps.author];
                for name in names {
                    let id = self
                        .deps
                        .repo
                        .member_id_by_username(self.deps.world_id, name)
                        .await
                        .map_err(ComposeError::Data)?
                        .ok_or(ComposeError::Roll(rolls::RollError::UnknownSecretRecipient))?;
                    if !recipients.contains(&id) {
                        recipients.push(id);
                    }
                }
                Ok(Visibility::Recipients { recipients })
            }
        }
    }

    /// Composes `body` (the whole message, or a secret's inner body at `depth`) into segments.
    fn compose_chunks<'f>(&'f mut self, body: &'f str, depth: usize) -> ComposeFuture<'f> {
        Box::pin(async move {
            let chunks =
                rolls::scan_body_capped(body, self.spans_left).map_err(ComposeError::Roll)?;
            let mut segments = Vec::with_capacity(chunks.len());
            if let [rolls::BodyChunk::Text(_)] = chunks.as_slice() {
                self.push_text(body, &mut segments);
                return Ok(segments);
            }
            self.spans_left -= chunks
                .iter()
                .filter(|c| !matches!(c, rolls::BodyChunk::Text(_)))
                .count();
            for chunk in chunks {
                match chunk {
                    rolls::BodyChunk::Text(t) => self.push_text(t, &mut segments),
                    rolls::BodyChunk::Inline(formula) => match self.mode {
                        ScanMode::NoExecute => return Err(ComposeError::Inline),
                        ScanMode::Execute => {
                            if self.dice_ctx.is_none() {
                                self.dice_ctx = Some(
                                    resolve_dice_context(
                                        self.deps.repo,
                                        self.deps.world_id,
                                        self.deps.channel,
                                    )
                                    .await,
                                );
                            }
                            if self.roll_host.is_none() {
                                self.roll_host = Some(match self.deps.actor_owner {
                                    Some(owner_ref) => {
                                        host::host_for_actor_owner(self.deps.repo, owner_ref)
                                            .await
                                            .map_err(ComposeError::Data)?
                                    }
                                    None => None,
                                });
                            }
                            let host_ref = self
                                .roll_host
                                .as_ref()
                                .expect("roll host computed")
                                .as_ref();
                            let (formula, outcome, spec, raw) = rolls::execute_roll(
                                formula,
                                self.dice_ctx.expect("dice context computed"),
                                host_ref,
                            )
                            .map_err(ComposeError::Roll)?;
                            segments.push(Segment::RollEmbed {
                                formula,
                                outcome,
                                roll_id: Uuid::new_v4(),
                                spec: Some(Box::new(spec)),
                                raw: Some(Box::new(raw)),
                                recalc_history: None,
                                flavor: None,
                            });
                        }
                    },
                    rolls::BodyChunk::Button {
                        formula,
                        label,
                        flavor,
                    } => {
                        if self.dice_ctx.is_none() {
                            self.dice_ctx = Some(
                                resolve_dice_context(
                                    self.deps.repo,
                                    self.deps.world_id,
                                    self.deps.channel,
                                )
                                .await,
                            );
                        }
                        // Stored/validated formula is trimmed -- the `roll:`/`|` split
                        // leaves incidental whitespace (e.g. "[[roll: 1d20|Attack]]")
                        // that must not survive into the button's stored formula.
                        let formula = formula.trim();
                        rolls::validate_formula(
                            formula,
                            self.dice_ctx.expect("dice context computed"),
                        )
                        .map_err(ComposeError::Roll)?;
                        segments.push(Segment::RollButton {
                            formula: formula.to_string(),
                            label: label.map(|s| s.to_string()),
                            flavor: flavor.map(|s| s.to_string()),
                        });
                    }
                    rolls::BodyChunk::DocLink { target, label } => {
                        segments.push(Segment::DocLink {
                            target,
                            label: label.map(|s| s.to_string()),
                        });
                    }
                    rolls::BodyChunk::Image { asset_id, alt } => {
                        if !self.deps.policy.images() {
                            return Err(ComposeError::Roll(rolls::RollError::ImagesDisabled));
                        }
                        let alt = alt.unwrap_or("");
                        if alt.chars().count() > super::MAX_IMAGE_ALT_CHARS {
                            return Err(ComposeError::Roll(rolls::RollError::AltTooLong));
                        }
                        let asset = self
                            .deps
                            .repo
                            .get_asset(asset_id)
                            .await
                            .map_err(ComposeError::Data)?;
                        let in_world =
                            matches!(&asset, Some(a) if a.world_id == self.deps.world_id);
                        if !in_world {
                            return Err(ComposeError::Roll(rolls::RollError::UnknownAsset));
                        }
                        segments.push(Segment::Image {
                            asset_id,
                            alt: alt.to_string(),
                        });
                    }
                    rolls::BodyChunk::Secret { spec, inner } => {
                        if depth + 1 > rolls::MAX_SECRET_DEPTH {
                            return Err(ComposeError::Roll(rolls::RollError::SecretTooDeep));
                        }
                        if inner.trim().is_empty() {
                            return Err(ComposeError::Roll(rolls::RollError::MalformedSecret));
                        }
                        let visibility = self.secret_visibility(spec).await?;
                        let content = self.compose_chunks(inner, depth + 1).await?;
                        segments.push(Segment::Secret {
                            visibility,
                            content,
                        });
                    }
                }
            }
            Ok(segments)
        })
    }
}

/// Composes a message body into its segment list plus every inline image
/// source `sanitize` collected across every `Text` chunk (see
/// `Sanitized.image_urls`'s doc -- the fast path's single whole-body sanitize
/// call and the per-chunk loop's per-`Text`-chunk calls both contribute),
/// per this module's doc comment. A `[[secret:...]]` span's inner body is composed
/// recursively under `rolls::MAX_SECRET_DEPTH`, sharing the one
/// `rolls::MAX_INLINE_ROLLS` span budget with the outer body.
pub(crate) async fn compose_message(
    body: &str,
    deps: ComposeDeps<'_>,
    mode: ScanMode,
) -> Result<(Vec<Segment>, Vec<sanitize::ImageSource>), ComposeError> {
    let mut composer = Composer {
        deps: &deps,
        mode,
        dice_ctx: None,
        roll_host: None,
        image_urls: Vec::new(),
        spans_left: rolls::MAX_INLINE_ROLLS,
    };
    let segments = composer.compose_chunks(body, 0).await?;
    Ok((segments, composer.image_urls))
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
/// A `[[secret:gm|...]]` span composes its inner body recursively into a
/// `Segment::Secret` under `Visibility::GmOnly`; a `@name` spec is refused
/// (`RollError::SecretRequiresGm`), because resolving a username needs a
/// repository this function does not have. Secret nesting is bounded by
/// `rolls::MAX_SECRET_DEPTH` and every span, nested ones included, counts
/// against `max_spans`.
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
    let mut spans_left = max_spans;
    compose_static_at(body, policy, &mut spans_left, 0)
}

/// `compose_static`'s recursive step: composes `body` at secret nesting `depth`, charging its
/// spans to the shared `spans_left` budget.
fn compose_static_at(
    body: &str,
    policy: &ChatContentPolicy,
    spans_left: &mut usize,
    depth: usize,
) -> Result<Vec<Segment>, rolls::RollError> {
    let chunks = rolls::scan_body_capped(body, *spans_left)?;
    if let [rolls::BodyChunk::Text(_)] = chunks.as_slice() {
        return Ok(sanitize::sanitize(body, policy).segments);
    }
    *spans_left -= chunks
        .iter()
        .filter(|c| !matches!(c, rolls::BodyChunk::Text(_)))
        .count();
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
                    flavor: None,
                });
            }
            rolls::BodyChunk::Button {
                formula,
                label,
                flavor,
            } => {
                let formula = formula.trim();
                rolls::validate_formula(formula, crate::dice::ParseContext::default())?;
                segments.push(Segment::RollButton {
                    formula: formula.to_string(),
                    label: label.map(|s| s.to_string()),
                    flavor: flavor.map(|s| s.to_string()),
                });
            }
            rolls::BodyChunk::DocLink { target, label } => {
                segments.push(Segment::DocLink {
                    target,
                    label: label.map(|s| s.to_string()),
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
            rolls::BodyChunk::Secret { spec, inner } => {
                if depth + 1 > rolls::MAX_SECRET_DEPTH {
                    return Err(rolls::RollError::SecretTooDeep);
                }
                if inner.trim().is_empty() {
                    return Err(rolls::RollError::MalformedSecret);
                }
                match rolls::parse_secret_spec(spec)? {
                    rolls::SecretSpec::Gm => {}
                    rolls::SecretSpec::Users(_) => return Err(rolls::RollError::SecretRequiresGm),
                }
                let content = compose_static_at(inner, policy, spans_left, depth + 1)?;
                segments.push(Segment::Secret {
                    visibility: Visibility::GmOnly,
                    content,
                });
            }
        }
    }
    Ok(segments)
}

#[cfg(test)]
mod tests;
