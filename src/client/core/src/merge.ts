// Pure structural-diff/clone primitives shared by the stamp/sync side of the templates system
// (`templates.ts`). The 3-way merge computation itself runs server-side (`crate::merge`) — see
// that module's doc comment; this file keeps only what `snapshotBase`/`syncState`/`stampInstance`
// need locally. Every value is plain JSON (objects recurse key-by-key, arrays are opaque leaves,
// scalars are leaves).
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

/** The `Document.base` snapshot shape and its embedded-child record — the ts-rs output of the
 * server's `merge::bands::MergeBase`/`EmbeddedBaseChild` (server-derived, client-read), re-exported
 * so the stamp/sync helpers here and their consumers name the ONE generated declaration rather
 * than a hand-written copy of it. */
export type { MergeBase, EmbeddedBaseChild } from "@shadowcat/types";

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
