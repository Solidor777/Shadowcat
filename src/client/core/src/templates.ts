// Client-core template stamp/sync helpers: stamping a new instance from a template, and the
// local `syncState` display derivation. The 3-way pull/push/revert merge itself is computed
// server-side (`crate::merge`, sent as `MergePull`/`MergePush`/`MergeRevert` intents) —
// `TemplatesController` sends the intents; nothing in this module composes a merge `Update`.
import type { WireDocument, WirePermissionSet } from "./wire";
import {
  structuralDiff, isPlacementExcluded, placementExclusions, isMergeableBandPointer, normalizeBase,
  restampSubtree, type MergeBase, type EmbeddedBaseChild,
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

/** The overrides of `doc` that govern its mergeable bands (`isMergeableBandPointer`) — what a
 * snapshot records as its policy. Not exported (folded into `snapshotBase`'s public surface).
 * @param doc The document whose `permissions.property_overrides` to filter.
 * @returns A fresh map of the mergeable-band overrides.
 * @example
 * ```
 * // internal helper; not part of the public API (see snapshotBase for the public entry point)
 * declare const doc: WireDocument;
 * recordedOverrides(doc);
 * ```
 */
function recordedOverrides(doc: WireDocument): MergeBase["property_overrides"] {
  const out: MergeBase["property_overrides"] = {};
  for (const [p, v] of Object.entries(doc.permissions.property_overrides)) if (isMergeableBandPointer(p)) out[p] = v;
  return out;
}

/** Recursively reduce a document's `embedded` collections to `EmbeddedBaseChild` records for a
 * `MergeBase` snapshot. Not exported (folded into `snapshotBase`'s public surface).
 * @param embedded The document's `embedded` collections, keyed by collection name.
 * @returns The same collections, each child reduced to `{sourceId, name, engine, system,
 * embedded, propertyOverrides}` (deep-cloned so the snapshot never aliases the live document).
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
      propertyOverrides: recordedOverrides(k),
    }));
  }
  return out;
}

/** Builds the value stored at `WireDocument.base` — see that field's own doc comment for what it
 * means and when it's present. Works for both a stamped instance (children keyed by their
 * `source.id`) and a template (children key on `source.id ?? id`, which for a template child is
 * its own id — the same correlation key its instances point to). Records the document's
 * mergeable-band policy alongside the bands (`property_overrides`, `propertyOverrides` on each
 * record), the same reduction the server's `snapshot_base` writes to `/base`, so `syncState` can
 * compare a stored base against this snapshot of the template key for key.
 * @param doc The document to snapshot.
 * @returns A deep-cloned `MergeBase` of `doc`'s `name`/`engine`/`system`/`embedded` bands plus
 * its recorded policy.
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
    property_overrides: recordedOverrides(doc),
  };
}

/** Deep-clone `source`'s mergeable bands into a NEW document (fresh id, initiator owner/perms,
 * caller parent/scope, `source` provenance, recursively fresh embedded ids + provenance), then
 * capture `base`. Deep-clone independence is load-bearing — never `{...doc}` for nested bands
 * ([[embedded-copy-needs-deep-clone]]). Stamping stays client-composed: it is a clone, not a
 * merge, so the server's authority over it is the `base` derivation the write path re-derives
 * at Create time (discarding this function's own `base` value) plus ordinary Create validation.
 * @param source The template document to stamp an instance from.
 * @param opts Where the new instance lands (world/owner/parent/permissions).
 * @returns A fresh `WireDocument` with `source: {id: source.id, ...}` and `base` set to a
 * snapshot of the assembled instance (see `snapshotBase`'s doc comment for why the snapshot
 * target is the stamped document, not `source`, even though their mergeable content is
 * equivalent). The server discards this `base` value and derives its own at Create.
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

/** Provenance/sync state of a document for the sheet chrome. */
export type SyncState = "none" | "up_to_date" | "template_changed";

/** All in-store documents stamped from `templateId` (correlated by `source.id`). Same-world,
 * see+write scoped is the caller's responsibility (it passes the visible store snapshot); kept
 * client-side for DISPLAY only (e.g. instance counts in chrome) — a `MergePush` intent finds its
 * own authoritative instance set server-side via `instances_of`.
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
 * it does NOT compute or request a merge plan and produces no conflict set; do not conflate the
 * two paths.
 *
 * Consistent for every seat: the stored `base` this client holds is the server's ONE canonical
 * snapshot of the template (full, the template's own policy recorded verbatim) cut at egress by
 * that recorded policy, and the template in store is cut by the template's current policy — so a
 * recipient compares its view of the snapshot with its view of the template, a GM both in full.
 * The recorded policy map is compared like every other key: a template policy change reads
 * `template_changed` on every seat until the merge that propagates it refreshes the snapshot.
 * Reading `base` through `normalizeBase` is what keeps a stripped snapshot key and a nulled
 * template band reading as the same value.
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
  const base: MergeBase = child.base === undefined || child.base === null ? snapshotBase(child) : normalizeBase(child.base);
  const excl = placementExclusions(child.doc_type);
  const diverged = structuralDiff(base, snapshotBase(template)).filter((d) => !isPlacementExcluded(d.path, excl));
  return diverged.length === 0 ? "up_to_date" : "template_changed";
}
