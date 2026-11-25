//! This module resolves the `"bars"` derived channel: each readable token's nameplate string and
//! attribute-bar numbers, evaluated server-side for one recipient.
//!
//! Egress rule: the channel carries only the evaluated `{current, max}` numbers and the resolved
//! nameplate, never a bar's `Formula` source or the host document's `system` band, and a value is
//! included only when the recipient is entitled to it. Three decisions compose, each made once:
//! - whether the recipient receives the token at all (`SceneEcs::token_footprint_visible`, the
//!   gate the `"footprints"` channel applies to the same token and actor documents);
//! - whether the recipient is the token's effective owner or the GM
//!   (`Access::can_see` at the `OwnerOrGm` tier), which `BarDisplayMode::delivered_to` reads;
//! - what the recipient sees of the documents the values come from: the host document is run
//!   through `filter_properties` for the recipient before a formula reads it, so a leaf the
//!   recipient cannot read evaluates as absent and its bar is omitted.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::{engine_as, GeometrySource, SceneEcs};
use crate::combat::eval::eval_formula;
use crate::data::document::{Document, Visibility, WorldCapDefaults};
use crate::data::engine as eng;
use crate::data::membership::PermissionContext;
use crate::data::permission::{filter_properties, Access};

/// The whole `"bars"` derived-channel payload: every readable token with something to show,
/// grouped by scene. Keyed maps give a stable fingerprint (the egress loop's change detection
/// compares whole payloads).
///
/// # Examples
///
/// ```
/// use shadowcat::scene::bars::BarsPayload;
///
/// let payload = BarsPayload { scenes: Default::default() };
/// let json = serde_json::to_value(&payload).unwrap();
/// let round_tripped: BarsPayload = serde_json::from_value(json).unwrap();
/// assert!(round_tripped.scenes.is_empty());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct BarsPayload {
    /// One entry per scene holding at least one token the recipient receives bars for.
    pub scenes: BTreeMap<Uuid, SceneBars>,
}

/// One scene's slice of the `"bars"` channel.
///
/// # Examples
///
/// ```
/// use shadowcat::scene::bars::SceneBars;
///
/// let scene = SceneBars { tokens: Default::default() };
/// assert!(scene.tokens.is_empty());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct SceneBars {
    /// Tokens of the scene with a nameplate or at least one bar for this recipient.
    pub tokens: BTreeMap<Uuid, TokenBars>,
}

/// One token's resolved nameplate and bars for one recipient.
///
/// # Examples
///
/// ```
/// use shadowcat::scene::bars::TokenBars;
///
/// use shadowcat::data::engine::BarDisplayMode;
///
/// let token = TokenBars {
///     nameplate: Some("Goblin".to_string()),
///     nameplate_mode: Some(BarDisplayMode::Always),
///     bars: Vec::new(),
/// };
/// assert_eq!(token.nameplate.as_deref(), Some("Goblin"));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct TokenBars {
    /// The name this recipient sees for the token, resolved from the documents as this
    /// recipient receives them; `None` when the recipient receives no name even though the
    /// nameplate is delivered (the client supplies its own fallback label), and when the
    /// nameplate is not delivered at all.
    pub nameplate: Option<String>,
    /// The resolved nameplate mode when the nameplate is delivered to this recipient, `None`
    /// when it is not. The client draws a nameplate only when this is `Some`, and reveals it
    /// per the mode.
    pub nameplate_mode: Option<eng::BarDisplayMode>,
    /// The bars delivered to this recipient, in authored order.
    pub bars: Vec<ResolvedBar>,
}

/// One bar with its numbers already evaluated.
///
/// # Examples
///
/// ```
/// use shadowcat::scene::bars::ResolvedBar;
///
/// use shadowcat::data::engine::BarDisplayMode;
///
/// let bar = ResolvedBar {
///     label: "HP".to_string(),
///     current: 4.0,
///     max: 10.0,
///     color: "#cc2222".to_string(),
///     mode: BarDisplayMode::Always,
/// };
/// assert!(bar.current <= bar.max);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct ResolvedBar {
    /// The bar's label.
    pub label: String,
    /// The evaluated filled amount; finite.
    pub current: f64,
    /// The evaluated full amount; finite.
    pub max: f64,
    /// Fill colour, a css `#rrggbb` string.
    pub color: String,
    /// The bar's authored mode; the client reveals a delivered bar per this mode (`Always`
    /// and `Owner` draw continuously, `Hover` on hover or selection).
    pub mode: eng::BarDisplayMode,
}

/// The nameplate-relevant text of an actor-shaped document: its envelope `name` and the
/// `displayName` in its `engine` band, read off the JSON because redaction can strip either
/// without leaving a deserializable `ActorEngine`.
fn name_and_display(actor: &Document) -> (Option<String>, Option<String>) {
    let display = actor
        .engine
        .as_ref()
        .and_then(|e| e.get("displayName"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    (actor.name.clone(), display)
}

impl SceneEcs {
    /// The resolved `"bars"` derived-channel payload for `ctx`. A token contributes only when
    /// `ctx` may see its scene and `token_footprint_visible` holds, and only when
    /// `token_bars` finds something delivered to `ctx`.
    pub(crate) fn resolved_bars(
        &self,
        ctx: &PermissionContext,
        world_defaults: &WorldCapDefaults,
    ) -> BarsPayload {
        let mut scenes: BTreeMap<Uuid, SceneBars> = BTreeMap::new();
        for e in self.world.query::<&super::SceneEntity>().iter() {
            let doc = &e.doc;
            if doc.doc_type != "token" {
                continue;
            }
            let Some(scene) = doc.parent_id else {
                continue;
            };
            if !self.scene_visible_to(ctx, world_defaults, scene)
                || !self.token_footprint_visible(ctx, world_defaults, doc)
            {
                continue;
            }
            if let Some(entry) = self.token_bars(ctx, world_defaults, doc) {
                scenes
                    .entry(scene)
                    .or_insert_with(|| SceneBars {
                        tokens: BTreeMap::new(),
                    })
                    .tokens
                    .insert(doc.id, entry);
            }
        }
        BarsPayload { scenes }
    }

    /// One token's nameplate and bars for `ctx`, or `None` when neither is delivered. The
    /// configuration resolves like `token_light_emission`: a linked token's `overrides` replace
    /// the shared actor's `bars` / `nameplate_mode` wholesale, a dangling link resolves to
    /// nothing, and an instanced token reads its embedded actor copy and ignores `overrides`.
    fn token_bars(
        &self,
        ctx: &PermissionContext,
        world_defaults: &WorldCapDefaults,
        token: &Document,
    ) -> Option<TokenBars> {
        let token_access = self.ctx_access(ctx, world_defaults, token);
        // The effective owner and the GM: the only recipients `Hover`/`Owner` values reach.
        let trusted = token_access.can_see(&Visibility::OwnerOrGm);
        let token_eng = self.engine_as_cached::<eng::TokenEngine>(token.id, token)?;
        let source = self.token_geometry_source(token)?;
        let (host, actor_access, linked) = match source {
            GeometrySource::Linked(actor) => {
                (actor, self.ctx_access(ctx, world_defaults, actor), true)
            }
            GeometrySource::Embedded(child) => (child, token_access.clone(), false),
        };
        let actor_eng = if linked {
            self.engine_as_cached::<eng::ActorEngine>(host.id, host)?
        } else {
            engine_as::<eng::ActorEngine>(host)?
        };
        let overrides = token_eng.overrides.as_ref().filter(|_| linked);
        let bar_list = overrides
            .and_then(|o| o.bars.clone())
            .or(actor_eng.bars)
            .unwrap_or_default();
        let nameplate_mode = overrides
            .and_then(|o| o.nameplate_mode)
            .or(actor_eng.nameplate_mode)
            .unwrap_or_default();

        // The recipient's own view of the token: identity redaction (`identity_hidden`) has
        // already removed what it hides, including the link a linked token resolves its actor by.
        let seen_token = filter_properties(token, &token_access).ok()?;

        let nameplate_delivered = nameplate_mode.delivered_to(trusted);
        let nameplate = nameplate_delivered
            .then(|| self.nameplate_for(&seen_token, host, &actor_access, linked))
            .flatten();

        // A linked token whose `actor_id` the recipient does not receive (a secret identity)
        // gets no bars: the actor's numbers would re-join the token to that actor for any
        // recipient who can read it. `nameplate_for` applies the same rule to the name.
        let link_received = !linked
            || engine_as::<eng::TokenEngine>(&seen_token)
                .and_then(|t| t.actor_id)
                .is_some();
        let mut bars = Vec::new();
        if link_received && bar_list.iter().any(|b| b.mode.delivered_to(trusted)) {
            let seen_host = if linked {
                filter_properties(host, &actor_access).ok()?
            } else {
                seen_token.embedded.get("actor")?.first()?.clone()
            };
            for bar in bar_list.iter().filter(|b| b.mode.delivered_to(trusted)) {
                let (Ok(current), Ok(max)) = (
                    eval_formula(&bar.current, Some(&seen_host)),
                    eval_formula(&bar.max, Some(&seen_host)),
                ) else {
                    continue; // one failing bar never costs the token its others
                };
                if !current.is_finite() || !max.is_finite() {
                    continue;
                }
                bars.push(ResolvedBar {
                    label: bar.label.clone(),
                    current,
                    max,
                    color: bar.color.clone(),
                    mode: bar.mode,
                });
            }
        }
        (nameplate_delivered || !bars.is_empty()).then_some(TokenBars {
            nameplate,
            nameplate_mode: nameplate_delivered.then_some(nameplate_mode),
            bars,
        })
    }

    /// The name `seen_token`'s recipient is entitled to, resolved in the client's own order
    /// (`resolveTokenActor`/`actorDisplayName`): the override name, else the actor's name, else
    /// the actor's `displayName`; an empty string counts as absent. A linked token whose
    /// `actor_id` the recipient does not receive (a secret identity) resolves nothing, and every
    /// document read here is the recipient's own redacted copy, so this adds no second name
    /// decision to the redaction that already ran.
    fn nameplate_for(
        &self,
        seen_token: &Document,
        host: &Document,
        actor_access: &Access,
        linked: bool,
    ) -> Option<String> {
        let (name, display) = if linked {
            let link_received = engine_as::<eng::TokenEngine>(seen_token)
                .and_then(|t| t.actor_id)
                .is_some();
            if !link_received {
                return None;
            }
            let seen_actor = filter_properties(host, actor_access).ok()?;
            let override_name = engine_as::<eng::TokenEngine>(seen_token)
                .and_then(|t| t.overrides)
                .and_then(|o| o.name);
            let (actor_name, display) = name_and_display(&seen_actor);
            (override_name.or(actor_name), display)
        } else {
            name_and_display(seen_token.embedded.get("actor")?.first()?)
        };
        name.filter(|n| !n.is_empty())
            .or(display.filter(|d| !d.is_empty()))
    }
}
