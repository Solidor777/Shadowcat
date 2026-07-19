// Client-core template operations: stamp (create-from-template) + the 3-way pull/push/revert
// emission (M13e). All produce document ops; the caller dispatches via `dispatchIntent`. The
// server never merges — a merge is an ordinary batched `Update`.
import type { WireDocument } from "./wire";
import { restampSubtree, type MergeBase, type EmbeddedBaseChild } from "./merge";

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
