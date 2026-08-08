// Client-core template operations: stamp (create-from-template) + the 3-way pull/push/revert
// emission. All produce document ops; the caller dispatches via `dispatchIntent`. The
// server never merges — a merge is an ordinary batched `Update`.
import type { WireDocument, WireOperation, WireFieldChange, WirePermissionSet } from "./wire";
import {
  merge3, merge3Tree, takeTemplate, structuralDiff, isPlacementExcluded, placementExclusions, deepEqual,
  restampSubtree, type MergeBase, type MergeBands, type MergePlan, type Conflict, type EmbeddedBaseChild,
} from "./merge";

/** Where a stamped instance lands: the initiator's world/owner/parent (never the template's). */
export interface StampOpts {
  /** The world the stamped instance is created in. */
  worldId: string;
  /** The stamped instance's owner, or `null` for none. */
  ownerId: string | null;
  /** The stamped instance's parent document id, or `null` for a top-level document. */
  parentId: string | null;
  /** The initiator's own permissions for the new doc; a deny-all default when omitted. */
  permissions?: WirePermissionSet;
}

/** A deny-all `PermissionSet` for a freshly stamped instance when `StampOpts.permissions` is
 * omitted. Not exported (folded into `stampInstance`'s public surface).
 * @returns `{default: "none", users: {}, property_overrides: {}, capabilities: {by_role: {},
 * by_user: {}}, gm_role: null}`.
 * @example
 * ```
 * // internal helper; not part of the public API (see stampInstance for the public entry point)
 * defaultPerms();
 * ```
 */
function defaultPerms(): WirePermissionSet {
  return { default: "none", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null };
}

/** Recursively reduce a document's `embedded` collections to `EmbeddedBaseChild` records for a
 * `MergeBase` snapshot. Not exported (folded into `snapshotBase`'s public surface).
 * @param embedded The document's `embedded` collections, keyed by collection name.
 * @returns The same collections, each child reduced to `{sourceId, name, engine, system,
 * embedded}` (deep-cloned so the snapshot never aliases the live document).
 * @example
 * ```
 * // internal helper; not part of the public API (see snapshotBase for the public entry point)
 * declare const doc: WireDocument;
 * snapshotEmbedded(doc.embedded);
 * ```
 */
function snapshotEmbedded(embedded: Record<string, WireDocument[]>): Record<string, EmbeddedBaseChild[]> {
  const out: Record<string, EmbeddedBaseChild[]> = {};
  for (const [coll, kids] of Object.entries(embedded)) {
    out[coll] = kids.map((k) => ({
      // Correlation key: the child's source.id (== its template child's id). A non-provenance
      // child falls back to its own id (still a stable per-child key).
      sourceId: k.source?.id ?? k.id,
      name: k.name,
      engine: structuredClone(k.engine ?? null),
      system: structuredClone(k.system ?? null),
      embedded: snapshotEmbedded(k.embedded),
    }));
  }
  return out;
}

/** Builds the value stored at `WireDocument.base` — see that field's own doc comment for what it
 * means and when it's present. Works for both a stamped instance (children keyed by their
 * `source.id`) and a template (children key on `source.id ?? id`, which for a template child is
 * its own id — the same correlation key its instances point to).
 * @param doc The document to snapshot.
 * @returns A deep-cloned `MergeBase` of `doc`'s `name`/`engine`/`system`/`embedded` bands.
 * @example
 * ```ts
 * import { snapshotBase, envelope } from "@shadowcat/core";
 *
 * const doc = envelope("world-1", "item", null, { weight: 1 });
 * const base = snapshotBase(doc);
 * base.system; // { weight: 1 }
 * ```
 */
export function snapshotBase(doc: WireDocument): MergeBase {
  return {
    name: doc.name,
    engine: structuredClone(doc.engine ?? null),
    system: structuredClone(doc.system ?? null),
    embedded: snapshotEmbedded(doc.embedded),
  };
}

/** Deep-clone `source`'s mergeable bands into a NEW document (fresh id, initiator owner/perms,
 * caller parent/scope, `source` provenance, recursively fresh embedded ids + provenance), then
 * capture `base`. Deep-clone independence is load-bearing — never `{...doc}` for nested bands
 * ([[embedded-copy-needs-deep-clone]]).
 * @param source The template document to stamp an instance from.
 * @param opts Where the new instance lands (world/owner/parent/permissions).
 * @returns A fresh `WireDocument` with `source: {id: source.id, ...}` and `base` set to a
 * snapshot of the assembled instance (see `snapshotBase`'s doc comment for why the snapshot
 * target is the stamped document, not `source`, even though their mergeable content is
 * equivalent).
 * @example
 * ```ts
 * import { stampInstance, envelope } from "@shadowcat/core";
 *
 * const template = envelope("world-1", "item", null, { weight: 2 });
 * const instance = stampInstance(template, { worldId: "world-1", ownerId: null, parentId: null });
 * instance.source?.id === template.id; // true
 * ```
 */
export function stampInstance(source: WireDocument, opts: StampOpts): WireDocument {
  const clone = structuredClone(source) as WireDocument;
  const embedded: Record<string, WireDocument[]> = {};
  // `restampSubtree` already deep-clones whatever it's handed; mapping it directly over
  // `source.embedded` avoids a redundant whole-subtree clone via `clone.embedded`.
  for (const [coll, kids] of Object.entries(source.embedded)) embedded[coll] = kids.map(restampSubtree);
  // Non-compendium case is unconditionally null (matches `restampSubtree`'s identical convention):
  // `source.source?.pack` is the TEMPLATE's own unrelated provenance, not this stamp's.
  const pack = source.scope.kind === "compendium" ? source.scope.pack : null;
  const now = Date.now();
  const stamped: WireDocument = {
    ...clone,
    id: crypto.randomUUID(),
    scope: { kind: "world", world_id: opts.worldId },
    owner: opts.ownerId,
    permissions: opts.permissions ? structuredClone(opts.permissions) : defaultPerms(),
    parent_id: opts.parentId,
    source: { id: source.id, pack, version: source.source?.version ?? 1 },
    embedded,
    created_at: now,
    updated_at: now,
  };
  stamped.base = snapshotBase(stamped);
  return stamped;
}

/** Provenance/sync state of a document for the sheet chrome (§6.4). */
export type SyncState = "none" | "up_to_date" | "template_changed";

/** 3-way pull: merge the template's current state into the child, preserving child-local diffs.
 * `base` is the child's stored snapshot (falls back to `snapshotBase(child)` — a snapshot of the
 * CHILD, not the template — when `child.base` is absent, so a base-less child yields a clean
 * template-wins result: see `merge3`'s doc comment for why that fallback produces zero conflicts).
 * @param child The instance document to pull into.
 * @param template The template document to pull from.
 * @returns The 3-way `MergePlan` (merged bands + any conflicts) — does not itself write anything;
 * pass conflict-free results to `planToUpdate`, or resolve conflicts via `applyResolutions` first.
 * @example
 * ```ts
 * import { computePull, type WireDocument } from "@shadowcat/core";
 *
 * declare const child: WireDocument;
 * declare const template: WireDocument;
 * const plan = computePull(child, template);
 * plan.conflicts.length; // 0 when nothing overlaps
 * ```
 */
export function computePull(child: WireDocument, template: WireDocument): MergePlan {
  const base: MergeBase = (child.base as MergeBase | undefined) ?? snapshotBase(child);
  return merge3(base, template, child, placementExclusions(child.doc_type));
}

/** Append a `WireFieldChange` iff `before` and `after` structurally differ. Not exported (folded
 * into `planToUpdate`'s public surface).
 * @param changes The change list to append to (mutated).
 * @param path The RFC-6901 pointer for the change.
 * @param before The pre-image (`old`).
 * @param after The proposed value (`new`).
 * @example
 * ```
 * // internal helper; not part of the public API (see planToUpdate for the public entry point)
 * declare const changes: WireFieldChange[];
 * declare const child: WireDocument;
 * declare const mergedBands: MergeBands;
 * pushIfChanged(changes, "/name", child.name, mergedBands.name);
 * ```
 */
function pushIfChanged(changes: WireFieldChange[], path: string, before: unknown, after: unknown): void {
  if (!deepEqual(before, after)) changes.push({ path, old: before, new: after });
}

/** True when a collection value represents "no items": absent (`null`) or an empty array. Used
 * to skip a vacuous embedded-collection change (both sides have nothing) even when `before` and
 * `after` differ in absence-vs-empty-array shape (which itself is never emitted — see
 * `planToUpdate`).
 * @param v The collection value (a stored/merged embedded-collection band).
 * @returns `true` iff `v` is `null` or an empty array.
 * @example
 * ```
 * // internal helper; not part of the public API (see planToUpdate for the public entry point)
 * isEmptyCollection(null); // true
 * ```
 */
function isEmptyCollection(v: WireDocument[] | null): boolean {
  return v === null || v.length === 0;
}

/** Turn merged bands into ONE `Update`: at most one whole-band change per changed band
 * (`/name`, `/engine`, `/system`), one per changed embedded collection (whole array), plus a
 * `/base` refresh (new = the template's current snapshot). Every `old` is the child's REAL
 * current stored value (OCC pre-image). Whole-band/whole-collection writes are the only
 * `set_pointer`-compatible way to delete keys / grow embedded arrays.
 * @param child The instance document the `Update` targets (its stored values seed every `old`).
 * @param template The template document (its current snapshot becomes the new `/base`).
 * @param mergedBands The resolved bands to write (from `computePull`/`computeRevert`, optionally
 * passed through `applyResolutions` first).
 * @returns One `Update` `WireOperation` for `child.id`.
 * @example
 * ```ts
 * import { planToUpdate, type WireDocument, type MergeBands } from "@shadowcat/core";
 *
 * declare const child: WireDocument;
 * declare const template: WireDocument;
 * declare const mergedBands: MergeBands;
 * const op = planToUpdate(child, template, mergedBands);
 * op.op; // "update"
 * ```
 */
export function planToUpdate(child: WireDocument, template: WireDocument, mergedBands: MergeBands): WireOperation {
  const changes: WireFieldChange[] = [];
  pushIfChanged(changes, "/name", child.name, mergedBands.name);
  pushIfChanged(changes, "/engine", child.engine ?? null, mergedBands.engine);
  pushIfChanged(changes, "/system", child.system ?? null, mergedBands.system);
  const colls = new Set([...Object.keys(child.embedded), ...Object.keys(mergedBands.embedded)]);
  for (const coll of [...colls].sort()) {
    // A collection key genuinely absent from `child.embedded` must fall back to `null`, NOT `[]`:
    // the server reads a missing JSON pointer as `Value::Null`, and its numeric-aware pre-image
    // comparison falls through to plain `==` for a (Null, Array([])) pair, which is `false` —
    // emitting `[]` here would produce an `old` that never matches the server's actual stored
    // state, causing a spurious OCC `Conflict` on an otherwise honest pre-image.
    const before = coll in child.embedded ? child.embedded[coll] : null;
    const after = coll in mergedBands.embedded ? mergedBands.embedded[coll] : null;
    if (!deepEqual(before, after) && !(isEmptyCollection(before) && isEmptyCollection(after))) {
      changes.push({ path: `/embedded/${coll}`, old: before, new: after });
    }
  }
  changes.push({ path: "/base", old: (child.base ?? null) as unknown, new: snapshotBase(template) });
  return { op: "update", doc_id: child.id, changes };
}

/** Apply the user's per-field conflict choices: for each conflict whose path is in `theirs`, take
 * the template value/deletion; the rest keep the child ("mine") value already in `mergedBands`.
 * Pure (clones its input).
 * @param mergedBands The child-wins-default bands from a `MergePlan`.
 * @param conflicts `MergePlan`'s unresolved conflicts.
 * @param theirs The set of conflict `path`s the user picked "take template" for.
 * @returns A new `MergeBands` with each `theirs` path resolved toward the template.
 * @example
 * ```ts
 * import { applyResolutions, type MergeBands, type Conflict } from "@shadowcat/core";
 *
 * declare const mergedBands: MergeBands;
 * declare const conflicts: Conflict[];
 * const resolved = applyResolutions(mergedBands, conflicts, new Set(["/system/hp"]));
 * ```
 */
export function applyResolutions(mergedBands: MergeBands, conflicts: Conflict[], theirs: Set<string>): MergeBands {
  const root = structuredClone({
    name: mergedBands.name, engine: mergedBands.engine, system: mergedBands.system, embedded: mergedBands.embedded,
  });
  for (const c of conflicts) if (theirs.has(c.path)) takeTemplate(root, c);
  return { name: root.name, engine: root.engine, system: root.system, embedded: root.embedded };
}

/** The subset of a document's mergeable bands `revertBands`/`revertChild` operate on —
 * `embedded` is handled by the separate `revertEmbedded` algorithm, not this type. */
type Bands = {
  /** The band's display name. */
  name: string | null;
  /** The band's engine body, if any. */
  engine?: unknown;
  /** The band's opaque system body, if any. */
  system?: unknown;
};

/** Reset one node's own bands to the template's current value, keeping placement paths intact. Reuses
 * `merge3Tree` with the child as its OWN base (so `childDiff` is always empty and every parent
 * diff auto-applies with zero conflicts) — the "always take template" trick. NOTE: this handles
 * only `name`/`engine`/`system`; embedded reset is a SEPARATE algorithm (`revertEmbedded`) — see
 * below for why `merge3Embedded`'s pull-shaped correlation table cannot be reused for revert. Not
 * exported (folded into `computeRevert`'s public surface).
 * @param child The node's own current bands (used only as the merge's shared "base"/"child").
 * @param template The node's template counterpart's current bands.
 * @param exclusions Placement-excluded pointers to keep from the child (never reset).
 * @returns The reset bands: every non-excluded path becomes the template's current value.
 * @example
 * ```
 * // internal helper; not part of the public API (see computeRevert for the public entry point)
 * declare const child: WireDocument;
 * declare const template: WireDocument;
 * revertBands(child, template, placementExclusions(child.doc_type));
 * ```
 */
function revertBands(child: Bands, template: Bands, exclusions: string[]): Bands {
  const selfBase = { name: child.name, engine: child.engine ?? null, system: child.system ?? null };
  const templateNow = { name: template.name, engine: template.engine ?? null, system: template.system ?? null };
  const { merged } = merge3Tree(selfBase, templateNow, selfBase, exclusions);
  return merged as Bands;
}

/**
 * Embedded-collection reset for revert. `merge3Embedded` (used by pull/push) KEEPS an
 * uncorrelated instance child ("instance-added" — correct when preserving local additions is
 * the point). Revert wants the opposite: discard every local addition. Correlation is by
 * `childKid.source.id` against a CURRENT template child's id — no stored `base` is consulted
 * (there is nothing to preserve), so:
 * - a child correlated to a current template child → recurse-reset it (`revertChild`);
 * - a child with no correlation (no `source`, or `source.id` not among the template's current
 *   children) → DROPPED (a locally-added child never belongs in a reverted instance);
 * - a template child with no correlating instance child → freshly stamped in (restores content
 *   the instance had locally deleted, or never had).
 *
 * Not exported (folded into `computeRevert`'s public surface).
 * @param parentEmbedded The template's current `embedded` collections.
 * @param childEmbedded The instance's current `embedded` collections.
 * @returns The reset collections, per the correlation rules above.
 * @example
 * ```
 * // internal helper; not part of the public API (see computeRevert for the public entry point)
 * declare const template: WireDocument;
 * declare const child: WireDocument;
 * revertEmbedded(template.embedded, child.embedded);
 * ```
 */
function revertEmbedded(
  parentEmbedded: Record<string, WireDocument[]>,
  childEmbedded: Record<string, WireDocument[]>,
): Record<string, WireDocument[]> {
  const merged: Record<string, WireDocument[]> = {};
  const colls = new Set([...Object.keys(parentEmbedded), ...Object.keys(childEmbedded)]);
  for (const coll of [...colls].sort()) {
    const parentKids = parentEmbedded[coll] ?? [];
    const childKids = childEmbedded[coll] ?? [];
    const templateById = new Map(parentKids.map((t) => [t.id, t]));
    const out: WireDocument[] = [];
    for (const cd of childKids) {
      const t = cd.source ? templateById.get(cd.source.id) : undefined;
      if (t) out.push(revertChild(cd, t));
      // else: no correlation → child-added → dropped.
    }
    for (const t of parentKids) {
      if (!childKids.some((cd) => cd.source?.id === t.id)) out.push(restampSubtree(t));
    }
    merged[coll] = out;
  }
  return merged;
}

/** Reset one matched embedded child: its own bands to the template counterpart (placement kept),
 * recursing into its own embedded collections the same way. Not exported (folded into
 * `computeRevert`'s public surface).
 * @param child The instance's embedded child.
 * @param template The correlated template embedded child.
 * @returns A full clone of `child` with its bands reset and its own embedded collections
 * recursively reset via `revertEmbedded`.
 * @example
 * ```
 * // internal helper; not part of the public API (see computeRevert for the public entry point)
 * declare const childKid: WireDocument;
 * declare const templateKid: WireDocument;
 * revertChild(childKid, templateKid);
 * ```
 */
function revertChild(child: WireDocument, template: WireDocument): WireDocument {
  const bands = revertBands(child, template, placementExclusions(child.doc_type));
  // Full clone (not a shallow spread) so the returned envelope never aliases `child`'s
  // `permissions`/`scope`/`source`/`owner` — mirrors `applyMergedBands`.
  const out = structuredClone(child) as WireDocument;
  return {
    ...out,
    name: bands.name,
    engine: bands.engine,
    system: bands.system,
    embedded: revertEmbedded(template.embedded, child.embedded),
  };
}

/** Revert: discard the child's local diffs on the mergeable bands — every path becomes the
 * template's current value, embedded content resets per `revertEmbedded` — except placement
 * paths (kept), then refresh `base`. No conflicts are possible (revert never asks the user
 * to choose; it always takes the template).
 * @param child The instance document to revert.
 * @param template The template document to revert against.
 * @returns One `Update` `WireOperation` for `child.id` (via `planToUpdate`).
 * @example
 * ```ts
 * import { computeRevert, type WireDocument } from "@shadowcat/core";
 *
 * declare const child: WireDocument;
 * declare const template: WireDocument;
 * const op = computeRevert(child, template);
 * op.op; // "update"
 * ```
 */
export function computeRevert(child: WireDocument, template: WireDocument): WireOperation {
  const bands = revertBands(child, template, placementExclusions(child.doc_type));
  const mergedBands: MergeBands = {
    name: bands.name,
    engine: bands.engine,
    system: bands.system,
    embedded: revertEmbedded(template.embedded, child.embedded),
  };
  return planToUpdate(child, template, mergedBands);
}

/** All in-store documents stamped from `templateId` (correlated by `source.id`). Same-world,
 * see+write scoped is the caller's responsibility (it passes the visible store snapshot).
 * @param templateId The template document's id to correlate against.
 * @param all The document snapshot to scan (typically `store.snapshot()`).
 * @returns Every document in `all` whose `source.id === templateId`.
 * @example
 * ```ts
 * import { findInstances, type WireDocument } from "@shadowcat/core";
 *
 * declare const all: WireDocument[];
 * findInstances("template-1", all);
 * ```
 */
export function findInstances(templateId: string, all: Iterable<WireDocument>): WireDocument[] {
  const out: WireDocument[] = [];
  for (const d of all) if (d.source?.id === templateId) out.push(d);
  return out;
}

/** "template changed" iff base diverges from the template's current mergeable snapshot, ignoring
 * placement exclusions. Purely local; `none` when unstamped or the template is not in store.
 * Computes this via its own `structuralDiff` call (a divergence-only comparison for a UI label) —
 * it does NOT call `merge3`/`computePull` and produces no `MergePlan`/conflicts; do not conflate
 * the two paths.
 * @param child The instance document to check.
 * @param template The template document, or `undefined` if not in store.
 * @returns `"none"` (unstamped or template missing), `"up_to_date"`, or `"template_changed"`.
 * @example
 * ```ts
 * import { syncState, type WireDocument } from "@shadowcat/core";
 *
 * declare const child: WireDocument;
 * declare const template: WireDocument | undefined;
 * syncState(child, template); // "none" | "up_to_date" | "template_changed"
 * ```
 */
export function syncState(child: WireDocument, template: WireDocument | undefined): SyncState {
  if (!child.source || !template) return "none";
  const base: MergeBase = (child.base as MergeBase | undefined) ?? snapshotBase(child);
  const excl = placementExclusions(child.doc_type);
  const diverged = structuralDiff(base, snapshotBase(template)).filter((d) => !isPlacementExcluded(d.path, excl));
  return diverged.length === 0 ? "up_to_date" : "template_changed";
}
