//! Effect discovery and clock-driven mutation: which effects belong to a
//! combatant, and how a boundary tick or a lifecycle-policy expiry writes
//! them. Lifecycle flags and duration amounts are evaluated server-side at
//! the moment of use (`eval::lifecycle_flags`/`eval::duration_amount`) over
//! the document hosting the effect; an evaluation failure skips that one
//! effect and is reported to the caller — never guessed around, and never a
//! stopped clock.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::{HashMap, HashSet};

use serde_json::json;
use uuid::Uuid;

use crate::data::command::{FieldChange, Operation};
use crate::data::document::Document;
use crate::data::engine::combat::{
    CombatantKind, DurationUnit, EffectEngine, EffectLifecycleDefaults, ExpiryPoint,
};
use crate::data::permission::TOKEN_DOC_TYPE;

use super::eval::{self, LifecycleFlags};
use super::ops::set_engine;
use super::{CombatError, CombatSnapshot, Combatant};

/// One effect document reachable from a combatant, located by its host
/// document and a JSON pointer to the effect within it (e.g.
/// `/embedded/effect/0`, `/embedded/item/0/embedded/effect/0`).
pub struct EffectRef {
    /// The document that embeds the effect (an actor or a token).
    pub host: Uuid,
    /// JSON pointer of the effect document inside `host`.
    pub path: String,
    /// The effect's engine band, as read from `host` at collection time.
    pub engine: EffectEngine,
}

/// Which anchor values a walk admits. The two axes an effect carries are
/// independent: its HOST owns where it physically lives, its
/// `duration.anchor` owns whose clock moves it. `OwnHost` is the walk of a
/// combatant's own host (an unanchored effect belongs to whoever hosts it,
/// and an explicitly self-anchored one too); `AnchoredOnly` is the walk of
/// every OTHER host, where nothing but an explicit anchor naming this
/// combatant may be claimed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AnchorScope {
    /// Admit `anchor == None` as well as `anchor == Some(combatant_id)`.
    OwnHost,
    /// Admit only `anchor == Some(combatant_id)`.
    AnchoredOnly,
}

/// Walks one embedded `effect` collection at `prefix` (e.g. `/embedded/effect`),
/// pushing every entry whose `transfer` flag is `include_all || engine.transfer`
/// and whose anchor `scope` admits for `combatant_id`.
fn walk_effect_collection(
    parent: &Document,
    host: Uuid,
    prefix: &str,
    combatant_id: Uuid,
    scope: AnchorScope,
    include_all: bool,
    out: &mut Vec<EffectRef>,
) {
    let Some(effects) = parent.embedded.get("effect") else {
        return;
    };
    for (i, doc) in effects.iter().enumerate() {
        let Some(raw) = doc.engine.clone() else {
            continue;
        };
        let Ok(engine) = serde_json::from_value::<EffectEngine>(raw) else {
            continue;
        };
        if !include_all && !engine.transfer {
            continue;
        }
        let anchored = match engine.duration.as_ref().and_then(|d| d.anchor) {
            None => scope == AnchorScope::OwnHost,
            Some(a) => a == combatant_id,
        };
        if !anchored {
            continue;
        }
        out.push(EffectRef {
            host,
            path: format!("{prefix}/{i}"),
            engine,
        });
    }
}

/// Walks the `effect` and item-embedded `effect` collections directly on
/// `host` (an actor or a token's embedded actor) under `scope`. An
/// item-embedded effect still requires its own `transfer` flag under either
/// scope: `transfer` decides whether the effect leaves the item at all,
/// which is a separate question from whose clock it is on.
fn walk_host(
    host_doc: &Document,
    host_id: Uuid,
    prefix: &str,
    combatant_id: Uuid,
    scope: AnchorScope,
    out: &mut Vec<EffectRef>,
) {
    walk_effect_collection(
        host_doc,
        host_id,
        &format!("{prefix}/embedded/effect"),
        combatant_id,
        scope,
        true,
        out,
    );
    if let Some(items) = host_doc.embedded.get("item") {
        for (j, item) in items.iter().enumerate() {
            walk_effect_collection(
                item,
                host_id,
                &format!("{prefix}/embedded/item/{j}/embedded/effect"),
                combatant_id,
                scope,
                false,
                out,
            );
        }
    }
}

/// Walks whichever effect collections `host_doc` exposes, under `scope`: an
/// actor document directly, a token document through its embedded actor
/// copy (`/embedded/actor/0`) — the same two shapes `collect_effects`
/// resolves for a combatant's own `actor_id`/`token_id`, applied to a host
/// reached by anchor rather than by ownership. A token with no embedded
/// actor exposes nothing.
fn walk_any_host(
    host_doc: &Document,
    host_id: Uuid,
    combatant_id: Uuid,
    scope: AnchorScope,
    out: &mut Vec<EffectRef>,
) {
    if host_doc.doc_type == TOKEN_DOC_TYPE {
        if let Some(actor) = host_doc.embedded.get("actor").and_then(|v| v.first()) {
            walk_host(
                actor,
                host_id,
                "/embedded/actor/0",
                combatant_id,
                scope,
                out,
            );
        }
    } else {
        walk_host(host_doc, host_id, "", combatant_id, scope, out);
    }
}

/// Collects every effect this combatant's clock moves, along BOTH axes the
/// shape defines:
/// - on its OWN host(s) — its linked actor and/or its token's embedded
///   actor — every effect that is unanchored (`duration.anchor: None`, i.e.
///   belonging to whoever hosts it) or explicitly anchored to this
///   combatant;
/// - on ANY OTHER host in the combat, every effect explicitly anchored to
///   this combatant (`duration.anchor == Some(combatant.doc.id)`).
///
/// The second pass is what keeps the host and the anchor independent: an
/// effect physically living on A's actor but anchored to B ticks, expires
/// and is captured on B's clock, and is claimed by neither combatant twice.
/// Results are deduplicated by `(host, path)` — the same key
/// `transition::run_boundary`/`transition::end`/`history::capture` dedup on
/// — so an effect reachable from both passes is returned once. An
/// item-embedded effect still requires its own `transfer` flag. An `Event`
/// combatant hosts no effects, but may still anchor effects hosted
/// elsewhere, so both passes run for it too.
pub fn collect_effects(snap: &CombatSnapshot, combatant: &Combatant) -> Vec<EffectRef> {
    let mut out = Vec::new();
    let mut own: HashSet<Uuid> = HashSet::new();
    if let CombatantKind::Actor { token_id, actor_id } = &combatant.engine.kind {
        if let Some(actor_id) = actor_id {
            own.insert(*actor_id);
            if let Some(host) = snap.hosts.get(actor_id) {
                walk_host(
                    host,
                    *actor_id,
                    "",
                    combatant.doc.id,
                    AnchorScope::OwnHost,
                    &mut out,
                );
            }
        }
        if let Some(token_id) = token_id {
            own.insert(*token_id);
            if let Some(token) = snap.hosts.get(token_id) {
                if let Some(actor) = token.embedded.get("actor").and_then(|v| v.first()) {
                    walk_host(
                        actor,
                        *token_id,
                        "/embedded/actor/0",
                        combatant.doc.id,
                        AnchorScope::OwnHost,
                        &mut out,
                    );
                }
            }
        }
    }

    let mut seen: HashSet<(Uuid, String)> = out.iter().map(|r| (r.host, r.path.clone())).collect();
    // Deterministic host order: `snap.hosts` is a `HashMap`, whose iteration
    // order varies run to run, and the collected order decides which
    // combatant's boundary sweep claims a shared `(host, path)` first.
    let mut cross: Vec<Uuid> = snap
        .hosts
        .keys()
        .copied()
        .filter(|h| !own.contains(h))
        .collect();
    cross.sort_unstable();
    for host_id in cross {
        let Some(host_doc) = snap.hosts.get(&host_id) else {
            continue;
        };
        let mut found = Vec::new();
        walk_any_host(
            host_doc,
            host_id,
            combatant.doc.id,
            AnchorScope::AnchoredOnly,
            &mut found,
        );
        for r in found {
            if seen.insert((r.host, r.path.clone())) {
                out.push(r);
            }
        }
    }
    out
}

/// `collect_effects` unioned over every combatant in the snapshot, paired
/// with the combatant it was collected for. Used by `transition::end`, which
/// must expire cleanup-eligible effects across the WHOLE combat rather than
/// one combatant's turn.
pub fn collect_all_effects(snap: &CombatSnapshot) -> Vec<(Uuid, EffectRef)> {
    snap.combatants
        .iter()
        .flat_map(|c| {
            collect_effects(snap, c)
                .into_iter()
                .map(move |r| (c.doc.id, r))
        })
        .collect()
}

/// Filters to only the refs that are still `active` — shared by `tick` and
/// `expire_by_policy`, since an already-inactive effect never counts toward
/// either a boundary tick or a lifecycle-policy expiry.
fn active_refs(refs: &[EffectRef]) -> impl Iterator<Item = &EffectRef> {
    refs.iter().filter(|r| r.engine.active)
}

/// Builds a `FieldChange` writing `new` at `{effect_path}{field}` on `host`
/// (e.g. `effect_path = "/embedded/effect/0"`, `field = "/engine/active"`).
/// The OCC pre-image is read from `host`'s own current value at that
/// pointer, same convention as `ops::set_engine`.
pub fn set_effect_field(
    host: &Document,
    effect_path: &str,
    field: &str,
    new: serde_json::Value,
) -> Result<FieldChange, CombatError> {
    set_engine(host, &format!("{effect_path}{field}"), new)
}

/// The evaluated lifecycle flags for one ref, or a recorded failure. Shared
/// by `tick` and `expire_by_policy` so the two consumers cannot diverge on
/// which document an effect's formulas read or how a failure is reported.
fn flags_for(
    r: &EffectRef,
    host: &Document,
    defaults: &EffectLifecycleDefaults,
    failures: &mut Vec<String>,
) -> Option<LifecycleFlags> {
    match eval::lifecycle_flags(
        r.engine.lifecycle.as_ref(),
        defaults,
        eval::effect_host_doc(host),
    ) {
        Ok(f) => Some(f),
        Err(e) => {
            // The host document id prefixes the relative pointer so identical
            // failures on DIFFERENT hosts cannot dedup-collapse into one line.
            failures.push(format!("effect at {}{}: {}", r.host, r.path, e.detail));
            None
        }
    }
}

/// Decrements `Duration.remaining` by one for every ref in `refs` that is
/// still `active`, whose evaluated lifecycle `on_advance` is set, and whose
/// `expires`/`unit` match `boundary`/`unit` exactly. A `remaining` of `None`
/// means not yet ticked: the first matching boundary evaluates
/// `Duration.amount` (`eval::duration_amount`) and writes that count minus
/// this tick. When a decrement reaches `0`, the same op also sets
/// `active = false`; an already-inactive ref is left untouched (a dead
/// effect no longer counts down). An evaluation failure (lifecycle or
/// amount) records a detail line in the returned failure list and skips that
/// one ref. `hosts` supplies each ref's CURRENT host document for the OCC
/// pre-image (a prior tick earlier in the same transition may already have
/// mutated it).
pub(crate) fn tick(
    hosts: &HashMap<Uuid, Document>,
    refs: &[EffectRef],
    boundary: ExpiryPoint,
    unit: DurationUnit,
    defaults: &EffectLifecycleDefaults,
) -> Result<(Vec<Operation>, Vec<String>), CombatError> {
    let mut ops = Vec::new();
    let mut failures = Vec::new();
    for r in active_refs(refs) {
        let Some(duration) = &r.engine.duration else {
            continue;
        };
        if duration.expires != boundary || duration.unit != unit {
            continue;
        }
        let host = hosts.get(&r.host).ok_or(CombatError::NotFound)?;
        let Some(flags) = flags_for(r, host, defaults, &mut failures) else {
            continue;
        };
        if !flags.on_advance {
            continue;
        }
        let remaining = match duration.remaining {
            Some(n) => n,
            None => match eval::duration_amount(&duration.amount, eval::effect_host_doc(host)) {
                Ok(n) => n,
                Err(e) => {
                    failures.push(format!("effect at {}{}: {}", r.host, r.path, e.detail));
                    continue;
                }
            },
        };
        let new_remaining = remaining.saturating_sub(1);
        let mut changes = vec![set_effect_field(
            host,
            &r.path,
            "/engine/duration/remaining",
            json!(new_remaining),
        )?];
        if new_remaining == 0 {
            changes.push(set_effect_field(
                host,
                &r.path,
                "/engine/active",
                json!(false),
            )?);
        }
        ops.push(Operation::Update {
            doc_id: r.host,
            changes,
        });
    }
    Ok((ops, failures))
}

/// Sets `active = false` for every ref in `refs` that is currently `active`
/// and whose evaluated lifecycle flags satisfy `pick` — used for
/// `on_combat_end`/`on_turn_end` expiry, independent of `Duration`/`remaining`
/// entirely (an effect with no duration at all still expires by policy). An
/// evaluation failure records a detail line in the returned failure list and
/// skips that one ref.
pub(crate) fn expire_by_policy(
    hosts: &HashMap<Uuid, Document>,
    refs: &[EffectRef],
    pick: fn(&LifecycleFlags) -> bool,
    defaults: &EffectLifecycleDefaults,
) -> Result<(Vec<Operation>, Vec<String>), CombatError> {
    let mut ops = Vec::new();
    let mut failures = Vec::new();
    for r in active_refs(refs) {
        let host = hosts.get(&r.host).ok_or(CombatError::NotFound)?;
        let Some(flags) = flags_for(r, host, defaults, &mut failures) else {
            continue;
        };
        if !pick(&flags) {
            continue;
        }
        let change = set_effect_field(host, &r.path, "/engine/active", json!(false))?;
        ops.push(Operation::Update {
            doc_id: r.host,
            changes: vec![change],
        });
    }
    Ok((ops, failures))
}
