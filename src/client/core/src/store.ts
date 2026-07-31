// The single, authoritative client document store: a faithful mirror of one
// world's document tree, updated by applying confirmed commands. It is the
// `base` the optimistic layer predicts on top of.
import type { WireCommand, WireDocument, WireOperation } from "./wire";
import { DocumentSchema } from "./wire";

/** Set `value` at JSON-pointer `pointer` in `root`, mirroring the server's
 * set-only `set_pointer` (creates intermediate objects; array indices replace).
 * A non-empty pointer must start with "/".
 * @param root The document (sub)tree to mutate in place.
 * @param pointer A JSON pointer (`/a/b/0`); must be non-empty and start with `/`.
 * @param value The value to write at `pointer`.
 * @example
 * ```ts
 * import { setPointer } from "@shadowcat/core";
 *
 * const doc: Record<string, unknown> = { system: {} };
 * setPointer(doc, "/system/hp", 10);
 * ```
 */
export function setPointer(
  root: unknown,
  pointer: string,
  value: unknown,
): void {
  if (pointer === "") {
    throw new Error("empty JSON pointer cannot target a field");
  }
  if (!pointer.startsWith("/")) {
    throw new Error(`invalid JSON pointer: ${pointer}`);
  }
  const tokens = pointer
    .split("/")
    .slice(1)
    .map((t) => t.replace(/~1/g, "/").replace(/~0/g, "~"));

  let cur: unknown = root;
  for (const tok of tokens.slice(0, -1)) {
    if (Array.isArray(cur)) {
      cur = cur[Number(tok)];
    } else if (cur !== null && typeof cur === "object") {
      const obj = cur as Record<string, unknown>;
      // An explicit `null` intermediate (e.g. a scene `vision`/`lighting` field with no
      // omit-when-absent behavior) descends the same as a missing key: getPointer/removePointer
      // already treat null == absent for reads/removes, so set now agrees for the
      // intermediate-descent case. Leaf null-vs-absent (the final-token branch below) is unchanged.
      if (!(tok in obj) || obj[tok] === null) obj[tok] = {};
      cur = obj[tok];
    } else {
      throw new Error(`cannot descend into non-container at ${pointer}`);
    }
  }
  const last = tokens[tokens.length - 1];
  if (Array.isArray(cur)) {
    // Match the server: an out-of-range or non-integer array index is rejected,
    // never a silent sparse extension.
    const idx = Number(last);
    if (!Number.isInteger(idx) || idx < 0 || idx >= cur.length) {
      throw new Error(`array index out of range at ${pointer}`);
    }
    cur[idx] = value;
  } else if (cur !== null && typeof cur === "object") {
    (cur as Record<string, unknown>)[last] = value;
  } else {
    throw new Error(`cannot set field on non-container at ${pointer}`);
  }
}

/** Remove the object key at JSON-pointer `pointer` in `root`, mirroring the server's
 * `remove_pointer`: object keys only; removing an already-absent key — or one beneath an
 * absent OR explicit-`null` intermediate — is a silent no-op (no intermediate is created).
 * Array-index removal throws (an array shrinks via whole-array replacement, never a leaf
 * remove). A non-empty pointer must start with "/".
 * @param root The document (sub)tree to mutate in place.
 * @param pointer A JSON pointer to an object key (`/a/b`); must be non-empty and start with `/`.
 * @example
 * ```ts
 * import { removePointer } from "@shadowcat/core";
 *
 * const doc: Record<string, unknown> = { system: { hp: 10 } };
 * removePointer(doc, "/system/hp"); // { system: {} } — key genuinely absent, not null
 * ```
 */
export function removePointer(root: unknown, pointer: string): void {
  if (pointer === "") {
    throw new Error("empty JSON pointer cannot target a field");
  }
  if (!pointer.startsWith("/")) {
    throw new Error(`invalid JSON pointer: ${pointer}`);
  }
  const tokens = pointer
    .split("/")
    .slice(1)
    .map((t) => t.replace(/~1/g, "/").replace(/~0/g, "~"));

  let cur: unknown = root;
  for (const tok of tokens.slice(0, -1)) {
    if (Array.isArray(cur)) {
      const idx = Number(tok);
      if (!Number.isInteger(idx) || idx < 0 || idx >= cur.length) return; // absent → no-op
      cur = cur[idx];
    } else if (cur !== null && typeof cur === "object") {
      const obj = cur as Record<string, unknown>;
      // Absent OR explicit-null intermediate: nothing lives beneath a null, so the target is
      // already absent and removal is a no-op — uniform with getPointer/setPointer, which also
      // treat a null intermediate as absent.
      if (!(tok in obj) || obj[tok] === null) return;
      cur = obj[tok];
    } else {
      throw new Error(`cannot descend into non-container at ${pointer}`);
    }
  }
  const last = tokens[tokens.length - 1];
  if (Array.isArray(cur)) {
    throw new Error(`cannot remove an array index at ${pointer}`);
  } else if (cur !== null && typeof cur === "object") {
    delete (cur as Record<string, unknown>)[last];
  } else {
    throw new Error(`cannot remove field on non-container at ${pointer}`);
  }
}

/** Reads the value at JSON-pointer `pointer` in `root`; `undefined` for any missing
 * segment or out-of-range array index. Never throws (the read-only mirror of
 * `setPointer`). An empty pointer returns `root`.
 * @param root The document (sub)tree to read from.
 * @param pointer A JSON pointer (`/a/b/0`); the empty string returns `root` itself.
 * @returns The value at `pointer`, or `undefined` if any segment is missing/out of range.
 * @example
 * ```ts
 * import { getPointer } from "@shadowcat/core";
 *
 * getPointer({ system: { hp: 10 } }, "/system/hp"); // 10
 * getPointer({ system: {} }, "/system/missing"); // undefined
 * ```
 */
export function getPointer(root: unknown, pointer: string): unknown {
  if (pointer === "") return root;
  const tokens = pointer.split("/").slice(1).map((t) => t.replace(/~1/g, "/").replace(/~0/g, "~"));
  let cur: unknown = root;
  for (const tok of tokens) {
    if (Array.isArray(cur)) {
      const idx = Number(tok);
      if (!Number.isInteger(idx) || idx < 0 || idx >= cur.length) return undefined;
      cur = cur[idx];
    } else if (cur !== null && typeof cur === "object") {
      cur = (cur as Record<string, unknown>)[tok];
    } else {
      return undefined;
    }
  }
  return cur;
}

/** Apply one operation to a document map (mutates it). Update clones the target
 * before mutating, so callers sharing document refs are not affected.
 * @param docs The document map to mutate in place, keyed by document id.
 * @param op The operation: `create` inserts, `delete` removes, `update` applies each
 * `FieldChange` in order via `setPointer`/`removePointer` and re-validates the result.
 * @example
 * ```ts
 * import { applyOperation } from "@shadowcat/core";
 * import type { WireOperation, WireDocument } from "@shadowcat/core";
 *
 * declare const docs: Map<string, WireDocument>;
 * declare const op: WireOperation;
 * applyOperation(docs, op);
 * ```
 */
export function applyOperation(
  docs: Map<string, WireDocument>,
  op: WireOperation,
): void {
  switch (op.op) {
    case "create":
      docs.set(op.doc.id, op.doc);
      break;
    case "delete":
      docs.delete(op.doc.id);
      break;
    case "update": {
      const cur = docs.get(op.doc_id);
      if (!cur) return; // unknown doc (e.g. not yet resynced); server is authoritative
      const whole = structuredClone(cur) as unknown;
      for (const ch of op.changes) {
        if (ch.remove) removePointer(whole, ch.path);
        else setPointer(whole, ch.path, ch.new);
      }
      // Re-validate: a parse failure signals client/server schema drift.
      docs.set(op.doc_id, DocumentSchema.parse(whole));
      break;
    }
  }
}

export type Listener = () => void;

/** Read-only document source the render engine consumes. Satisfied by both the
 * authoritative `DocumentStore` and the `OptimisticClient` view, so the canvas can
 * render predicted (unconfirmed) documents for immediate placement/move feedback while
 * the authoritative store remains the rollback base. `appliedSeq` (confirmed-seq on
 * both) backs the derived-frame watermark. */
export interface ReadableDocuments {
  query(docType: string): WireDocument[];
  get(id: string): WireDocument | undefined;
  subscribe(listener: Listener): () => void;
  readonly appliedSeq: number;
}

/** Authoritative mirror of one world's documents. */
export class DocumentStore implements ReadableDocuments {
  private docs = new Map<string, WireDocument>();
  private listeners = new Set<Listener>();
  /** Highest authoritative seq applied. */
  appliedSeq = 0;

  /** Apply a confirmed, sequenced command, then notify subscribers.
   * @param cmd An authoritative, server-sequenced command (wire `WsClient.onCommand`
   * to this — see `shadowcat-codebase-realtime-sync`).
   * @example
   * ```ts
   * import { DocumentStore } from "@shadowcat/core";
   * import type { WireCommand } from "@shadowcat/core";
   *
   * const store = new DocumentStore();
   * declare const cmd: WireCommand;
   * store.applyCommand(cmd);
   * ```
   */
  applyCommand(cmd: WireCommand): void {
    for (const op of cmd.ops) applyOperation(this.docs, op);
    this.appliedSeq = cmd.seq;
    this.emit();
  }

  /** Look up one document by id.
   * @param id The document id.
   * @returns The document, or `undefined` if not present in this world's store.
   * @example
   * ```ts
   * import { DocumentStore } from "@shadowcat/core";
   *
   * const store = new DocumentStore();
   * store.get("00000000-0000-0000-0000-000000000001"); // undefined until applyCommand creates it
   * ```
   */
  get(id: string): WireDocument | undefined {
    return this.docs.get(id);
  }

  /** All documents of one `doc_type`.
   * @param docType The `doc_type` string to filter on (e.g. `"actor"`, `"scene"`).
   * @returns Every stored document whose `doc_type` matches, in no particular order.
   * @example
   * ```ts
   * import { DocumentStore } from "@shadowcat/core";
   *
   * const store = new DocumentStore();
   * store.query("actor"); // []
   * ```
   */
  query(docType: string): WireDocument[] {
    return [...this.docs.values()].filter((d) => d.doc_type === docType);
  }

  /** Every document currently held.
   * @returns All stored documents, in no particular order.
   * @example
   * ```ts
   * import { DocumentStore } from "@shadowcat/core";
   *
   * const store = new DocumentStore();
   * store.snapshot(); // []
   * ```
   */
  snapshot(): WireDocument[] {
    return [...this.docs.values()];
  }

  /** Subscribe to any change; returns an unsubscribe.
   * @param listener Called (with no arguments) after every `applyCommand`.
   * @returns A function that removes `listener`.
   * @example
   * ```ts
   * import { DocumentStore } from "@shadowcat/core";
   *
   * const store = new DocumentStore();
   * const unsubscribe = store.subscribe(() => console.log("changed"));
   * unsubscribe();
   * ```
   */
  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Notify every subscriber. Private: called only from `applyCommand`.
   * @example
   * ```
   * // internal; not part of the public API — invoked only from applyCommand
   * this.emit();
   * ```
   */
  private emit(): void {
    for (const fn of this.listeners) fn();
  }
}
