// Optimistic-apply + rollback over the authoritative stream.
//
// The visible state is `view = base + ordered pending intents`:
//   base    — everything applied from confirmed authoritative commands.
//   pending — intents applied locally but not yet confirmed/rejected.
//   view    — base with all pending ops applied, in order (what callers read).
//
// applyIntent predicts locally (push pending, rebuild view). A command authored
// by us confirms the oldest pending intent (FIFO) — its effect is now in base,
// so it leaves pending. A reject simply drops the pending entry. Rollback is
// therefore "recompute view from base + remaining pending"; no inverse ops are
// needed on the client (the M2 reversible representation backs server-side
// rollback / undo, not this local prediction). The server stays authoritative:
// optimism is a prediction, replaced by `base` on confirm or discarded on reject.
import { applyOperation, type Listener, type ReadableDocuments } from "./store";
import type { WireCommand, WireDocument, WireOperation } from "./wire";
import { silentLogger, type Logger } from "./logger";

interface Pending {
  intentId: string;
  ops: WireOperation[];
}

export class OptimisticClient implements ReadableDocuments {
  private base = new Map<string, WireDocument>();
  private view = new Map<string, WireDocument>();
  private pending: Pending[] = [];
  private listeners = new Set<Listener>();
  appliedSeq = 0;

  /** `self` is the actor id used to recognize our own authored echoes. */
  constructor(
    private readonly self: string,
    private readonly logger: Logger = silentLogger,
  ) {}

  /** Apply an authoritative command (wire `WsClient.onCommand` to this). */
  applyCommand(cmd: WireCommand): void {
    for (const op of cmd.ops) applyOperation(this.base, op);
    this.appliedSeq = cmd.seq;
    // Our own authored echo confirms the oldest outstanding intent (FIFO):
    // its effect is now in base, so drop the prediction.
    if (cmd.author === this.self && this.pending.length > 0) {
      this.pending.shift();
    }
    this.rebuildView();
  }

  /** Discard a rejected intent's prediction (wire `WsClient.onReject` to this). */
  reject(intentId: string): void {
    const i = this.pending.findIndex((p) => p.intentId === intentId);
    // No match means a correlation/reconnect mismatch (the echo already shifted
    // it); nothing to roll back, and no view change to broadcast.
    if (i < 0) return;
    this.pending.splice(i, 1);
    this.rebuildView();
  }

  /** Predict `ops` locally under `intentId` (the caller sends the Intent). */
  applyIntent(intentId: string, ops: WireOperation[]): void {
    this.pending.push({ intentId, ops });
    this.rebuildView();
  }

  /** Outstanding (unconfirmed) intent ids, oldest first. */
  pendingIntents(): string[] {
    return this.pending.map((p) => p.intentId);
  }

  get(id: string): WireDocument | undefined {
    return this.view.get(id);
  }

  query(docType: string): WireDocument[] {
    return [...this.view.values()].filter((d) => d.doc_type === docType);
  }

  snapshot(): WireDocument[] {
    return [...this.view.values()];
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private rebuildView(): void {
    // Shares unchanged doc refs with base; applyOperation clones on update, so
    // pending updates never mutate base. Each pending intent is applied to a scratch
    // copy of the running view and adopted ONLY if every one of its ops succeeds — a
    // throwing op (e.g. a stale/malformed path) is isolated to its own intent rather
    // than aborting the whole rebuild, which would otherwise wedge every later
    // applyIntent/applyCommand/reject call on this instance (they all route through
    // rebuildView). The failed intent stays in `pending`; only the server's
    // confirm/reject removes it — this method never mutates `pending`.
    let view = new Map(this.base);
    for (const p of this.pending) {
      const scratch = new Map(view);
      try {
        for (const op of p.ops) applyOperation(scratch, op);
        view = scratch;
      } catch (err) {
        this.logger.warn("dropping optimistic intent that failed to apply", {
          intentId: p.intentId,
          err,
        });
      }
    }
    this.view = view;
    for (const fn of this.listeners) fn();
  }
}
