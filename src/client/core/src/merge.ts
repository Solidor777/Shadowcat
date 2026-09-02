// Pure structural-diff/clone primitives shared by the stamp/sync side of the templates system
// (`templates.ts`). The 3-way merge computation itself runs server-side
// (`crate::merge`, the Rust behavioural twin) — see that module's doc comment; this file keeps
// only what `snapshotBase`/`syncState`/`stampInstance` need locally. Every value is plain JSON
// (objects recurse key-by-key, arrays are opaque leaves, scalars are leaves).
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
 * intermediate segment. The set-only server `set_pointer` cannot delete; a whole-band merge
 * result that removes a key/element rewrites the whole enclosing container server-side.
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

/** The mergeable bands of a live document; `embedded` children are full documents (envelope
 * preserved). Written whole-band by the server's `merge::plan::plan_to_update`. */
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
  /** The child's `source.id` at sync time — the correlation key the server's merge engine
   * matches instance/template children by. */
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

/** The opaque `Document.base` snapshot shape (server-derived, client-read). Top-level bands +
 * recursive embedded content keyed for provenance correlation. */
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

/** Per-`doc_type` instance-local paths that never merge.
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
