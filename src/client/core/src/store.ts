// The single, authoritative client document store: a faithful mirror of one
// world's document tree, updated by applying confirmed commands. It is the
// `base` the optimistic layer predicts on top of.
import type { WireCommand, WireDocument, WireOperation } from "./wire";
import { DocumentSchema } from "./wire";

/** Set `value` at JSON-pointer `pointer` in `root`, mirroring the server's
 * set-only `set_pointer` (creates intermediate objects; array indices replace).
 * A non-empty pointer must start with "/". */
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
      if (!(tok in obj)) obj[tok] = {};
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

/** Apply one operation to a document map (mutates it). Update clones the target
 * before mutating, so callers sharing document refs are not affected. */
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
      for (const ch of op.changes) setPointer(whole, ch.path, ch.new);
      // Re-validate: a parse failure signals client/server schema drift.
      docs.set(op.doc_id, DocumentSchema.parse(whole));
      break;
    }
  }
}

export type Listener = () => void;

/** Authoritative mirror of one world's documents. */
export class DocumentStore {
  private docs = new Map<string, WireDocument>();
  private listeners = new Set<Listener>();
  /** Highest authoritative seq applied. */
  appliedSeq = 0;

  /** Apply a confirmed, sequenced command, then notify subscribers. */
  applyCommand(cmd: WireCommand): void {
    for (const op of cmd.ops) applyOperation(this.docs, op);
    this.appliedSeq = cmd.seq;
    this.emit();
  }

  get(id: string): WireDocument | undefined {
    return this.docs.get(id);
  }

  query(docType: string): WireDocument[] {
    return [...this.docs.values()].filter((d) => d.doc_type === docType);
  }

  snapshot(): WireDocument[] {
    return [...this.docs.values()];
  }

  /** Subscribe to any change; returns an unsubscribe. */
  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private emit(): void {
    for (const fn of this.listeners) fn();
  }
}
