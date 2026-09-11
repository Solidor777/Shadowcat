import type { WireDocument } from "@shadowcat/core";

/** Shape of a note's `engine.sort` field, needed only for ordering — an `engine` is `unknown`
 * until narrowed, matching the sheet's own read-site convention. */
interface SortableEngine {
  /** `NoteEngine.sort`, read as `number` (the ts-rs `bigint` mapping never reaches the wire —
   * see `buildNoteDoc`'s doc comment). */
  sort?: number;
}

/** One node in the rendered note tree: the note document itself plus its already-built
 * children, ordered per `buildNoteTree`. */
export interface NoteTreeNode {
  /** The note document this node renders. */
  doc: WireDocument;
  /** This note's children, in display order. */
  children: NoteTreeNode[];
}

/** The `(engine.sort, created_at)` ordering pair for one note.
 * @param doc The note document.
 * @returns The sort key tuple.
 * @example
 * ```
 * // private helper; not part of the public API
 * declare const doc: WireDocument;
 * sortKey(doc);
 * ```
 */
const sortKey = (doc: WireDocument): [number, number] => [
  (doc.engine as SortableEngine | null | undefined)?.sort ?? 0,
  doc.created_at,
];

/** Comparator ordering two notes by `(engine.sort, created_at)`.
 * @param a The first note.
 * @param b The second note.
 * @returns Negative, zero, or positive per `Array.prototype.sort`'s contract.
 * @example
 * ```
 * // private helper; not part of the public API
 * declare const a: WireDocument, b: WireDocument;
 * bySortThenCreated(a, b);
 * ```
 */
const bySortThenCreated = (a: WireDocument, b: WireDocument): number => {
  const [as, ac] = sortKey(a);
  const [bs, bc] = sortKey(b);
  return as - bs || ac - bc;
};

/** The `(created_at, id)` tie-break used to deterministically pick which member of a cycle
 * `buildNoteTree` promotes to root — independent of `bySortThenCreated`, which orders siblings
 * for display rather than choosing a promotion candidate.
 * @param a The first note.
 * @param b The second note.
 * @returns Negative, zero, or positive per `Array.prototype.sort`'s contract.
 * @example
 * ```
 * // private helper; not part of the public API
 * declare const a: WireDocument, b: WireDocument;
 * byCreatedThenId(a, b);
 * ```
 */
const byCreatedThenId = (a: WireDocument, b: WireDocument): number =>
  a.created_at - b.created_at || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);

/** Builds the note tree from the recipient's own redacted document list: a root is a note whose
 * `parent_id` is `null` OR names a document absent from `notes` (a child whose parent this
 * recipient cannot read is promoted to root, never hidden — a readable note stays reachable).
 * INVARIANT: a readable note is always reachable from the returned forest — including a note
 * whose `parent_id` participates in a cycle (two notes naming each other, or a note naming
 * itself). Normal ingress (`data::sqlite::notes::check_note_parent`) refuses a cyclic parent, but
 * a world-bundle import does not run that check, so a cycle can still reach the store. After
 * building from the notes above, any note the recursion never visited belongs to such a cycle;
 * this builder groups those notes by their mutual parent/child edges, promotes the
 * `(created_at, id)`-lowest member of each group to a root, and builds it under the same
 * visited-set guard `build` already threads — a child already visited (the earlier member of its
 * own cycle) is skipped rather than recursed into again, which is what breaks the cycle and keeps
 * this pass linear. Siblings at every level, including the promoted roots, are ordered by
 * `(engine.sort, created_at)`.
 * @param notes Every `note` document the caller's optimistic view currently holds.
 * @returns The root-level nodes, in display order.
 * @example
 * ```ts
 * import { buildNoteTree } from "./tree";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const notes: WireDocument[];
 * buildNoteTree(notes);
 * ```
 */
export function buildNoteTree(notes: WireDocument[]): NoteTreeNode[] {
  const byId = new Map(notes.map((n) => [n.id, n]));
  const childrenOf = new Map<string, WireDocument[]>();
  const roots: WireDocument[] = [];
  for (const n of notes) {
    if (n.parent_id && byId.has(n.parent_id)) {
      const list = childrenOf.get(n.parent_id) ?? [];
      list.push(n);
      childrenOf.set(n.parent_id, list);
    } else {
      roots.push(n);
    }
  }
  for (const list of childrenOf.values()) list.sort(bySortThenCreated);

  const visited = new Set<string>();
  const build = (doc: WireDocument): NoteTreeNode => {
    visited.add(doc.id);
    return {
      doc,
      children: (childrenOf.get(doc.id) ?? []).filter((c) => !visited.has(c.id)).map(build),
    };
  };

  const result = roots.sort(bySortThenCreated).map(build);

  // Every note not yet visited is unreachable from a real root, so (per the invariant above) it
  // must sit on a parent-pointer cycle. Group the remaining notes into their weakly-connected
  // components (parent AND child edges, restricted to unvisited members) and promote one root
  // per component, so a normal child hanging off a cycle node still attaches under it.
  const unvisited = new Set(notes.filter((n) => !visited.has(n.id)).map((n) => n.id));
  while (unvisited.size > 0) {
    const start = unvisited.values().next().value as string;
    const seen = new Set([start]);
    const stack = [start];
    while (stack.length > 0) {
      const id = stack.pop() as string;
      const doc = byId.get(id) as WireDocument;
      const parentId = doc.parent_id;
      if (parentId && unvisited.has(parentId) && !seen.has(parentId)) {
        seen.add(parentId);
        stack.push(parentId);
      }
      for (const child of childrenOf.get(id) ?? []) {
        if (unvisited.has(child.id) && !seen.has(child.id)) {
          seen.add(child.id);
          stack.push(child.id);
        }
      }
    }
    const component = [...seen].map((id) => byId.get(id) as WireDocument);
    for (const id of seen) unvisited.delete(id);
    component.sort(byCreatedThenId);
    result.push(build(component[0]));
  }

  return result.sort(bySortThenCreated);
}
