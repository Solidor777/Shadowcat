// Client-core template operations: stamp (create-from-template) + the 3-way pull/push/revert
// emission (M13e). All produce document ops; the caller dispatches via `dispatchIntent`. The
// server never merges — a merge is an ordinary batched `Update`.
import type { WireDocument, WireOperation, WireFieldChange } from "./wire";
import {
  merge3, merge3Tree, takeTemplate, structuralDiff, isPlacementExcluded, placementExclusions, deepEqual,
  restampSubtree, type MergeBase, type MergeBands, type MergePlan, type Conflict, type EmbeddedBaseChild,
} from "./merge";

/** Where a stamped instance lands: the initiator's world/owner/parent (never the template's). */
export interface StampOpts {
  worldId: string;
  ownerId: string | null;
  parentId: string | null;
  /** The initiator's own permissions for the new doc; a deny-all default when omitted. */
  permissions?: WireDocument["permissions"];
}

function defaultPerms(): WireDocument["permissions"] {
  return { default: "none", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null };
}

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

/** The opaque `base` snapshot of a document's current mergeable content. Works for both a stamped
 * instance (children keyed by their `source.id`) and a template (children key on `source.id ?? id`,
 * which for a template child is its own id — the same correlation key its instances point to). */
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
 * ([[embedded-copy-needs-deep-clone]]). */
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
 * `base` is the child's stored snapshot (falls back to the child's own snapshot when absent, so a
 * base-less child yields a clean template-wins result). */
export function computePull(child: WireDocument, template: WireDocument): MergePlan {
  const base: MergeBase = (child.base as MergeBase | undefined) ?? snapshotBase(child);
  return merge3(base, template, child, placementExclusions(child.doc_type));
}

function pushIfChanged(changes: WireFieldChange[], path: string, before: unknown, after: unknown): void {
  if (!deepEqual(before, after)) changes.push({ path, old: before, new: after });
}

/** True when a collection value represents "no items": absent (`null`) or an empty array. Used
 * to skip a vacuous embedded-collection change (both sides have nothing) even when `before` and
 * `after` differ in absence-vs-empty-array shape (which itself is never emitted — see
 * `planToUpdate`). */
function isEmptyCollection(v: WireDocument[] | null): boolean {
  return v === null || v.length === 0;
}

/** Turn merged bands into ONE `Update`: at most one whole-band change per changed band
 * (`/name`, `/engine`, `/system`), one per changed embedded collection (whole array), plus a
 * `/base` refresh (new = the template's current snapshot). Every `old` is the child's REAL
 * current stored value (OCC pre-image). Whole-band/whole-collection writes are the only
 * `set_pointer`-compatible way to delete keys / grow embedded arrays. */
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
 * Pure (clones its input). */
export function applyResolutions(mergedBands: MergeBands, conflicts: Conflict[], theirs: Set<string>): MergeBands {
  const root = structuredClone({
    name: mergedBands.name, engine: mergedBands.engine, system: mergedBands.system, embedded: mergedBands.embedded,
  });
  for (const c of conflicts) if (theirs.has(c.path)) takeTemplate(root, c);
  return { name: root.name, engine: root.engine, system: root.system, embedded: root.embedded };
}

type Bands = { name: string | null; engine?: unknown; system?: unknown };

/** Reset one node's own bands to the template's current value, keeping placement (E8). Reuses
 * `merge3Tree` with the child as its OWN base (so `childDiff` is always empty and every parent
 * diff auto-applies with zero conflicts) — the "always take template" trick. NOTE: this handles
 * only `name`/`engine`/`system`; embedded reset is a SEPARATE algorithm (`revertEmbedded`) — see
 * below for why `merge3Embedded`'s pull-shaped correlation table cannot be reused for revert. */
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
 * recursing into its own embedded collections the same way. */
function revertChild(child: WireDocument, template: WireDocument): WireDocument {
  const bands = revertBands(child, template, placementExclusions(child.doc_type));
  // Full clone (not a shallow spread) so the returned envelope never aliases `child`'s
  // `permissions`/`scope`/`source`/`owner` — mirrors `applyMergedBands` in `merge.ts`.
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
 * paths (kept, E8), then refresh `base`. No conflicts are possible (revert never asks the user
 * to choose; it always takes the template). */
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
 * see+write scoped is the caller's responsibility (it passes the visible store snapshot). */
export function findInstances(templateId: string, all: Iterable<WireDocument>): WireDocument[] {
  const out: WireDocument[] = [];
  for (const d of all) if (d.source?.id === templateId) out.push(d);
  return out;
}

/** "template changed" iff base diverges from the template's current mergeable snapshot, ignoring
 * placement exclusions. Purely local; `none` when unstamped or the template is not in store. */
export function syncState(child: WireDocument, template: WireDocument | undefined): SyncState {
  if (!child.source || !template) return "none";
  const base: MergeBase = (child.base as MergeBase | undefined) ?? snapshotBase(child);
  const excl = placementExclusions(child.doc_type);
  const diverged = structuralDiff(base, snapshotBase(template)).filter((d) => !isPlacementExcluded(d.path, excl));
  return diverged.length === 0 ? "up_to_date" : "template_changed";
}
