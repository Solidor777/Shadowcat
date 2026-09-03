// Pure structural-diff/clone primitives shared by the stamp/sync side of the templates system
// (`templates.ts`). The 3-way merge computation itself runs server-side (`crate::merge`) — see
// that module's doc comment; this file keeps only what `snapshotBase`/`syncState`/`stampInstance`
// need locally. Every value is plain JSON (objects recurse key-by-key, arrays are opaque leaves,
// scalars are leaves).
import type { WireDocument } from "./wire";
import type { MergeBase } from "@shadowcat/types";

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

/** Whether an override pointer names a MERGEABLE band — `/name`, `/engine`, `/system`, or a
 * path inside `engine`/`system` — the content a `MergeBase` snapshots and a merge writes. The
 * server's `writes_a_content_band` is the definition; `snapshotBase` records exactly these
 * overrides (a `/base…` override says nothing about the document's own bands), so the client's
 * template snapshot and the server-written stored base agree on the recorded policy.
 * @param pointer The `property_overrides` key to classify.
 * @returns `true` iff the pointer is a mergeable band or a path inside one.
 * @example
 * ```ts
 * import { isMergeableBandPointer } from "@shadowcat/core";
 *
 * isMergeableBandPointer("/system/secret"); // true
 * isMergeableBandPointer("/base/system/secret"); // false
 * ```
 */
export function isMergeableBandPointer(pointer: string): boolean {
  return pointer === "/name" || ["/engine", "/system"].some((b) => pointer === b || pointer.startsWith(`${b}/`));
}

/** Whether `obj` carries `key` as an own property (present, possibly `null` — never merely
 * `undefined` through a missing key), the JS-side reading of "key present" a JSON-sourced value
 * needs (a JSON payload never carries an explicit `undefined`). Not exported.
 * @param obj The object to check.
 * @param key The property name to look for.
 * @returns `true` iff `obj` has `key` as an own property.
 * @example
 * ```
 * // internal helper; not part of the public API
 * hasKey({ name: null }, "name"); // true
 * ```
 */
function hasKey(obj: Record<string, unknown>, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(obj, key);
}

/** Parse a stored `WireDocument.base` node against the shape the server's `MergeBase`/
 * `EmbeddedBaseChild` deserializer enforces at ingest (`check_base_node_shape`) — but ingest and
 * this READ differ on exactly the axis the server's own egress does: `name`/`engine`/`system` are
 * the node's MERGEABLE content bands, and the server's egress redaction can legitimately REMOVE
 * any of them wholesale from the wire copy it sends a given recipient (`redaction_target`
 * classifies a pointer inside `/base` as `Within`, an object-key deletion, when that band is
 * hidden per the node's own recorded policy) — so a recipient's redacted view can be missing any
 * of those three keys with nothing wrong, and their absence is read as `null`, matching how the
 * server reads a hidden band NULLED in place on the live template (`Access::can_see` /
 * `redaction_target::Band`). `embedded`, the policy map itself (`property_overrides`/
 * `propertyOverrides`), and a record's `sourceId` are never named by a recorded policy entry
 * (`writes_a_content_band` admits only `/name`, `/engine…`, `/system…`) — they are STRUCTURAL,
 * never subject to redaction, so their absence at any depth is not a legitimate wire shape at
 * all; the caller (`syncState`) treats that the same way it treats a genuinely absent `base`
 * rather than defaulting the gap to a value that could spuriously compare equal to the template's
 * current state and read as caught up when the stored snapshot cannot be trusted.
 * @param base The raw `base` value off the wire (`unknown`).
 * @returns The parsed `MergeBase`, or `null` if `base` is missing a required STRUCTURAL key
 * (`embedded`, `property_overrides`, or a record's `sourceId`) at any depth.
 * @example
 * ```ts
 * import { normalizeBase } from "@shadowcat/core";
 *
 * normalizeBase({ system: { hp: 1 } }); // null — missing the structural `embedded`/`property_overrides` keys
 * normalizeBase({ engine: null, system: { hp: 1 }, embedded: {}, property_overrides: {} })?.name; // null — `name` legitimately redacted away
 * ```
 */
export function normalizeBase(base: unknown): MergeBase | null {
  if (!isPlainObject(base) || !hasKey(base, "embedded") || !hasKey(base, "property_overrides")) return null;
  const embedded = normalizeBaseEmbedded(base.embedded);
  const propertyOverrides = normalizeBasePolicy(base.property_overrides);
  if (embedded === null || propertyOverrides === null) return null;
  return {
    name: typeof base.name === "string" ? base.name : null,
    engine: base.engine ?? null,
    system: base.system ?? null,
    embedded,
    property_overrides: propertyOverrides,
  };
}

/** `normalizeBase`'s recursion over the embedded records (`EmbeddedBaseChild`'s own key set,
 * `sourceId`/`propertyOverrides` spelled camelCase; `sourceId` is structural — never redacted —
 * so it is required alongside `embedded`/`propertyOverrides`, while `name`/`engine`/`system`
 * follow the same legitimately-absent rule `normalizeBase` applies at the root). Not exported.
 * @param embedded The raw `embedded` value of a snapshot node.
 * @returns The parsed record collections, or `null` if `embedded` or any record within it is
 * missing a required structural key.
 * @example
 * ```
 * // internal helper; not part of the public API (see normalizeBase for the public entry point)
 * normalizeBaseEmbedded({ items: [{ sourceId: "t", embedded: {}, propertyOverrides: {} }] });
 * ```
 */
function normalizeBaseEmbedded(embedded: unknown): MergeBase["embedded"] | null {
  if (!isPlainObject(embedded)) return null;
  const out: MergeBase["embedded"] = {};
  for (const [coll, records] of Object.entries(embedded)) {
    if (!Array.isArray(records)) return null;
    const parsed: MergeBase["embedded"][string] = [];
    for (const r of records) {
      if (!isPlainObject(r) || typeof r.sourceId !== "string") return null;
      if (!hasKey(r, "embedded") || !hasKey(r, "propertyOverrides")) return null;
      const childEmbedded = normalizeBaseEmbedded(r.embedded);
      const propertyOverrides = normalizeBasePolicy(r.propertyOverrides);
      if (childEmbedded === null || propertyOverrides === null) return null;
      parsed.push({
        sourceId: r.sourceId,
        name: typeof r.name === "string" ? r.name : null,
        engine: r.engine ?? null,
        system: r.system ?? null,
        embedded: childEmbedded,
        propertyOverrides,
      });
    }
    out[coll] = parsed;
  }
  return out;
}

/** `normalizeBase`'s reading of a recorded policy map: an object of visibility tiers. The policy
 * map itself is structural (never named by its own entries), so a non-object reads as malformed,
 * not as an empty policy. Not exported.
 * @param policy The raw policy value of a snapshot node.
 * @returns A deep clone of `policy`, or `null` if it is not an object.
 * @example
 * ```
 * // internal helper; not part of the public API (see normalizeBase for the public entry point)
 * normalizeBasePolicy({ "/system/secret": "gm_only" });
 * ```
 */
function normalizeBasePolicy(policy: unknown): MergeBase["property_overrides"] | null {
  return isPlainObject(policy) ? (structuredClone(policy) as MergeBase["property_overrides"]) : null;
}

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
