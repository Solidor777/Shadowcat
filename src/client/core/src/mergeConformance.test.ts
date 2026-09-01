// Dual-mode conformance harness for the template 3-way merge engine.
//
// Default mode asserts the engine reproduces `__fixtures__/merge-conformance.json` exactly —
// the fixture is the measured behaviour of this implementation, and the server's Rust twin
// asserts byte-identical results against the same file forever. Default mode also asserts the
// fixture's stored inputs are exactly the serialized case matrix below, so a stale or
// hand-edited fixture fails instead of silently passing. Generation mode
// (`MERGE_CORPUS_GENERATE=1`) re-runs the case matrix below and (over)writes the fixture;
// a regeneration that produces no diff is the equivalence evidence.
//
// `restampSubtree` mints random uuids for template-added children, so both modes normalize
// every fresh id to a deterministic `restamped-N` placeholder (in output traversal order)
// before comparing or writing. `null`-valued `base`/`engine` envelope keys are stripped the
// same way in every mode: absent and null are interchangeable to the merge (`?? null`
// coalesces both), while JSON object identity treats a present-null key and an absent key as
// different shapes.
import { readFileSync, writeFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { applyResolutions, computePull, computeRevert, planToUpdate } from "./templates";
import type { EmbeddedBaseChild, MergeBase } from "./merge";
import type { WireDocument } from "./wire";

/** Corpus case kinds: `pull` compares merged bands + conflicts, `resolve` additionally
 * applies `theirs` and compares the emitted update, `revert` compares the emitted update. */
type CaseKind = "pull" | "resolve" | "revert";

/** One corpus case, as authored below and as stored in the fixture. `base: null` means the
 * child carries no stored snapshot, exercising `computePull`'s snapshot-of-self fallback. */
interface CorpusCase {
  /** Unique case name, reported on failure. */
  name: string;
  /** Case kind; omitted means `pull`. */
  kind?: CaseKind;
  /** The child's stored merge snapshot, or `null` for a base-less child. */
  base: MergeBase | null;
  /** The template document. */
  parent: WireDocument;
  /** The instance document. */
  child: WireDocument;
  /** Conflict paths resolved toward the template (`resolve` cases only). */
  theirs?: string[];
  /** The expected engine output, normalized (assertion mode only; generated otherwise). */
  expect?: Record<string, unknown>;
}

/** The corpus file's shape. */
interface Corpus {
  /** Every merge case. */
  cases: CorpusCase[];
}

/** The fixture path, beside the engine it measures. */
const FIXTURE_URL = new URL("./__fixtures__/merge-conformance.json", import.meta.url);

/** Whether this run (re)writes the fixture instead of asserting against it. */
const GENERATE = process.env.MERGE_CORPUS_GENERATE === "1";

/** A deny-all permission set; the merge never reads permissions, but envelopes carry them. */
function denyAll(): WireDocument["permissions"] {
  return {
    default: "none",
    users: {},
    property_overrides: {},
    capabilities: { by_role: {}, by_user: {} },
    gm_role: null,
  };
}

/** A full envelope with a deterministic id; every unset band takes its merge-neutral default. */
function doc(over: Partial<WireDocument> & { id: string }): WireDocument {
  return {
    id: over.id,
    scope: over.scope ?? { kind: "world", world_id: "w1" },
    doc_type: over.doc_type ?? "actor",
    schema_version: 1,
    name: over.name ?? null,
    source: over.source ?? null,
    owner: over.owner ?? null,
    permissions: over.permissions ?? denyAll(),
    embedded: over.embedded ?? {},
    parent_id: over.parent_id ?? null,
    engine: over.engine,
    system: over.system ?? {},
    created_at: 0,
    updated_at: 0,
  };
}

/** A `MergeBase` snapshot with merge-neutral defaults for every unmentioned band. */
function base(over: Partial<MergeBase>): MergeBase {
  return { name: null, engine: null, system: {}, embedded: {}, ...over };
}

/** One embedded child record inside a `MergeBase`, keyed by its `sourceId` correlation. */
function baseChild(over: Partial<EmbeddedBaseChild> & { sourceId: string }): EmbeddedBaseChild {
  return { name: null, engine: null, system: {}, embedded: {}, ...over };
}

/** A stamped-instance `source` pointing at template document `id`. */
function fromTemplate(id: string): WireDocument["source"] {
  return { id, pack: null, version: 1 };
}

/** The case matrix the fixture is generated from. Every envelope id is a deterministic
 * placeholder (`t1`/`c1`/`tc1`/…); only `restampSubtree`'s fresh uuids are normalized. */
const CASES: CorpusCase[] = [
  {
    name: "parent-set-applies",
    base: base({ name: "Goblin", system: { hp: 1 } }),
    parent: doc({ id: "t1", name: "Goblin", system: { hp: 2 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), name: "Goblin", system: { hp: 1 } }),
  },
  {
    name: "child-edit-kept",
    base: base({ system: { hp: 1 } }),
    parent: doc({ id: "t1", system: { hp: 1 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: { hp: 7 } }),
  },
  {
    name: "both-set-conflict",
    base: base({ system: { hp: 1 } }),
    parent: doc({ id: "t1", system: { hp: 2 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: { hp: 3 } }),
  },
  {
    // Both sides ADD the same key with different values: the snapshot has no value at the
    // conflict path, so the conflict's `base` key is ABSENT from the serialized JSON (the
    // `skip_serializing_if` contract on `MergeConflict.base`), not present-null.
    name: "both-add-absent-base-conflict",
    base: base({}),
    parent: doc({ id: "t1", system: { hp: 2 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: { hp: 3 } }),
  },
  {
    name: "same-result-no-conflict",
    base: base({ system: { hp: 1 } }),
    parent: doc({ id: "t1", system: { hp: 5 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: { hp: 5 } }),
  },
  {
    // Same-result suppression with an extra child change SIBLING to the exact match: the
    // sibling path does not overlap the parent diff, so the exact match stays the sole
    // overlap and the same-result suppression still applies.
    name: "same-result-sibling-change-no-conflict",
    base: base({ system: { a: { b: 1, c: 1 } } }),
    parent: doc({ id: "t1", system: { a: { b: 2, c: 1 } } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: { a: { b: 2, c: 9 } } }),
  },
  {
    name: "parent-set-child-delete-conflict",
    base: base({ system: { hp: 1 } }),
    parent: doc({ id: "t1", system: { hp: 2 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: {} }),
  },
  {
    name: "parent-delete-child-set-conflict",
    base: base({ system: { hp: 1 } }),
    parent: doc({ id: "t1", system: {} }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: { hp: 3 } }),
  },
  {
    name: "parent-delete-applies",
    base: base({ system: { hp: 1, mp: 2 } }),
    parent: doc({ id: "t1", system: { hp: 1 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: { hp: 1, mp: 2 } }),
  },
  {
    name: "both-delete-no-conflict",
    base: base({ system: { hp: 1 } }),
    parent: doc({ id: "t1", system: {} }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: {} }),
  },
  {
    name: "ancestor-descendant-conflict",
    base: base({ system: { stats: { str: 10 } } }),
    parent: doc({ id: "t1", system: { stats: { str: 12 } } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: {} }),
  },
  {
    name: "descendant-ancestor-conflict",
    base: base({ system: { stats: { str: 10 } } }),
    parent: doc({ id: "t1", system: { stats: 5 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: { stats: { str: 12 } } }),
  },
  {
    name: "array-parent-win",
    base: base({ system: { xs: [1, 2] } }),
    parent: doc({ id: "t1", system: { xs: [1, 2, 3] } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: { xs: [1, 2] } }),
  },
  {
    name: "array-conflict",
    base: base({ system: { xs: [1, 2] } }),
    parent: doc({ id: "t1", system: { xs: [1, 2, 3] } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: { xs: [1, 9] } }),
  },
  {
    name: "escaped-key-conflict",
    base: base({ system: { "a/b": 1, "c~d": 1 } }),
    parent: doc({ id: "t1", system: { "a/b": 2, "c~d": 1 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: { "a/b": 3, "c~d": 9 } }),
  },
  {
    name: "name-band-conflict",
    base: base({ name: "A" }),
    parent: doc({ id: "t1", name: "B" }),
    child: doc({ id: "c1", source: fromTemplate("t1"), name: "C" }),
  },
  {
    name: "engine-band-merge",
    base: base({ engine: { a: 1, b: 1 } }),
    parent: doc({ id: "t1", engine: { a: 2, b: 1 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), engine: { a: 1, b: 9 } }),
  },
  {
    name: "token-placement-excluded",
    base: base({ engine: { x: 0, y: 0, rotation: 0, hp: 1 } }),
    parent: doc({ id: "t1", doc_type: "token", engine: { x: 99, y: 5, rotation: 90, hp: 5 } }),
    child: doc({
      id: "c1",
      doc_type: "token",
      source: fromTemplate("t1"),
      engine: { x: 3, y: 4, rotation: 10, hp: 1 },
    }),
  },
  {
    name: "base-missing-fallback",
    base: null,
    parent: doc({ id: "t1", name: "T", system: { hp: 2 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), name: "Mine", system: { hp: 99, extra: true } }),
  },
  {
    name: "conflict-order-sorted",
    base: base({ name: "A", engine: { e: 1 }, system: { s: 1 } }),
    parent: doc({ id: "t1", name: "B", engine: { e: 2 }, system: { s: 2 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), name: "C", engine: { e: 3 }, system: { s: 3 } }),
  },
  {
    // A case with BOTH top-level and embedded conflicts pins the concatenation order:
    // every tree conflict (sorted) precedes every embedded conflict.
    name: "conflict-order-tree-then-embedded",
    base: base({
      system: { hp: 1 },
      embedded: { items: [baseChild({ sourceId: "tc1", system: { x: 1 } })] },
    }),
    parent: doc({
      id: "t1",
      system: { hp: 2 },
      embedded: { items: [doc({ id: "tc1", system: { x: 2 } })] },
    }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      system: { hp: 3 },
      embedded: { items: [doc({ id: "ic1", source: fromTemplate("tc1"), system: { x: 3 } })] },
    }),
  },
  {
    name: "embedded-correlated-merge",
    base: base({ embedded: { items: [baseChild({ sourceId: "tc1", system: { hp: 1 } })] } }),
    parent: doc({ id: "t1", embedded: { items: [doc({ id: "tc1", system: { hp: 5 } })] } }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      embedded: { items: [doc({ id: "ic1", source: fromTemplate("tc1"), system: { hp: 1 } })] },
    }),
  },
  {
    name: "embedded-instance-added-kept",
    base: base({ embedded: { items: [baseChild({ sourceId: "tc1", system: { hp: 1 } })] } }),
    parent: doc({ id: "t1", embedded: { items: [doc({ id: "tc1", system: { hp: 1 } })] } }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      embedded: {
        items: [
          doc({ id: "ic1", source: fromTemplate("tc1"), system: { hp: 1 } }),
          doc({ id: "local1", system: { own: true } }),
        ],
      },
    }),
  },
  {
    name: "embedded-base-missing-fail-safe",
    base: base({ embedded: { items: [] } }),
    parent: doc({ id: "t1", embedded: { items: [doc({ id: "tc1", system: { hp: 5 } })] } }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      embedded: { items: [doc({ id: "ic1", source: fromTemplate("tc1"), system: { hp: 1 } })] },
    }),
  },
  {
    name: "embedded-template-added-restamped",
    base: base({ embedded: { items: [baseChild({ sourceId: "tc1", system: { hp: 1 } })] } }),
    parent: doc({
      id: "t1",
      embedded: {
        items: [
          doc({ id: "tc1", system: { hp: 1 } }),
          doc({ id: "tc2", name: "New", system: { k: 1 } }),
        ],
      },
    }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      embedded: { items: [doc({ id: "ic1", source: fromTemplate("tc1"), system: { hp: 1 } })] },
    }),
  },
  {
    name: "embedded-template-deleted-unchanged-dropped",
    base: base({ embedded: { items: [baseChild({ sourceId: "tc1", system: { hp: 1 } })] } }),
    parent: doc({ id: "t1", embedded: { items: [] } }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      embedded: { items: [doc({ id: "ic1", source: fromTemplate("tc1"), system: { hp: 1 } })] },
    }),
  },
  {
    name: "embedded-template-deleted-changed-conflict",
    base: base({ embedded: { items: [baseChild({ sourceId: "tc1", system: { hp: 1 } })] } }),
    parent: doc({ id: "t1", embedded: { items: [] } }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      embedded: { items: [doc({ id: "ic1", source: fromTemplate("tc1"), system: { hp: 9 } })] },
    }),
  },
  {
    name: "embedded-nested-recursion",
    base: base({
      embedded: {
        items: [
          baseChild({
            sourceId: "tc1",
            embedded: { sub: [baseChild({ sourceId: "gc1", system: { deep: 1 } })] },
          }),
        ],
      },
    }),
    parent: doc({
      id: "t1",
      embedded: {
        items: [doc({ id: "tc1", embedded: { sub: [doc({ id: "gc1", system: { deep: 7 } })] } })],
      },
    }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      embedded: {
        items: [
          doc({
            id: "ic1",
            source: fromTemplate("tc1"),
            embedded: { sub: [doc({ id: "igc1", source: fromTemplate("gc1"), system: { deep: 1 } })] },
          }),
        ],
      },
    }),
  },
  {
    name: "embedded-template-reorder-preserves-instance-order",
    base: base({
      embedded: {
        items: [
          baseChild({ sourceId: "tc1", system: { a: 1 } }),
          baseChild({ sourceId: "tc2", system: { b: 1 } }),
        ],
      },
    }),
    parent: doc({
      id: "t1",
      embedded: { items: [doc({ id: "tc1", system: { a: 1 } }), doc({ id: "tc2", system: { b: 1 } })] },
    }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      embedded: {
        items: [
          doc({ id: "ic2", source: fromTemplate("tc2"), system: { b: 1 } }),
          doc({ id: "ic1", source: fromTemplate("tc1"), system: { a: 1 } }),
        ],
      },
    }),
  },
  {
    name: "embedded-token-child-placement",
    base: base({
      embedded: { items: [baseChild({ sourceId: "tc1", engine: { x: 0, hp: 1 } })] },
    }),
    parent: doc({
      id: "t1",
      embedded: { items: [doc({ id: "tc1", doc_type: "token", engine: { x: 9, hp: 2 } })] },
    }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      embedded: {
        items: [doc({ id: "ic1", doc_type: "token", source: fromTemplate("tc1"), engine: { x: 3, hp: 1 } })],
      },
    }),
  },
  {
    // The FIRST instance child is template-deleted-unchanged (dropped from the output) while
    // a LATER child conflicts: the conflict's index is the child's position in the OUTPUT
    // array being built, so the surviving first-out child is `/embedded/items/0`, not `1`.
    name: "embedded-conflict-index-after-drop",
    base: base({
      embedded: {
        items: [
          baseChild({ sourceId: "tc1", system: { hp: 1 } }),
          baseChild({ sourceId: "tc2", system: { hp: 1 } }),
        ],
      },
    }),
    parent: doc({ id: "t1", embedded: { items: [doc({ id: "tc2", system: { hp: 2 } })] } }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      embedded: {
        items: [
          doc({ id: "ic1", source: fromTemplate("tc1"), system: { hp: 1 } }),
          doc({ id: "ic2", source: fromTemplate("tc2"), system: { hp: 3 } }),
        ],
      },
    }),
  },
  {
    // A template-embedded child that ITSELF carries provenance keeps its stored
    // `source.version` through the restamp: the copy's new provenance reads
    // `{ id: <template child>, version: 3 }`, not the `?? 1` fallback.
    name: "embedded-restamp-version-passthrough",
    base: base({ embedded: { items: [baseChild({ sourceId: "tc1", system: { hp: 1 } })] } }),
    parent: doc({
      id: "t1",
      embedded: {
        items: [
          doc({ id: "tc1", system: { hp: 1 } }),
          doc({ id: "tc2", source: { id: "other-t", pack: null, version: 3 }, system: { k: 1 } }),
        ],
      },
    }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      embedded: { items: [doc({ id: "ic1", source: fromTemplate("tc1"), system: { hp: 1 } })] },
    }),
  },
  {
    // An instance child whose non-null `source.id` is FOREIGN (in neither the template's
    // child ids nor the base's membership records) is the third correlation shape: not
    // correlated at all, so it is kept as instance-added, exactly like a source-less child.
    name: "embedded-foreign-source-kept",
    base: base({ embedded: { items: [baseChild({ sourceId: "tc1", system: { hp: 1 } })] } }),
    parent: doc({ id: "t1", embedded: { items: [doc({ id: "tc1", system: { hp: 1 } })] } }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      embedded: {
        items: [
          doc({ id: "ic1", source: fromTemplate("tc1"), system: { hp: 1 } }),
          doc({ id: "ic2", source: fromTemplate("foreign1"), system: { own: true } }),
        ],
      },
    }),
  },
  {
    name: "resolve-takes-template",
    kind: "resolve",
    theirs: ["/system/hp"],
    base: base({ system: { hp: 1 } }),
    parent: doc({ id: "t1", system: { hp: 2 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: { hp: 3 } }),
  },
  {
    name: "resolve-keeps-mine",
    kind: "resolve",
    theirs: [],
    base: base({ system: { hp: 1 } }),
    parent: doc({ id: "t1", system: { hp: 2 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: { hp: 3 } }),
  },
  {
    name: "resolve-embedded-deletion-takes-template",
    kind: "resolve",
    theirs: ["/embedded/items/0"],
    base: base({ embedded: { items: [baseChild({ sourceId: "tc1", system: { hp: 1 } })] } }),
    parent: doc({ id: "t1", embedded: { items: [] } }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      embedded: { items: [doc({ id: "ic1", source: fromTemplate("tc1"), system: { hp: 9 } })] },
    }),
  },
  {
    name: "plan-absent-collection-old-null",
    kind: "resolve",
    theirs: [],
    base: null,
    parent: doc({ id: "t1", embedded: { items: [doc({ id: "tc1", system: { k: 1 } })] } }),
    child: doc({ id: "c1", source: fromTemplate("t1") }),
  },
  {
    name: "plan-empty-collection-skipped",
    kind: "resolve",
    theirs: [],
    base: base({}),
    parent: doc({ id: "t1", embedded: { items: [] } }),
    child: doc({ id: "c1", source: fromTemplate("t1") }),
  },
  {
    name: "plan-changed-bands-only",
    kind: "resolve",
    theirs: [],
    base: base({ name: "A", engine: { e: 1 }, system: { hp: 1 } }),
    parent: doc({ id: "t1", name: "A", engine: { e: 1 }, system: { hp: 2 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), name: "A", engine: { e: 1 }, system: { hp: 1 } }),
  },
  {
    // Taking the template's side of an ancestor/descendant conflict: the child deleted the
    // whole `a` object, so the merged bands lack the intermediate container and the resolve
    // must CREATE `/system/a` before writing `/system/a/b` (setPointer descent).
    name: "resolve-ancestor-delete-takes-template",
    kind: "resolve",
    theirs: ["/system/a/b"],
    base: base({ system: { a: { b: 1 } } }),
    parent: doc({ id: "t1", system: { a: { b: 2 } } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: {} }),
  },
  {
    // Taking the template's side of a top-level `parentKind: "delete"` conflict: the key is
    // REMOVED from the merged bands (deletePointer), not written.
    name: "resolve-parent-delete-takes-template",
    kind: "resolve",
    theirs: ["/system/hp"],
    base: base({ system: { hp: 1 } }),
    parent: doc({ id: "t1", system: {} }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: { hp: 3 } }),
  },
  {
    // Escaped-token keys resolved toward the template: `/system/a~1b` unescapes to the key
    // `a/b` and `/system/c~0d` to `c~d` (both escape forms) before the value is written.
    name: "resolve-escaped-keys-take-template",
    kind: "resolve",
    theirs: ["/system/a~1b", "/system/c~0d"],
    base: base({ system: { "a/b": 1, "c~d": 1 } }),
    parent: doc({ id: "t1", system: { "a/b": 2, "c~d": 2 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), system: { "a/b": 3, "c~d": 9 } }),
  },
  {
    // Two embedded collections change in one merge: the update's collection changes come in
    // sorted collection-key order (`items` before `spells`), pinning both the sorted
    // collection iteration and `planToUpdate`'s changes-array order.
    name: "plan-multi-collection-order",
    kind: "resolve",
    theirs: [],
    base: base({ embedded: { items: [], spells: [] } }),
    parent: doc({
      id: "t1",
      embedded: {
        spells: [doc({ id: "ts1", system: { s: 1 } })],
        items: [doc({ id: "ti1", system: { i: 1 } })],
      },
    }),
    child: doc({ id: "c1", source: fromTemplate("t1"), embedded: { items: [], spells: [] } }),
  },
  {
    name: "revert-bands-take-template",
    kind: "revert",
    base: base({ name: "T", engine: { x: 99, hp: 5 }, system: { s: 1 } }),
    parent: doc({ id: "t1", doc_type: "token", name: "T", engine: { x: 99, hp: 5 }, system: { s: 1 } }),
    child: doc({
      id: "c1",
      doc_type: "token",
      source: fromTemplate("t1"),
      name: "C",
      engine: { x: 3, hp: 8 },
      system: { s: 2, extra: true },
    }),
  },
  {
    name: "revert-drops-local-additions",
    kind: "revert",
    base: base({ embedded: { items: [] } }),
    parent: doc({ id: "t1", embedded: { items: [doc({ id: "tc1", system: { k: 1 } })] } }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      embedded: { items: [doc({ id: "local1", system: { own: 1 } })] },
    }),
  },
  {
    name: "revert-restores-locally-deleted",
    kind: "revert",
    base: base({ embedded: { items: [baseChild({ sourceId: "tc1", system: { k: 1 } })] } }),
    parent: doc({ id: "t1", embedded: { items: [doc({ id: "tc1", system: { k: 1 } })] } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), embedded: { items: [] } }),
  },
  {
    // Revert at embedded depth 2: the recursion must reset each level against the TEMPLATE
    // side (template-first argument order) — a locally edited grandchild resets to the
    // template grandchild's bands, a locally added grandchild is dropped, and a locally
    // deleted grandchild is restamped back in.
    name: "revert-nested-embedded",
    kind: "revert",
    base: base({
      embedded: {
        items: [
          baseChild({
            sourceId: "tc1",
            system: { a: 5 },
            embedded: {
              sub: [
                baseChild({ sourceId: "gc1", system: { deep: 7 } }),
                baseChild({ sourceId: "gc2", system: { deep: 8 } }),
              ],
            },
          }),
        ],
      },
    }),
    parent: doc({
      id: "t1",
      embedded: {
        items: [
          doc({
            id: "tc1",
            system: { a: 5 },
            embedded: {
              sub: [doc({ id: "gc1", system: { deep: 7 } }), doc({ id: "gc2", system: { deep: 8 } })],
            },
          }),
        ],
      },
    }),
    child: doc({
      id: "c1",
      source: fromTemplate("t1"),
      embedded: {
        items: [
          doc({
            id: "ic1",
            source: fromTemplate("tc1"),
            system: { a: 9 },
            embedded: {
              sub: [
                doc({ id: "igc1", source: fromTemplate("gc1"), system: { deep: 1 } }),
                doc({ id: "ilocal", system: { own: 1 } }),
              ],
            },
          }),
        ],
      },
    }),
  },
  {
    name: "revert-no-op-still-emits-base",
    kind: "revert",
    base: base({ name: "T", system: { s: 1 } }),
    parent: doc({ id: "t1", name: "T", system: { s: 1 } }),
    child: doc({ id: "c1", source: fromTemplate("t1"), name: "T", system: { s: 1 } }),
  },
];

/** Every envelope id reachable from `docs`, recursively through `embedded` collections. */
function collectIds(docs: WireDocument[]): Set<string> {
  const out = new Set<string>();
  const walk = (d: WireDocument): void => {
    out.add(d.id);
    for (const kids of Object.values(d.embedded)) kids.forEach(walk);
  };
  docs.forEach(walk);
  return out;
}

/** A non-null, non-array JSON object. */
function isObject(v: unknown): v is Record<string, unknown> {
  return v !== null && typeof v === "object" && !Array.isArray(v);
}

/** Whether `v` is a full document envelope (as opposed to a payload object or base record). */
function isEnvelope(v: unknown): v is Record<string, unknown> {
  return isObject(v) && typeof v.id === "string" && typeof v.doc_type === "string";
}

/** The normalization both modes apply before writing or comparing: any envelope `id` not in
 * `known` is a freshly minted `restampSubtree` uuid and becomes `restamped-N` in first-seen
 * traversal order (envelope `id` before its subtrees, object keys sorted, arrays positional);
 * every later string occurrence of the same uuid (e.g. a `source.id` pointing at it) rewrites
 * to the same placeholder, and `null`-valued `base`/`engine` keys on envelopes are dropped
 * (absent and null are interchangeable to the merge's `?? null` coalescing). */
function normalize(value: unknown, known: Set<string>): unknown {
  const json = JSON.parse(JSON.stringify(value)) as unknown;
  const restamped = new Map<string, string>();
  const assign = (v: unknown): void => {
    if (Array.isArray(v)) {
      for (const e of v) assign(e);
      return;
    }
    if (!isObject(v)) return;
    if (isEnvelope(v) && !known.has(v.id as string) && !restamped.has(v.id as string)) {
      restamped.set(v.id as string, `restamped-${restamped.size + 1}`);
    }
    for (const k of Object.keys(v).sort()) assign(v[k]);
  };
  assign(json);
  const rewrite = (v: unknown): unknown => {
    if (typeof v === "string") return restamped.get(v) ?? v;
    if (Array.isArray(v)) return v.map(rewrite);
    if (!isObject(v)) return v;
    const out: Record<string, unknown> = {};
    for (const [k, val] of Object.entries(v)) {
      if (isEnvelope(v) && (k === "base" || k === "engine") && val === null) continue;
      out[k] = rewrite(val);
    }
    return out;
  };
  return rewrite(json);
}

/** Run one case through the engine and normalize the output. `pull` runs `computePull`;
 * `resolve` additionally applies `theirs` and emits `planToUpdate`; `revert` runs
 * `computeRevert`. The case's `base` becomes the child's stored snapshot (`null` = none). */
function runCase(c: CorpusCase): Record<string, unknown> {
  const child = structuredClone(c.child);
  if (c.base !== null) child.base = structuredClone(c.base);
  const parent = structuredClone(c.parent);
  const known = collectIds([child, parent]);
  const kind = c.kind ?? "pull";
  if (kind === "revert") {
    return normalize({ update: computeRevert(child, parent) }, known) as Record<string, unknown>;
  }
  const plan = computePull(child, parent);
  if (kind === "pull") {
    return normalize({ mergedBands: plan.mergedBands, conflicts: plan.conflicts }, known) as Record<
      string,
      unknown
    >;
  }
  const resolved = applyResolutions(plan.mergedBands, plan.conflicts, new Set(c.theirs ?? []));
  return normalize(
    { mergedBands: plan.mergedBands, conflicts: plan.conflicts, update: planToUpdate(child, parent, resolved) },
    known,
  ) as Record<string, unknown>;
}

/** Serialize one authored case's INPUTS for the fixture (no expected output). Shared by
 * generation mode (which appends `expect`) and assertion mode's provenance check, so a
 * stale or hand-edited fixture fails identically in both. */
function serializeInputs(c: CorpusCase): Omit<CorpusCase, "expect"> {
  const known = collectIds([c.child, c.parent]);
  return {
    name: c.name,
    ...(c.kind !== undefined && c.kind !== "pull" ? { kind: c.kind } : {}),
    base: c.base === null ? null : (normalize(c.base, known) as MergeBase),
    parent: normalize(c.parent, known) as WireDocument,
    child: normalize(c.child, known) as WireDocument,
    ...(c.theirs !== undefined ? { theirs: c.theirs } : {}),
  };
}

/** Serialize one authored case for the fixture: normalized inputs plus its expected output. */
function serializeCase(c: CorpusCase): CorpusCase {
  return { ...serializeInputs(c), expect: runCase(c) };
}

describe("merge conformance corpus", () => {
  if (GENERATE) {
    it("regenerates the fixture from the case matrix", () => {
      const corpus: Corpus = { cases: CASES.map(serializeCase) };
      writeFileSync(FIXTURE_URL, `${JSON.stringify(corpus, null, 2)}\n`);
    });
    return;
  }

  const corpus = JSON.parse(readFileSync(FIXTURE_URL, "utf8")) as Corpus;

  it("has unique case names", () => {
    expect(new Set(corpus.cases.map((c) => c.name)).size).toBe(corpus.cases.length);
  });

  it("covers every case kind", () => {
    const kinds = new Set(corpus.cases.map((c) => c.kind ?? "pull"));
    expect(kinds).toEqual(new Set<CaseKind>(["pull", "resolve", "revert"]));
  });

  it("stores inputs identical to the case matrix (no stale or hand-edited fixture)", () => {
    const stored = corpus.cases.map((c) => {
      const inputs = { ...c };
      delete inputs.expect;
      return inputs;
    });
    expect(stored).toEqual(CASES.map(serializeInputs));
  });

  for (const c of corpus.cases) {
    it(`${c.kind ?? "pull"}: ${c.name}`, () => {
      expect(runCase(c)).toEqual(c.expect);
    });
  }
});
