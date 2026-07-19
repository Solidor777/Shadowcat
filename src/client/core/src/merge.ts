// Pure, order-independent 3-way merge primitives (client-core). The server never merges;
// the merge is computed here and applied as an ordinary batched `Update` (M13e). Every value
// is plain JSON (objects recurse key-by-key, arrays are opaque leaves, scalars are leaves).
import { setPointer, getPointer } from "./store";
import type { WireDocument } from "./wire";

/** One structural change between two JSON trees at an RFC-6901 pointer. */
export type Diff =
  | { path: string; kind: "set"; value: unknown }
  | { path: string; kind: "delete" };

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return v !== null && typeof v === "object" && !Array.isArray(v);
}

/** Deep structural equality: objects key-order-independent, arrays positional, scalars strict. */
export function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!deepEqual(a[i], b[i])) return false;
    return true;
  }
  if (isPlainObject(a) && isPlainObject(b)) {
    const ak = Object.keys(a);
    if (ak.length !== Object.keys(b).length) return false;
    for (const k of ak) {
      if (!Object.prototype.hasOwnProperty.call(b, k)) return false;
      if (!deepEqual(a[k], b[k])) return false;
    }
    return true;
  }
  return false;
}

/** RFC-6901 token escaping (`~` → `~0`, `/` → `~1`). */
function escapeToken(k: string): string {
  return k.replace(/~/g, "~0").replace(/\//g, "~1");
}

/**
 * Structural diff of `now` against `base` as one JSON tree. Objects recurse key-by-key;
 * arrays are opaque leaves (any inequality → one whole-array `set`); scalars/type-changes are
 * leaves. Sorted-key traversal makes the output order-independent.
 */
export function structuralDiff(base: unknown, now: unknown, prefix = ""): Diff[] {
  if (isPlainObject(base) && isPlainObject(now)) {
    const out: Diff[] = [];
    const keys = new Set([...Object.keys(base), ...Object.keys(now)]);
    for (const key of [...keys].sort()) {
      const p = `${prefix}/${escapeToken(key)}`;
      const inBase = Object.prototype.hasOwnProperty.call(base, key);
      const inNow = Object.prototype.hasOwnProperty.call(now, key);
      if (inBase && !inNow) out.push({ path: p, kind: "delete" });
      else if (!inBase && inNow) out.push({ path: p, kind: "set", value: now[key] });
      else out.push(...structuralDiff(base[key], now[key], p));
    }
    return out;
  }
  if (deepEqual(base, now)) return [];
  return [{ path: prefix, kind: "set", value: now }];
}

function tokenize(pointer: string): string[] {
  return pointer.split("/").slice(1).map((t) => t.replace(/~1/g, "/").replace(/~0/g, "~"));
}

/**
 * Remove the object key or array element at `pointer` in `root` (mutates). No-op on any missing
 * intermediate segment. The set-only server `set_pointer` cannot delete; a merge that removes a
 * key/element rewrites the whole enclosing container (see `planToUpdate`), and this builds that
 * rewritten container in memory first.
 */
export function deletePointer(root: unknown, pointer: string): void {
  if (pointer === "") throw new Error("cannot delete the document root");
  const tokens = tokenize(pointer);
  let cur: unknown = root;
  for (const tok of tokens.slice(0, -1)) {
    if (Array.isArray(cur)) cur = cur[Number(tok)];
    else if (isPlainObject(cur)) cur = cur[tok];
    else return;
  }
  const last = tokens[tokens.length - 1];
  if (Array.isArray(cur)) {
    const i = Number(last);
    if (Number.isInteger(i) && i >= 0 && i < cur.length) cur.splice(i, 1);
  } else if (isPlainObject(cur)) {
    delete cur[last];
  }
}

/** A field changed on both the template (parent) and instance (child) sides since the last
 * sync. `parent`/`child` are `undefined` when that side deleted the key. `parentKind` records
 * how "take template" resolves it (set the parent value, or delete the key). */
export type Conflict = {
  path: string;
  base: unknown;
  parent: unknown;
  child: unknown;
  parentKind: "set" | "delete";
};

/** Whether `path` is inside the placement exclusion set (equal or a descendant). */
export function isPlacementExcluded(path: string, exclusions: string[]): boolean {
  return exclusions.some((e) => path === e || path.startsWith(`${e}/`));
}

/** JSON-pointer subtree overlap (either contains the other, or equal). */
function pathsOverlap(a: string, b: string): boolean {
  return a === b || a.startsWith(`${b}/`) || b.startsWith(`${a}/`);
}

function sameResult(a: Diff, b: Diff): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "delete") return true;
  return deepEqual(a.value, (b as { value: unknown }).value);
}

function applyDiff(root: unknown, d: Diff): void {
  // `d.value` (a `set` diff) is a live reference into the source tree `structuralDiff` walked
  // (e.g. `parentNow`); clone before splicing into `root` so `root` never aliases that tree.
  if (d.kind === "set") setPointer(root, d.path, structuredClone(d.value));
  else deletePointer(root, d.path);
}

/**
 * 3-way merge of one JSON tree (used for the `name`+`engine`+`system` synthetic band tree).
 * `merged` starts from `childNow` and applies parent-only changes; a path changed on both sides
 * with a differing result is a conflict, left at the child value ("keep mine" default). Paths in
 * `exclusions` are dropped from the parent side (never merge, never conflict). Pure +
 * order-independent (sorted-key `structuralDiff`).
 *
 * Ancestor/descendant overlap (e.g. child deletes an object the parent edits inside) is treated
 * as a conflict at the parent change's path (`pathsOverlap`) — the safe direction; arrays are
 * opaque leaves so deep-array changes are single-path.
 */
export function merge3Tree(
  base: unknown,
  parentNow: unknown,
  childNow: unknown,
  exclusions: string[],
): { merged: unknown; conflicts: Conflict[] } {
  const parentDiff = structuralDiff(base, parentNow).filter((d) => !isPlacementExcluded(d.path, exclusions));
  const childDiff = structuralDiff(base, childNow);
  const merged = structuredClone(childNow) as unknown;
  const conflicts: Conflict[] = [];
  for (const p of parentDiff) {
    const overlapping = childDiff.filter((c) => pathsOverlap(c.path, p.path));
    if (overlapping.length === 0) {
      applyDiff(merged, p);
      continue;
    }
    const exact = overlapping.find((c) => c.path === p.path);
    if (exact && overlapping.length === 1 && sameResult(p, exact)) continue;
    // `parent`/`child` may reference subtrees of `parentNow`/`childNow`; clone them so a
    // `Conflict` can never alias (and a caller mutating it can never corrupt) either source tree.
    conflicts.push({
      path: p.path,
      base: getPointer(base, p.path),
      parent: p.kind === "set" ? structuredClone(p.value) : undefined,
      child:
        exact && exact.kind === "set"
          ? structuredClone(exact.value)
          : structuredClone(getPointer(childNow, p.path)),
      parentKind: p.kind,
    });
  }
  return { merged, conflicts };
}

/** Apply the parent's decision for a conflict into `root` (in place). */
export function takeTemplate(root: unknown, c: Conflict): void {
  if (c.parentKind === "delete") deletePointer(root, c.path);
  else setPointer(root, c.path, c.parent);
}

/** The mergeable bands of a live document; `embedded` children are full documents (envelope
 * preserved). Produced by `merge3`, written whole-band by `planToUpdate`. */
export type MergeBands = {
  name: string | null;
  engine: unknown;
  system: unknown;
  embedded: Record<string, WireDocument[]>;
};

/** One embedded child inside a `base` snapshot: bands + the `sourceId` correlation key (the
 * child's `source.id` at sync time — the template child's id). Recurses (finite-depth embedding). */
export type EmbeddedBaseChild = {
  sourceId: string;
  name: string | null;
  engine: unknown;
  system: unknown;
  embedded: Record<string, EmbeddedBaseChild[]>;
};

/** The opaque `Document.base` snapshot shape (client-owned). Top-level bands + recursive
 * embedded content keyed for provenance correlation. */
export type MergeBase = {
  name: string | null;
  engine: unknown;
  system: unknown;
  embedded: Record<string, EmbeddedBaseChild[]>;
};

/** Result of a 3-way merge: the child-wins-default merged bands + the conflicts to resolve. */
export type MergePlan = { mergedBands: MergeBands; conflicts: Conflict[] };

/** Per-`doc_type` instance-local paths that never merge (E8). */
export function placementExclusions(docType: string): string[] {
  return docType === "token" ? ["/engine/x", "/engine/y", "/engine/rotation"] : [];
}

/** Deep-clone `doc` into a new subtree: fresh `id`, `source` pointing at the template (`doc.id`),
 * recursively for every embedded child. Used to stamp a template-added embedded child into an
 * instance. Deep-clone independence is load-bearing ([[embedded-copy-needs-deep-clone]]). */
export function restampSubtree(doc: WireDocument): WireDocument {
  const out = structuredClone(doc) as WireDocument;
  out.id = crypto.randomUUID();
  out.source = { id: doc.id, pack: null, version: doc.source?.version ?? 1 };
  // `base` (a sync snapshot) belongs only on a top-level stamped document, set explicitly by
  // `stampInstance` after the whole tree is assembled; a restamped subtree has no prior sync
  // snapshot of its own, so clear any stale/foreign `base` the clone inherited from `doc`.
  out.base = undefined;
  const embedded: Record<string, WireDocument[]> = {};
  for (const [coll, kids] of Object.entries(doc.embedded)) embedded[coll] = kids.map(restampSubtree);
  out.embedded = embedded;
  return out;
}

/** The three synthetic-tree bands as one object, so `merge3Tree` addresses `/name`, `/engine/*`,
 * `/system/*` at exactly the document's real pointers. */
function bandsTree(name: string | null, engine: unknown, system: unknown): Record<string, unknown> {
  return { name, engine: engine ?? null, system: system ?? null };
}

/** MergeBase-shaped bands of a live embedded child (for the recursive 3-way base). */
function baseFromChild(b: EmbeddedBaseChild): MergeBase {
  return { name: b.name, engine: b.engine, system: b.system, embedded: b.embedded };
}

/** Bands of a live document as a MergeBase (no sourceId at the top). */
function bandsMergeBase(d: WireDocument): MergeBase {
  const emb: Record<string, EmbeddedBaseChild[]> = {};
  for (const [coll, kids] of Object.entries(d.embedded)) {
    emb[coll] = kids.map((k) => ({
      sourceId: k.source?.id ?? k.id,
      name: k.name,
      engine: k.engine ?? null,
      system: k.system ?? null,
      embedded: bandsMergeBase(k).embedded,
    }));
  }
  return { name: d.name, engine: d.engine ?? null, system: d.system ?? null, embedded: emb };
}

/** Whether an instance child's bands are unchanged versus its base record. */
function childUnchangedVsBase(child: WireDocument, b: EmbeddedBaseChild): boolean {
  return structuralDiff(
    { name: b.name, engine: b.engine, system: b.system, embedded: b.embedded },
    bandsMergeBase(child),
  ).length === 0;
}

function applyMergedBands(child: WireDocument, bands: MergeBands): WireDocument {
  // Full clone (not a shallow spread) so the returned envelope never aliases `child`'s
  // `permissions`/`scope`/`source`/`owner` — matches every other crossing point in
  // `merge3Embedded` (instance-added, base-missing fail-safe, kept-pending-conflict).
  const out = structuredClone(child) as WireDocument;
  return { ...out, name: bands.name, engine: bands.engine, system: bands.system, embedded: bands.embedded };
}

function prefixConflicts(conflicts: Conflict[], coll: string, idx: number): Conflict[] {
  const p = `/embedded/${coll}/${idx}`;
  return conflicts.map((c) => ({ ...c, path: `${p}${c.path}` }));
}

/** 3-way merge of the embedded collections, correlating instance↔template children by
 * `source.id`↔`id` (E7), using `base.embedded[coll][*].sourceId` as the membership record. */
function merge3Embedded(
  base: Record<string, EmbeddedBaseChild[]>,
  parentEmbedded: Record<string, WireDocument[]>,
  childEmbedded: Record<string, WireDocument[]>,
): { merged: Record<string, WireDocument[]>; conflicts: Conflict[] } {
  const merged: Record<string, WireDocument[]> = {};
  const conflicts: Conflict[] = [];
  const colls = new Set([...Object.keys(base), ...Object.keys(parentEmbedded), ...Object.keys(childEmbedded)]);
  for (const coll of [...colls].sort()) {
    const baseKids = base[coll] ?? [];
    const parentKids = parentEmbedded[coll] ?? [];
    const childKids = childEmbedded[coll] ?? [];
    const templateById = new Map(parentKids.map((t) => [t.id, t]));
    const baseBySource = new Map(baseKids.map((b) => [b.sourceId, b]));
    const out: WireDocument[] = [];

    // Pass 1: walk instance children in order.
    for (const cd of childKids) {
      const sid = cd.source?.id ?? null;
      const correlated = sid !== null && (templateById.has(sid) || baseBySource.has(sid));
      if (!correlated) {
        out.push(structuredClone(cd)); // instance-added → keep (cloned; never alias childNow's live tree)
        continue;
      }
      const t = sid !== null ? templateById.get(sid) : undefined;
      const b = sid !== null ? baseBySource.get(sid) : undefined;
      if (t) {
        if (b) {
          const idx = out.length;
          const plan = merge3(baseFromChild(b), t, cd, placementExclusions(cd.doc_type));
          out.push(applyMergedBands(cd, plan.mergedBands));
          conflicts.push(...prefixConflicts(plan.conflicts, coll, idx));
        } else {
          // base-missing matched → keep instance (fail-safe; no 3-way base); cloned so the
          // result never aliases childNow's live tree.
          out.push(structuredClone(cd));
        }
        continue;
      }
      // template absent, base present → template-deleted this correlation
      // invariant: correlated && !t implies b is defined (correlated's disjunction forces
      // baseBySource.has(sid) when templateById.has(sid) is false)
      if (b) {
        if (childUnchangedVsBase(cd, b)) continue; // drop
        const idx = out.length;
        out.push(structuredClone(cd)); // kept pending resolution; cloned, never aliases childNow
        conflicts.push({
          path: `/embedded/${coll}/${idx}`,
          base: b.system,
          parent: undefined,
          // `cd.system` is a live reference into `childNow`'s object graph; clone at the
          // crossing point into `Conflict` per the `merge3Tree` `child` convention.
          child: structuredClone(cd.system ?? null),
          parentKind: "delete",
        });
      }
    }

    // Pass 2: template-added children (in template, absent from base, no instance copy).
    for (const t of parentKids) {
      if (baseBySource.has(t.id)) continue;
      if (childKids.some((cd) => cd.source?.id === t.id)) continue;
      out.push(restampSubtree(t));
    }

    merged[coll] = out;
  }
  return { merged, conflicts };
}

/** Full 3-way merge over the mergeable bands (`name`+`engine`+`system` tree + `embedded`).
 * `exclusions` apply to the top-level doc; embedded children use their own doc_type exclusions.
 * Pure + order-independent. Conflicts default to the child ("keep mine") in `mergedBands`. */
export function merge3(
  base: MergeBase,
  parentNow: WireDocument,
  childNow: WireDocument,
  exclusions: string[],
): MergePlan {
  const tree = merge3Tree(
    bandsTree(base.name, base.engine, base.system),
    bandsTree(parentNow.name, parentNow.engine ?? null, parentNow.system),
    bandsTree(childNow.name, childNow.engine ?? null, childNow.system),
    exclusions,
  );
  const m = tree.merged as { name: string | null; engine: unknown; system: unknown };
  const emb = merge3Embedded(base.embedded, parentNow.embedded, childNow.embedded);
  return {
    mergedBands: { name: m.name, engine: m.engine, system: m.system, embedded: emb.merged },
    conflicts: [...tree.conflicts, ...emb.conflicts],
  };
}
