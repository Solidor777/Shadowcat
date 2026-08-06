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

/** One unconfirmed local prediction, queued FIFO awaiting its authored echo or a reject. */
interface Pending {
  /** The id the matching Intent frame was sent under. */
  intentId: string;
  /** The predicted operations, applied in order to build the view. */
  ops: WireOperation[];
}

/** The optimistic (predicted) document view: `base` (confirmed authoritative commands)
 * plus every `pending` intent applied in FIFO order. This is the `ReadableDocuments`
 * the canvas/render engine reads from (`AppContext.documents`) — the UI renders the
 * OPTIMISTIC view, not the raw authoritative `DocumentStore`, so a locally-predicted
 * create/move/edit is visible immediately, before server confirmation. `appliedSeq` is
 * kept identical to the authoritative watermark (set only from confirmed `WireCommand.seq`,
 * never from a pending prediction), so a consumer deriving a frame from `appliedSeq` sees
 * the same watermark regardless of which `ReadableDocuments` it reads.
 * @example
 * ```ts
 * import { OptimisticClient } from "@shadowcat/core";
 *
 * const client = new OptimisticClient("00000000-0000-0000-0000-000000000001");
 * client.query("actor"); // []
 * ```
 */
export class OptimisticClient implements ReadableDocuments {
  /** Documents built from confirmed authoritative commands only. */
  private base = new Map<string, WireDocument>();
  /** `base` with every remaining `pending` intent applied, in order — what callers read. */
  private view = new Map<string, WireDocument>();
  /** Unconfirmed local predictions, oldest first. */
  private pending: Pending[] = [];
  /** Registered view-change subscribers. */
  private listeners = new Set<Listener>();
  /** The highest confirmed-seq applied to `base`; never advanced by a pending prediction. */
  appliedSeq = 0;

  /** `self` is the actor id used to recognize our own authored echoes.
   * @param self The connection's own user/actor id — matched against `WireCommand.author`
   * to recognize our own authored echoes and confirm the oldest pending intent.
   * @param logger Diagnostic sink for a dropped optimistic intent; defaults to `silentLogger`.
   * @example
   * ```ts
   * import { OptimisticClient } from "@shadowcat/core";
   *
   * const client = new OptimisticClient("00000000-0000-0000-0000-000000000001");
   * ```
   */
  constructor(
    private readonly self: string,
    private readonly logger: Logger = silentLogger,
  ) {}

  /** Apply an authoritative command (wire `WsClient.onCommand` to this).
   * @param cmd An authoritative, server-sequenced command.
   * @example
   * ```ts
   * import { OptimisticClient } from "@shadowcat/core";
   * import type { WireCommand } from "@shadowcat/core";
   *
   * const client = new OptimisticClient("00000000-0000-0000-0000-000000000001");
   * declare const cmd: WireCommand;
   * client.applyCommand(cmd);
   * ```
   */
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

  /** Discard a rejected intent's prediction (wire `WsClient.onReject` to this).
   * @param intentId The id of the intent the server rejected.
   * @example
   * ```ts
   * import { OptimisticClient } from "@shadowcat/core";
   *
   * const client = new OptimisticClient("00000000-0000-0000-0000-000000000001");
   * client.reject("intent-1"); // no-op if intentId isn't (or is no longer) pending
   * ```
   */
  reject(intentId: string): void {
    const i = this.pending.findIndex((p) => p.intentId === intentId);
    // No match means a correlation/reconnect mismatch (the echo already shifted
    // it); nothing to roll back, and no view change to broadcast.
    if (i < 0) return;
    this.pending.splice(i, 1);
    this.rebuildView();
  }

  /** Predict `ops` locally under `intentId` (the caller sends the Intent).
   * @param intentId The id the caller is sending the matching Intent frame under.
   * @param ops The operations to predict; applied in order, queued FIFO behind any
   * already-pending intents.
   * @example
   * ```ts
   * import { OptimisticClient } from "@shadowcat/core";
   * import type { WireOperation } from "@shadowcat/core";
   *
   * const client = new OptimisticClient("00000000-0000-0000-0000-000000000001");
   * declare const ops: WireOperation[];
   * client.applyIntent("intent-1", ops);
   * ```
   */
  applyIntent(intentId: string, ops: WireOperation[]): void {
    this.pending.push({ intentId, ops });
    this.rebuildView();
  }

  /** Outstanding (unconfirmed) intent ids, oldest first.
   * @returns The pending intent ids in FIFO confirm/reject order.
   * @example
   * ```ts
   * import { OptimisticClient } from "@shadowcat/core";
   *
   * const client = new OptimisticClient("00000000-0000-0000-0000-000000000001");
   * client.pendingIntents(); // []
   * ```
   */
  pendingIntents(): string[] {
    return this.pending.map((p) => p.intentId);
  }

  /** Look up one document in the optimistic view (base + pending intents applied).
   * @param id The document id.
   * @returns The document, or `undefined` if not present in the current view.
   * @example
   * ```ts
   * import { OptimisticClient } from "@shadowcat/core";
   *
   * const client = new OptimisticClient("00000000-0000-0000-0000-000000000001");
   * client.get("00000000-0000-0000-0000-000000000001"); // undefined until applyCommand/applyIntent creates it
   * ```
   */
  get(id: string): WireDocument | undefined {
    return this.view.get(id);
  }

  /** All documents of one `doc_type` in the optimistic view.
   * @param docType The `doc_type` string to filter on (e.g. `"actor"`, `"scene"`).
   * @returns Every document in the current view whose `doc_type` matches, in no particular order.
   * @example
   * ```ts
   * import { OptimisticClient } from "@shadowcat/core";
   *
   * const client = new OptimisticClient("00000000-0000-0000-0000-000000000001");
   * client.query("actor"); // []
   * ```
   */
  query(docType: string): WireDocument[] {
    return [...this.view.values()].filter((d) => d.doc_type === docType);
  }

  /** Every document currently in the optimistic view.
   * @returns All documents in the current view, in no particular order.
   * @example
   * ```ts
   * import { OptimisticClient } from "@shadowcat/core";
   *
   * const client = new OptimisticClient("00000000-0000-0000-0000-000000000001");
   * client.snapshot(); // []
   * ```
   */
  snapshot(): WireDocument[] {
    return [...this.view.values()];
  }

  /** Subscribe to any view change (confirm, reject, or new prediction); returns an unsubscribe.
   * @param listener Called (with no arguments) after every `applyCommand`/`reject`/`applyIntent`.
   * @returns A function that removes `listener`.
   * @example
   * ```ts
   * import { OptimisticClient } from "@shadowcat/core";
   *
   * const client = new OptimisticClient("00000000-0000-0000-0000-000000000001");
   * const unsubscribe = client.subscribe(() => console.log("view changed"));
   * unsubscribe();
   * ```
   */
  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Recompute `view` from `base` + every remaining `pending` intent, in order. Shares
   * unchanged doc refs with `base`; `applyOperation` clones on update, so pending updates
   * never mutate `base`. Each pending intent is applied to a scratch copy of the running
   * view and adopted ONLY if every one of its ops succeeds — a throwing op (e.g. a
   * stale/malformed path) is isolated to its own intent rather than aborting the whole
   * rebuild, which would otherwise wedge every later `applyIntent`/`applyCommand`/`reject`
   * call on this instance (they all route through `rebuildView`). The failed intent stays
   * in `pending`; only the server's confirm/reject removes it — this method never mutates
   * `pending`. Private: called from `applyCommand`, `reject`, and `applyIntent`.
   * @example
   * ```
   * // internal; not part of the public API — invoked after every base/pending change
   * this.rebuildView();
   * ```
   */
  private rebuildView(): void {
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
