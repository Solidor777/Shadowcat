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

/** Builds the note tree from the recipient's own redacted document list: a root is a note whose
 * `parent_id` is `null` OR names a document absent from `notes` (a child whose parent this
 * recipient cannot read is promoted to root, never hidden — a readable note stays reachable).
 * Siblings at every level are ordered by `(engine.sort, created_at)`.
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
  const build = (doc: WireDocument): NoteTreeNode => ({
    doc,
    children: (childrenOf.get(doc.id) ?? []).sort(bySortThenCreated).map(build),
  });
  return roots.sort(bySortThenCreated).map(build);
}
