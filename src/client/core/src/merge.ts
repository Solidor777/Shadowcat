// Pure, order-independent 3-way merge primitives (client-core). The server never merges;
// the merge is computed here and applied as an ordinary batched `Update` (M13e). Every value
// is plain JSON (objects recurse key-by-key, arrays are opaque leaves, scalars are leaves).
import { setPointer, getPointer } from "./store";

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
