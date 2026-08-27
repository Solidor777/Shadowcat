#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::dice::outcome::{DieRecord, RawDie, RawRoll};
use crate::dice::rng::{roll_uniform, RngSource};
use crate::dice::spec::{DiceGroup, DieKind, ExplodeKind, GroupModifier};

/// Hard cap on chained explosions/rerolls per die — prevents infinite loops when a
/// comparator is always true (e.g. explode on `>=1`).
const CHAIN_CAP: usize = 100;

/// Look up a die's post-index value/symbols. `Numeric` dice pass their natural
/// straight through (no faces table). A `Faces` die's `natural` is a face
/// INDEX; a `None`-value face contributes `0` numerically.
fn face_value_and_symbols(kind: &DieKind, natural: i32) -> (i32, Vec<crate::dice::spec::Symbol>) {
    match kind {
        DieKind::Numeric { .. } => (natural, vec![]),
        DieKind::Faces { faces } => {
            let face = &faces[natural as usize];
            (face.value.unwrap_or(0), face.symbols.clone())
        }
    }
}

/// Resolve one group's dice: apply modifiers in order, returning per-die records.
/// Reroll/explode mutate the die set (new dice allocated via `raws`); keep/drop only
/// flip `kept`. All rolled dice stay in the returned vec (dropped dice for display).
///
/// CONTRACT: Reroll and Explode only act on currently-`kept` dice. Since modifiers
/// apply in vec order and keep/drop can precede reroll/explode in the same group,
/// a die already dropped by an earlier `KeepHighest`/`KeepLowest`/`DropHighest`/
/// `DropLowest` must not be rerolled or exploded by a later modifier.
///
/// `group_index` identifies which `Dice` AST node this call is resolving (assigned
/// by the caller in AST left-to-right order); it is stamped onto every `DieRecord`
/// produced here, including exploded/penetrated children, so Total-mode evaluation
/// can fold per-group without positional heuristics.
pub fn resolve_group(
    group: &DiceGroup,
    group_index: usize,
    naturals: &[RawDie],
    rng: &mut dyn RngSource,
    raws: &mut RawRoll,
) -> Vec<DieRecord> {
    let ordered = group.kind.is_ordered();
    let mut recs: Vec<DieRecord> = naturals
        .iter()
        .map(|d| {
            let (value, symbols) = face_value_and_symbols(&group.kind, d.natural);
            DieRecord {
                id: d.id,
                group_index,
                natural: d.natural,
                value,
                kept: true,
                exploded: false,
                rerolled_from: None,
                crit_success: false,
                crit_fail: false,
                expertise: 0,
                label: group.label.clone(),
                symbols,
                ordered,
            }
        })
        .collect();

    // Redraw a fresh face for the current DieKind: a numeric value for `Numeric`,
    // a face INDEX for `Faces` (the caller must re-derive value/symbols from it
    // via `face_value_and_symbols`).
    let redraw = |rng: &mut dyn RngSource| -> i32 {
        match &group.kind {
            DieKind::Numeric { min, max } => roll_uniform(rng, *min, *max),
            DieKind::Faces { faces } => roll_uniform(rng, 0, faces.len() as i32 - 1),
        }
    };

    for m in &group.modifiers {
        if !ordered {
            // Unordered Faces dice have no rankable value — every value-reading
            // modifier (reroll/explode-by-comparator, keep/drop) is a no-op.
            continue;
        }
        match m {
            GroupModifier::Reroll { comp, target, once } => {
                for r in recs.iter_mut() {
                    // Dropped dice are not live participants in later modifiers.
                    if !r.kept {
                        continue;
                    }
                    let mut chain = 0;
                    while comp.test(r.value, *target) && chain < CHAIN_CAP {
                        r.rerolled_from = Some(r.value);
                        let drawn = redraw(rng);
                        let (value, symbols) = face_value_and_symbols(&group.kind, drawn);
                        r.value = value;
                        r.symbols = symbols;
                        chain += 1;
                        if *once {
                            break;
                        }
                    }
                }
            }
            GroupModifier::Explode { kind, comp, target } => {
                // Snapshot the pool length before this pass: the outer loop only
                // trigger-scans dice that existed when the modifier started. The
                // inner chain loop below is the SOLE mechanism that extends any
                // given die's own chain (it rechecks the trigger on the fresh
                // face itself); a child it pushes must never be independently
                // re-scanned by the outer loop as if it were a fresh, un-exploded
                // die — that double-applies CHAIN_CAP per pushed child and can
                // balloon without bound when the comparator is often-true.
                let initial_len = recs.len();
                let mut i = 0;
                while i < initial_len {
                    // Dropped dice are not live participants in later modifiers.
                    if recs[i].kept && comp.test(recs[i].value, *target) {
                        recs[i].exploded = true;
                        let mut chain = 0;
                        loop {
                            if chain >= CHAIN_CAP {
                                break;
                            }
                            let extra = redraw(rng);
                            // Derive the die's actual configured value at this draw
                            // BEFORE branching: for `Numeric`, `face_value_and_symbols`
                            // is a pure pass-through (`derived_value == extra`), so
                            // every retrigger/Numeric-arm behavior below is byte-for-
                            // byte identical to testing `extra` directly. For an
                            // ordered `Faces` die, `extra` is a face INDEX — testing
                            // the comparator against the index instead of the die's
                            // configured value would silently misfire whenever face
                            // value doesn't track index order.
                            let (derived_value, symbols) =
                                face_value_and_symbols(&group.kind, extra);
                            match kind {
                                ExplodeKind::Compound
                                    if matches!(group.kind, DieKind::Numeric { .. }) =>
                                {
                                    recs[i].value += extra;
                                }
                                ExplodeKind::Penetrate
                                    if matches!(group.kind, DieKind::Numeric { .. }) =>
                                {
                                    // Penetrate: the chain-continuation recheck below
                                    // uses the RAW roll `extra` (== `derived_value` for
                                    // `Numeric`, mirroring Compound), not the
                                    // decremented value — otherwise a `Gte(max)` chain
                                    // could never extend past one extra die, since
                                    // `max - 1` never satisfies `Gte(max)` even when
                                    // the die keeps rolling max. `natural` records the
                                    // true RNG face for the audit trail; only the
                                    // stored `value` (used for scoring/display) takes
                                    // the -1 penalty, and may therefore land below
                                    // `min` by design.
                                    let value = extra - 1;
                                    push_extra(
                                        &mut recs,
                                        raws,
                                        ExtraDie {
                                            kind: group.kind.clone(),
                                            group_index,
                                            label: group.label.clone(),
                                            natural: extra,
                                            value,
                                            // Reached only in the Numeric-guarded Penetrate arm.
                                            ordered: true,
                                        },
                                    );
                                }
                                _ => {
                                    // Standard explode (or Compound/Penetrate on an
                                    // ordered Faces die, where "add"/"−1" have no
                                    // defined meaning): push a fresh die at the
                                    // drawn index, using the already-derived value/symbols.
                                    let id = raws.push(group.kind.clone(), extra);
                                    recs.push(DieRecord {
                                        id,
                                        group_index,
                                        natural: extra,
                                        value: derived_value,
                                        kept: true,
                                        exploded: false,
                                        rerolled_from: None,
                                        crit_success: false,
                                        crit_fail: false,
                                        expertise: 0,
                                        label: group.label.clone(),
                                        symbols,
                                        // Reached only inside the `!ordered { continue }`-gated
                                        // modifier loop, so the producing group is always ordered.
                                        ordered: true,
                                    });
                                }
                            }
                            chain += 1;
                            // Re-check the trigger on the fresh face's DERIVED value
                            // (identical to the raw face for `Numeric`; the die's
                            // actual configured value at the drawn index for `Faces`).
                            if !comp.test(derived_value, *target) {
                                break;
                            }
                        }
                    }
                    i += 1;
                }
            }
            GroupModifier::KeepHighest(n) => keep(&mut recs, *n as usize, true, true),
            GroupModifier::KeepLowest(n) => keep(&mut recs, *n as usize, false, true),
            GroupModifier::DropHighest(n) => keep(&mut recs, *n as usize, true, false),
            GroupModifier::DropLowest(n) => keep(&mut recs, *n as usize, false, false),
        }
    }
    recs
}

/// Flip `kept` on records. `highest` = which end to select; `keep_selected` = true for
/// keep-N (select N to keep), false for drop-N (select N to drop).
fn keep(recs: &mut [DieRecord], n: usize, highest: bool, keep_selected: bool) {
    let mut idx: Vec<usize> = (0..recs.len()).collect();
    idx.sort_by(|&a, &b| {
        if highest {
            recs[b].value.cmp(&recs[a].value)
        } else {
            recs[a].value.cmp(&recs[b].value)
        }
    });
    let selected: std::collections::HashSet<usize> = idx.into_iter().take(n).collect();
    for (i, r) in recs.iter_mut().enumerate() {
        let is_selected = selected.contains(&i);
        r.kept = if keep_selected {
            is_selected
        } else {
            !is_selected
        };
    }
}

/// One exploded/penetrated extra die, as drawn and before it is logged. The
/// remaining `DieRecord` fields are constants for an extra die: it is always kept,
/// never itself marks an explosion, and carries no reroll, crit or expertise state.
struct ExtraDie {
    /// The die's kind, matching its originating group.
    kind: DieKind,
    /// Index of the `Dice` AST node that produced the originating group.
    group_index: usize,
    /// The originating group's label, if any.
    label: Option<String>,
    /// The true RNG face, recorded for the audit trail.
    natural: i32,
    /// The post-modifier stored value, used for scoring and display. For Penetrate
    /// this differs from `natural` by the -1 penalty and may land below the die's
    /// `min` by design.
    value: i32,
    /// The originating die is ordered (a `Faces` die with defined sequence).
    ordered: bool,
}

/// Push one exploded/penetrated extra die into both the raw log and the per-die
/// record vec.
fn push_extra(recs: &mut Vec<DieRecord>, raws: &mut RawRoll, extra: ExtraDie) {
    let id = raws.push(extra.kind, extra.natural);
    recs.push(DieRecord {
        id,
        group_index: extra.group_index,
        natural: extra.natural,
        value: extra.value,
        kept: true,
        exploded: false,
        rerolled_from: None,
        crit_success: false,
        crit_fail: false,
        expertise: 0,
        label: extra.label,
        symbols: vec![],
        ordered: extra.ordered,
    });
}

#[cfg(test)]
mod tests;
