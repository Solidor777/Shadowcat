// Pure, order-independent 3-way merge primitives (client-core). The server never merges;
// the merge is computed here and applied as an ordinary batched `Update`. Every value
// is plain JSON (objects recurse key-by-key, arrays are opaque leaves, scalars are leaves).
import { setPointer, getPointer } from "./store";
import type { WireDocument } from "./wire";

/** One structural change between two JSON trees at an RFC-6901 pointer. */
export type Diff =
  | {
      /** The RFC-6901 pointer where the change occurred. */
      path: string;
      /** A value was written or overwritten at `path`. */
      kind: "set";
      /** The new value at `path`. */
      value: unknown;
    }
  | {
      /** The RFC-6901 pointer where the change occurred. */
      path: string;
      /** The key/element at `path` was removed (no value). */
      kind: "delete";
    };

/** Narrows `v` to a non-null, non-array object — the recursion boundary `structuralDiff`/
 * `deepEqual` use to decide "recurse key-by-key" vs "treat as an opaque leaf". Not exported.
 * @param v The value to test.
 * @returns `true` iff `v` is a non-null object that is not an array.
 * @example
 * ```
 * // internal predicate; not part of the public API
 * isPlainObject({ a: 1 }); // true
 * ```
 */
function isPlainObject(v: unknown): v is Record<string, unknown> {
  return v !== null && typeof v === "object" && !Array.isArray(v);
}

/** Deep structural equality: objects key-order-independent, arrays positional, scalars strict.
 * @param a The first value.
 * @param b The second value.
 * @returns `true` iff `a` and `b` are structurally equal per the rules above.
 * @example
 * ```ts
 * import { deepEqual } from "@shadowcat/core";
 *
 * deepEqual({ a: 1, b: 2 }, { b: 2, a: 1 }); // true
 * deepEqual([1, 2], [2, 1]); // false
 * ```
 */
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

/** RFC-6901 token escaping (`~` → `~0`, `/` → `~1`). Not exported.
 * @param k The raw object key to escape into a pointer token.
 * @returns The escaped token, safe to join with `/` into a JSON pointer.
 * @example
 * ```
 * // internal helper; not part of the public API
 * escapeToken("a/b"); // "a~1b"
 * ```
 */
function escapeToken(k: string): string {
  return k.replace(/~/g, "~0").replace(/\//g, "~1");
}

/**
 * Structural diff of `now` against `base` as one JSON tree. Objects recurse key-by-key;
 * arrays are opaque leaves (any inequality → one whole-array `set`); scalars/type-changes are
 * leaves. Sorted-key traversal makes the output order-independent.
 * @param base The prior (pre-image) tree.
 * @param now The current tree to diff against `base`.
 * @param prefix The RFC-6901 pointer prefix for this call's subtree (empty at the root;
 * callers should not pass this).
 * @returns The list of `set`/`delete` diffs needed to turn `base` into `now`.
 * @example
 * ```ts
 * import { structuralDiff } from "@shadowcat/core";
 *
 * const diffs = structuralDiff({ hp: 10 }, { hp: 7 });
 * diffs; // [{ path: "/hp", kind: "set", value: 7 }]
 * ```
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

/** Split an RFC-6901 pointer into unescaped tokens (drops the leading empty segment). Not
 * exported.
 * @param pointer An RFC-6901 JSON pointer, e.g. `"/a/b~1c"`.
 * @returns The unescaped path segments, e.g. `["a", "b/c"]`.
 * @example
 * ```
 * // internal helper; not part of the public API
 * tokenize("/a/b~1c"); // ["a", "b/c"]
 * ```
 */
function tokenize(pointer: string): string[] {
  return pointer.split("/").slice(1).map((t) => t.replace(/~1/g, "/").replace(/~0/g, "~"));
}

/**
 * Remove the object key or array element at `pointer` in `root` (mutates). No-op on any missing
 * intermediate segment. The set-only server `set_pointer` cannot delete; a merge that removes a
 * key/element rewrites the whole enclosing container (see `planToUpdate`), and this builds that
 * rewritten container in memory first.
 * @param root The tree to mutate.
 * @param pointer The RFC-6901 pointer of the key/element to remove.
 * @example
 * ```ts
 * import { deletePointer } from "@shadowcat/core";
 *
 * const doc = { system: { hp: 10 } };
 * deletePointer(doc, "/system/hp");
 * doc; // { system: {} }
 * ```
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
  /** The RFC-6901 pointer of the conflicting field. */
  path: string;
  /** The value at `path` in the last-synced snapshot both sides diverged from. */
  base: unknown;
  /** The template/parent side's current value at `path`; `undefined` iff the parent deleted it. */
  parent: unknown;
  /** The instance/child side's current value at `path`; `undefined` iff the child deleted it. */
  child: unknown;
  /** How "take template" (`takeTemplate`) resolves this conflict: `"set"` writes `parent`,
   * `"delete"` removes the key. */
  parentKind: "set" | "delete";
};

/** Whether `path` is inside the placement exclusion set (equal or a descendant).
 * @param path The RFC-6901 pointer to test.
 * @param exclusions The placement-excluded pointers (see `placementExclusions`).
 * @returns `true` iff `path` equals or is a descendant of any entry in `exclusions`.
 * @example
 * ```ts
 * import { isPlacementExcluded } from "@shadowcat/core";
 *
 * isPlacementExcluded("/engine/x", ["/engine/x", "/engine/y", "/engine/rotation"]); // true
 * isPlacementExcluded("/engine/hp", ["/engine/x"]); // false
 * ```
 */
export function isPlacementExcluded(path: string, exclusions: string[]): boolean {
  return exclusions.some((e) => path === e || path.startsWith(`${e}/`));
}

/** JSON-pointer subtree overlap (either contains the other, or equal). Not exported.
 * @param a The first pointer.
 * @param b The second pointer.
 * @returns `true` iff `a` and `b` are equal or one is an ancestor of the other.
 * @example
 * ```
 * // internal helper; not part of the public API
 * pathsOverlap("/system/hp", "/system"); // true
 * ```
 */
function pathsOverlap(a: string, b: string): boolean {
  return a === b || a.startsWith(`${b}/`) || b.startsWith(`${a}/`);
}

/** Whether two diffs at the SAME kind produce the same outcome (`set` with `deepEqual` values,
 * or both `delete`). Used by `merge3Tree` to decide a same-value overlap is not a real
 * conflict. Not exported.
 * @param a The first diff.
 * @param b The second diff.
 * @returns `true` iff `a` and `b` describe the same resulting change.
 * @example
 * ```
 * // internal helper; not part of the public API
 * sameResult({ path: "/hp", kind: "set", value: 7 }, { path: "/hp", kind: "set", value: 7 }); // true
 * ```
 */
function sameResult(a: Diff, b: Diff): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "delete") return true;
  return deepEqual(
    a.value,
    (b as { /** The `set` diff's value, narrowed past the `Diff` union's `"delete"` arm. */ value: unknown })
      .value,
  );
}

/** Apply one `Diff` into `root` (mutates): `set` clones `d.value` before splicing it in (`d.value`
 * is a live reference into the source tree `structuralDiff` walked, e.g. `parentNow` — cloning
 * keeps `root` from aliasing that tree), `delete` removes the key/element via `deletePointer`.
 * Not exported.
 * @param root The tree to mutate.
 * @param d The diff to apply.
 * @example
 * ```
 * // internal helper; not part of the public API
 * applyDiff(root, { path: "/hp", kind: "set", value: 7 });
 * ```
 */
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
 * @param base The last-synced snapshot both sides diverged from.
 * @param parentNow The template/parent's current tree.
 * @param childNow The instance/child's current tree.
 * @param exclusions Placement-excluded pointers dropped from the parent side before diffing
 * (never merged, never conflict) — see `placementExclusions`.
 * @returns The merged tree (child-wins default) plus the list of unresolved conflicts.
 * @example
 * ```ts
 * import { merge3Tree } from "@shadowcat/core";
 *
 * const { merged, conflicts } = merge3Tree({ hp: 10 }, { hp: 12 }, { hp: 10, name: "orc" }, []);
 * merged; // { hp: 12, name: "orc" }
 * conflicts; // []
 * ```
 */
export function merge3Tree(
  base: unknown,
  parentNow: unknown,
  childNow: unknown,
  exclusions: string[],
): {
  /** The merged tree (child-wins default for unresolved conflicts). */
  merged: unknown;
  /** Unresolved conflicts, left at the child value in `merged`. */
  conflicts: Conflict[];
} {
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

/** Apply the parent's decision for a conflict into `root` (in place).
 * @param root The tree to mutate (typically a merged-bands clone).
 * @param c The conflict to resolve toward the parent/template side.
 * @example
 * ```ts
 * import { takeTemplate, type Conflict } from "@shadowcat/core";
 *
 * const root = { hp: 10 };
 * const c: Conflict = { path: "/hp", base: 8, parent: 12, child: 10, parentKind: "set" };
 * takeTemplate(root, c);
 * root; // { hp: 12 }
 * ```
 */
export function takeTemplate(root: unknown, c: Conflict): void {
  if (c.parentKind === "delete") deletePointer(root, c.path);
  else setPointer(root, c.path, c.parent);
}

/** The mergeable bands of a live document; `embedded` children are full documents (envelope
 * preserved). Produced by `merge3`, written whole-band by `planToUpdate`. */
export type MergeBands = {
  /** The document's `name` band after merge. */
  name: string | null;
  /** The document's `engine` band after merge. */
  engine: unknown;
  /** The document's `system` band after merge. */
  system: unknown;
  /** Merged embedded collections, keyed by collection name; each child is a full document
   * (envelope preserved), not a bands-only record. */
  embedded: Record<string, WireDocument[]>;
};

/** One embedded child inside a `base` snapshot: bands + the `sourceId` correlation key (the
 * child's `source.id` at sync time — the template child's id). Recurses (finite-depth embedding). */
export type EmbeddedBaseChild = {
  /** The child's `source.id` at sync time — the correlation key `merge3Embedded` matches
   * instance/template children by. */
  sourceId: string;
  /** The child's `name` band at sync time. */
  name: string | null;
  /** The child's `engine` band at sync time. */
  engine: unknown;
  /** The child's `system` band at sync time. */
  system: unknown;
  /** The child's own embedded collections at sync time, recursively in the same shape. */
  embedded: Record<string, EmbeddedBaseChild[]>;
};

/** The opaque `Document.base` snapshot shape (client-owned). Top-level bands + recursive
 * embedded content keyed for provenance correlation. */
export type MergeBase = {
  /** The document's `name` band at sync time. */
  name: string | null;
  /** The document's `engine` band at sync time. */
  engine: unknown;
  /** The document's `system` band at sync time. */
  system: unknown;
  /** Embedded collections at sync time, keyed by collection name, each reduced to
   * `EmbeddedBaseChild` records (not full documents). */
  embedded: Record<string, EmbeddedBaseChild[]>;
};

/** Result of a 3-way merge: the child-wins-default merged bands + the conflicts to resolve. */
export type MergePlan = {
  /** The merged bands (child-wins default for unresolved conflicts). */
  mergedBands: MergeBands;
  /** Every unresolved conflict, top-level and embedded. */
  conflicts: Conflict[];
};

/** Per-`doc_type` instance-local paths that never merge (E8).
 * @param docType The document's `doc_type`.
 * @returns The list of `/engine/*` pointers excluded from template merge for this doc type
 * (currently only `token`'s placement fields; every other doc type gets `[]`).
 * @example
 * ```ts
 * import { placementExclusions } from "@shadowcat/core";
 *
 * placementExclusions("token"); // ["/engine/x", "/engine/y", "/engine/rotation"]
 * placementExclusions("actor"); // []
 * ```
 */
export function placementExclusions(docType: string): string[] {
  return docType === "token" ? ["/engine/x", "/engine/y", "/engine/rotation"] : [];
}

/** Deep-clone `doc` into a new subtree: fresh `id`, `source` pointing at the template (`doc.id`),
 * recursively for every embedded child. Used to stamp a template-added embedded child into an
 * instance. Deep-clone independence is load-bearing ([[embedded-copy-needs-deep-clone]]).
 * @param doc The template (or template-embedded) document to restamp.
 * @returns A fresh clone with a new `id` and `source` pointing at `doc.id`; every embedded
 * child is restamped the same way, recursively.
 * @example
 * ```ts
 * import { restampSubtree, envelope } from "@shadowcat/core";
 *
 * const template = envelope("world-1", "item", null, { weight: 1 });
 * const stamped = restampSubtree(template);
 * stamped.id !== template.id; // true
 * stamped.source?.id === template.id; // true
 * ```
 */
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
 * `/system/*` at exactly the document's real pointers. Not exported.
 * @param name The document's `name` band.
 * @param engine The document's `engine` band (`?? null`).
 * @param system The document's `system` band (`?? null`).
 * @returns `{ name, engine, system }`, the synthetic tree `merge3Tree` diffs/merges.
 * @example
 * ```
 * // internal helper; not part of the public API
 * bandsTree("Goblin", { x: 0, y: 0 }, { hp: 7 });
 * ```
 */
function bandsTree(name: string | null, engine: unknown, system: unknown): Record<string, unknown> {
  return { name, engine: engine ?? null, system: system ?? null };
}

/** MergeBase-shaped bands of a live embedded child (for the recursive 3-way base). Not exported.
 * @param b The embedded base child record.
 * @returns The same bands, minus `sourceId`, shaped as a `MergeBase`.
 * @example
 * ```
 * // internal helper; not part of the public API
 * baseFromChild({ sourceId: "t1", name: "Sword", engine: null, system: {}, embedded: {} });
 * ```
 */
function baseFromChild(b: EmbeddedBaseChild): MergeBase {
  return { name: b.name, engine: b.engine, system: b.system, embedded: b.embedded };
}

/** Bands of a live document as a MergeBase (no sourceId at the top). Not exported.
 * @param d The live document to extract bands from.
 * @returns The document's `name`/`engine`/`system` bands plus its `embedded` collections,
 * each child recursively reduced to its own `EmbeddedBaseChild` record.
 * @example
 * ```
 * // internal helper; not part of the public API
 * bandsMergeBase(someWireDocument);
 * ```
 */
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

/** Whether an instance child's bands are unchanged versus its base record. Not exported.
 * @param child The live embedded instance child.
 * @param b The correlated base record for `child` (by `sourceId`).
 * @returns `true` iff `child`'s current bands structurally equal `b`'s recorded bands.
 * @example
 * ```
 * // internal helper; not part of the public API
 * childUnchangedVsBase(someWireDocument, someBaseChild);
 * ```
 */
function childUnchangedVsBase(child: WireDocument, b: EmbeddedBaseChild): boolean {
  return structuralDiff(
    { name: b.name, engine: b.engine, system: b.system, embedded: b.embedded },
    bandsMergeBase(child),
  ).length === 0;
}

/** Write merged bands into a fresh clone of `child`'s envelope. Not exported.
 * @param child The live embedded instance child (envelope source: `permissions`/`scope`/
 * `source`/`owner` etc. are preserved from here).
 * @param bands The merged `name`/`engine`/`system`/`embedded` bands to write in.
 * @returns A full clone of `child` (never aliasing it) with `bands` written in.
 * @example
 * ```
 * // internal helper; not part of the public API
 * applyMergedBands(someWireDocument, someMergeBands);
 * ```
 */
function applyMergedBands(child: WireDocument, bands: MergeBands): WireDocument {
  // Full clone (not a shallow spread) so the returned envelope never aliases `child`'s
  // `permissions`/`scope`/`source`/`owner` — matches every other crossing point in
  // `merge3Embedded` (instance-added, base-missing fail-safe, kept-pending-conflict).
  const out = structuredClone(child) as WireDocument;
  return { ...out, name: bands.name, engine: bands.engine, system: bands.system, embedded: bands.embedded };
}

/** Rewrite each conflict's `path` to be relative to the top-level document, prefixing it with
 * the embedded child's collection + index (`/embedded/<coll>/<idx>`). Not exported.
 * @param conflicts The conflicts, with paths relative to the embedded child's own tree.
 * @param coll The embedded collection key the child lives in.
 * @param idx The child's index within `merged[coll]` (the OUTPUT array being built, not its
 * position in `childKids`/`parentKids`).
 * @returns `conflicts`, each with `path` prefixed by `/embedded/<coll>/<idx>`.
 * @example
 * ```
 * // internal helper; not part of the public API
 * prefixConflicts(someConflicts, "items", 0);
 * ```
 */
function prefixConflicts(conflicts: Conflict[], coll: string, idx: number): Conflict[] {
  const p = `/embedded/${coll}/${idx}`;
  return conflicts.map((c) => ({ ...c, path: `${p}${c.path}` }));
}

/** 3-way merge of the embedded collections, correlating instance↔template children by
 * `source.id`↔`id` (E7), using `base.embedded[coll][*].sourceId` as the membership record.
 * Not exported (folded into `merge3`'s public surface).
 * @param base The last-synced `MergeBase.embedded` snapshot.
 * @param parentEmbedded The template's current `embedded` collections.
 * @param childEmbedded The instance's current `embedded` collections.
 * @returns The merged collections (instance-added kept, template-added restamped in,
 * template-deleted dropped or conflicted per `childUnchangedVsBase`) plus path-prefixed
 * conflicts.
 * @example
 * ```
 * // internal helper; not part of the public API (see merge3 for the public entry point)
 * merge3Embedded({}, {}, {});
 * ```
 */
function merge3Embedded(
  base: Record<string, EmbeddedBaseChild[]>,
  parentEmbedded: Record<string, WireDocument[]>,
  childEmbedded: Record<string, WireDocument[]>,
): {
  /** The merged embedded collections, keyed by collection name. */
  merged: Record<string, WireDocument[]>;
  /** Conflicts found while merging embedded children, path-prefixed with `/embedded/<coll>/<idx>`. */
  conflicts: Conflict[];
} {
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
 * Pure + order-independent. Conflicts default to the child ("keep mine") in `mergedBands`.
 * @param base The last-synced `MergeBase` snapshot. `computePull` passes
 * `child.base ?? snapshotBase(child)` — a base-less (unstamped) child falls back to a snapshot
 * of ITSELF, not the template: `childDiff` is then empty against that base, so every
 * template-side change auto-applies with zero conflicts (a clean template-wins result),
 * matching `computePull`'s own doc comment. (`merge3` also recurses on itself for correlated
 * embedded children, via `merge3Embedded`.)
 * @param parentNow The template's current document.
 * @param childNow The instance's current document.
 * @param exclusions Top-level placement-excluded pointers (see `placementExclusions`); embedded
 * children compute their own from their own `doc_type`.
 * @returns The merged bands (child-wins default) plus every unresolved conflict, top-level and
 * embedded (embedded paths are `/embedded/<coll>/<idx>`-prefixed by `merge3Embedded`).
 * @example
 * ```ts
 * import { merge3, envelope, type MergeBase } from "@shadowcat/core";
 *
 * const template = envelope("world-1", "item", null, { weight: 2 });
 * const instance = envelope("world-1", "item", null, { weight: 1 }, undefined, undefined, "Sword");
 * const base: MergeBase = { name: null, engine: undefined, system: { weight: 1 }, embedded: {} };
 * const plan = merge3(base, template, instance, []);
 * plan.mergedBands.system; // { weight: 2 } — template-only change auto-applies
 * ```
 */
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
  const m = tree.merged as {
    /** The merged `name` band. */
    name: string | null;
    /** The merged `engine` band. */
    engine: unknown;
    /** The merged `system` band. */
    system: unknown;
  };
  const emb = merge3Embedded(base.embedded, parentNow.embedded, childNow.embedded);
  return {
    mergedBands: { name: m.name, engine: m.engine, system: m.system, embedded: emb.merged },
    conflicts: [...tree.conflicts, ...emb.conflicts],
  };
}
