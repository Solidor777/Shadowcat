// WebSocket client over the M5 protocol: maintains an ordered application
// watermark (the client-side sequence guard), recovers gaps via ResyncRequest,
// reconnects with exponential backoff, and tracks the server time offset. It
// emits in-order commands and rejects to its handlers; wiring to the document
// store / optimistic engine is the caller's job.
import type { RejectReason } from "@shadowcat/types";
import {
  parseServerMsg,
  type ClientMsg,
  type ServerMsg,
  type WireCommand,
  type WireSearchHit,
} from "./wire";

/** A resolved page of search results (Core.search). */
export interface SearchPage {
  hits: WireSearchHit[];
  nextCursor?: string;
}

/** A resolved pathfind result (WsClient.pathfind). */
export interface PathResult {
  path: [number, number][];
  cost: number;
}

/** Handle to an active live search subscription (Core.subscribeSearch). */
export interface SubscriptionHandle {
  unsubscribe(): void;
}

/** A SceneDerived frame delivered to a scene subscription. */
export interface SceneFrame {
  payload: unknown;
  computedAtSeq: number;
}

/** Handle to an active SceneDerived subscription. */
export interface SceneSubscription {
  unsubscribe(): void;
}

/** The `Welcome` server frame (capability fields included). */
export type WireWelcome = Extract<ServerMsg, { type: "welcome" }>;
import type { Connect, Transport } from "./transport";

export interface WsClientHandlers {
  /** An in-order, sequence-guarded authoritative command (live or replayed). */
  onCommand(cmd: WireCommand): void;
  /** An intent the server refused. */
  onReject?(intentId: string, reason: RejectReason): void;
  onWelcome?(welcome: WireWelcome): void;
  /** Fires once per (re)connect after any resync replay is fully applied (or
   * immediately when no replay is needed). The seam for replaying actions queued
   * while offline, after the optimistic view has rebased onto authoritative state. */
  onResyncComplete?(): void;
  /** A command that failed to apply (e.g. schema drift). Surfaced, never thrown
   * into the socket loop. */
  onError?(error: unknown): void;
  /** An out-of-band asset mutation notice (replace/delete); carries no seq. */
  onAssetChanged?(msg: { uuid: string; op: "replaced" | "deleted" }): void;
  /** An out-of-band relayed location ping (carries no seq). */
  onScenePing?(msg: { scene: string; x: number; y: number; user: string }): void;
}

export interface WsClientOptions {
  connect: Connect;
  handlers: WsClientHandlers;
  now?: () => number;
  sleep?: (ms: number) => Promise<void>;
  backoffBaseMs?: number;
  backoffMaxMs?: number;
}

const defaultSleep = (ms: number): Promise<void> =>
  new Promise((r) => setTimeout(r, ms));

export class WsClient {
  private transport: Transport | null = null;
  private running_ = false;
  private reconnectAttempt = 0;
  /** Next seq to apply; the client-side ordering watermark. Persists across
   * reconnects so resync resumes from where application left off. */
  private nextExpected = 1;
  private serverOffsetMs = 0;
  /** In-flight correlated requests (search, pathfind), keyed by request_id. */
  private pending = new Map<
    string,
    {
      resolve: (result: SearchPage | PathResult) => void;
      reject: (e: Error) => void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();
  /** Active live search subscriptions, keyed by request_id; persists across
   * updates until unsubscribe/disconnect. */
  private subscriptions = new Map<string, (hits: WireSearchHit[]) => void>();
  /** Active scene subscriptions, keyed by request_id (ongoing onUpdate dispatch). */
  private sceneSubs = new Map<string, (frame: SceneFrame) => void>();
  /** In-flight scene-subscribe initial promises, keyed by request_id. */
  private scenePending = new Map<
    string,
    { resolve: (s: SceneSubscription) => void; reject: (e: Error) => void; timer: ReturnType<typeof setTimeout> }
  >();

  private readonly now: () => number;
  private readonly sleep: (ms: number) => Promise<void>;
  private readonly backoffBaseMs: number;
  private readonly backoffMaxMs: number;

  constructor(private readonly opts: WsClientOptions) {
    this.now = opts.now ?? Date.now;
    this.sleep = opts.sleep ?? defaultSleep;
    this.backoffBaseMs = opts.backoffBaseMs ?? 250;
    this.backoffMaxMs = opts.backoffMaxMs ?? 10_000;
  }

  /** Open the connection (and keep it open across drops until `stop`). */
  async start(): Promise<void> {
    this.running_ = true;
    await this.open();
  }

  stop(): void {
    this.running_ = false;
    this.transport?.close();
    this.transport = null;
    this.failPending("client stopped");
  }

  /** Reject every in-flight correlated request (e.g. on disconnect/stop): the
   * request was sent on a socket that will not answer it, so fail fast rather
   * than wait out the timeout (which would also outlive the connection). */
  private failPending(reason: string): void {
    for (const p of this.pending.values()) {
      clearTimeout(p.timer);
      p.reject(new Error(reason));
    }
    this.pending.clear();
    // Live subscriptions are bound to this socket; a reconnect does not replay
    // them, so drop them (the caller re-subscribes after reconnect if desired).
    this.subscriptions.clear();
    for (const p of this.scenePending.values()) {
      clearTimeout(p.timer);
      p.reject(new Error(reason));
    }
    this.scenePending.clear();
    // Scene subscriptions are bound to this socket; WorldSession re-subscribes on
    // the next Welcome, so drop them here.
    this.sceneSubs.clear();
  }

  /** Run a consumer callback in isolation: a throw is routed to `onError` and
   * never propagates into the socket message pump. A throw from `onError`
   * itself is swallowed so the pump cannot die. */
  private safeEmit(fn: () => void): void {
    try {
      fn();
    } catch (err) {
      try {
        this.opts.handlers.onError?.(err);
      } catch {
        // onError must not break the pump; ignore its failure.
      }
    }
  }

  /** Send a client frame (no-op if currently disconnected). */
  send(msg: ClientMsg): void {
    this.transport?.send(JSON.stringify(msg));
  }

  /** The highest authoritative seq applied. */
  get appliedSeq(): number {
    return this.nextExpected - 1;
  }

  /** True when a live transport is attached, so `send` will actually transmit. */
  get connected(): boolean {
    return this.transport !== null;
  }

  /** True between `start` and `stop`: a dropped transport will reconnect. Lets a
   * caller distinguish "reconnecting" (queue + retry) from "stopped" (give up). */
  get running(): boolean {
    return this.running_;
  }

  /** Estimated server clock. */
  serverNow(): number {
    return this.now() + this.serverOffsetMs;
  }

  private async open(): Promise<void> {
    if (!this.running_) return;
    try {
      this.transport = await this.opts.connect({
        onMessage: (d) => this.handleFrame(d),
        onClose: () => this.handleClose(),
      });
      this.reconnectAttempt = 0;
    } catch {
      this.scheduleReconnect();
    }
  }

  private handleClose(): void {
    this.transport = null;
    // In-flight requests were sent on the now-dead socket; a reconnect will not
    // replay them, so reject rather than leave them hanging until timeout.
    this.failPending("connection closed");
    if (this.running_) this.scheduleReconnect();
  }

  private scheduleReconnect(): void {
    const attempt = this.reconnectAttempt++;
    const ceiling = Math.min(
      this.backoffMaxMs,
      this.backoffBaseMs * 2 ** attempt,
    );
    const delay = ceiling * (0.5 + Math.random() * 0.5); // full jitter (half..full)
    void this.sleep(delay).then(() => this.open());
  }

  private handleFrame(text: string): void {
    const msg = parseServerMsg(text);
    if (!msg) return;
    switch (msg.type) {
      case "welcome":
        this.serverOffsetMs = msg.server_time - this.now();
        this.safeEmit(() => this.opts.handlers.onWelcome?.(msg));
        // Catch up anything applied-after our watermark (initial sync or a
        // reconnect gap). Idempotent: the server replays from from_seq.
        if (msg.current_seq >= this.nextExpected) {
          this.send({ type: "resync_request", from_seq: this.nextExpected });
          // onResyncComplete fires on the resulting resync_end.
        } else {
          // Already caught up: no replay will arrive, so signal completion now.
          this.safeEmit(() => this.opts.handlers.onResyncComplete?.());
        }
        break;
      case "event":
        this.applyEvent(msg.command);
        break;
      case "reject":
        this.safeEmit(() => this.opts.handlers.onReject?.(msg.intent_id, msg.reason));
        break;
      case "resync_begin":
        break;
      case "resync_end":
        this.nextExpected = Math.max(this.nextExpected, msg.current_seq + 1);
        this.safeEmit(() => this.opts.handlers.onResyncComplete?.());
        break;
      case "time_pong":
        this.serverOffsetMs = msg.server_t - this.now();
        break;
      case "ping":
        this.send({ type: "pong" });
        break;
      case "error":
        break;
      case "search_result": {
        const p = this.pending.get(msg.request_id);
        if (p) {
          clearTimeout(p.timer);
          this.pending.delete(msg.request_id);
          (p.resolve as (r: SearchPage) => void)({ hits: msg.hits, nextCursor: msg.next_cursor ?? undefined });
        }
        break;
      }
      case "search_error": {
        const p = this.pending.get(msg.request_id);
        if (p) {
          clearTimeout(p.timer);
          this.pending.delete(msg.request_id);
          p.reject(new Error(msg.message));
        }
        // A live subscription that errors server-side is dropped.
        this.subscriptions.delete(msg.request_id);
        break;
      }
      case "path_result": {
        const p = this.pending.get(msg.request_id);
        if (p) {
          clearTimeout(p.timer);
          this.pending.delete(msg.request_id);
          (p.resolve as (r: PathResult) => void)({ path: msg.path, cost: msg.cost });
        }
        break;
      }
      case "path_error": {
        const p = this.pending.get(msg.request_id);
        if (p) {
          clearTimeout(p.timer);
          this.pending.delete(msg.request_id);
          p.reject(new Error(msg.message));
        }
        break;
      }
      case "search_update": {
        const handler = this.subscriptions.get(msg.request_id);
        if (handler) this.safeEmit(() => handler(msg.hits));
        break;
      }
      case "asset_changed":
        this.safeEmit(() => this.opts.handlers.onAssetChanged?.({ uuid: msg.uuid, op: msg.op }));
        break;
      case "scene_ping":
        this.safeEmit(() =>
          this.opts.handlers.onScenePing?.({ scene: msg.scene, x: msg.x, y: msg.y, user: msg.user }),
        );
        break;
      case "scene_derived": {
        const handler = this.sceneSubs.get(msg.request_id);
        if (handler) this.safeEmit(() => handler({ payload: msg.payload, computedAtSeq: msg.computed_at_seq }));
        const init = this.scenePending.get(msg.request_id);
        if (init) {
          clearTimeout(init.timer);
          this.scenePending.delete(msg.request_id);
          init.resolve({
            unsubscribe: () => {
              this.sceneSubs.delete(msg.request_id);
              this.send({ type: "scene_unsubscribe", request_id: msg.request_id });
            },
          });
        }
        break;
      }
      case "scene_error": {
        const init = this.scenePending.get(msg.request_id);
        if (init) {
          clearTimeout(init.timer);
          this.scenePending.delete(msg.request_id);
          init.reject(new Error(msg.message));
        }
        this.sceneSubs.delete(msg.request_id);
        break;
      }
    }
  }

  /**
   * Core.search — issue a correlated full-text search request and resolve with
   * the page when the matching reply arrives. Rejects on a `search_error` frame
   * or after `timeoutMs`.
   */
  search(
    query: string,
    opts: { limit?: number; cursor?: string; timeoutMs?: number } = {},
  ): Promise<SearchPage> {
    const request_id = crypto.randomUUID();
    const timeoutMs = opts.timeoutMs ?? 10_000;
    return new Promise<SearchPage>((resolve, reject) => {
      if (!this.transport) {
        reject(new Error("not connected"));
        return;
      }
      const timer = setTimeout(() => {
        this.pending.delete(request_id);
        reject(new Error("search request timeout"));
      }, timeoutMs);
      this.pending.set(request_id, { resolve: resolve as (r: SearchPage | PathResult) => void, reject, timer });
      this.send({
        type: "search",
        request_id,
        query,
        limit: opts.limit ?? 20,
        cursor: opts.cursor,
        subscribe: false,
      });
    });
  }

  /**
   * Core.subscribeSearch — live top-N search. Resolves once the initial result
   * arrives (and fires `onUpdate` for it); subsequent server pushes fire
   * `onUpdate(hits)`. `unsubscribe()` stops updates and tells the server. On
   * disconnect the subscription is dropped and a pending initial rejects.
   */
  subscribeSearch(
    query: string,
    opts: { limit?: number; timeoutMs?: number },
    onUpdate: (hits: WireSearchHit[]) => void,
  ): Promise<SubscriptionHandle> {
    const request_id = crypto.randomUUID();
    const timeoutMs = opts.timeoutMs ?? 10_000;
    return new Promise<SubscriptionHandle>((resolve, reject) => {
      if (!this.transport) {
        reject(new Error("not connected"));
        return;
      }
      this.subscriptions.set(request_id, onUpdate);
      const timer = setTimeout(() => {
        this.pending.delete(request_id);
        this.subscriptions.delete(request_id);
        reject(new Error("subscribe request timeout"));
      }, timeoutMs);
      this.pending.set(request_id, {
        resolve: (page) => {
          this.safeEmit(() => onUpdate((page as SearchPage).hits));
          resolve({
            unsubscribe: () => {
              this.subscriptions.delete(request_id);
              this.send({ type: "unsubscribe", request_id });
            },
          });
        },
        reject,
        timer,
      });
      this.send({
        type: "search",
        request_id,
        query,
        limit: opts.limit ?? 20,
        cursor: undefined,
        subscribe: true,
      });
    });
  }

  /**
   * Subscribe to a SceneDerived channel. Resolves once the first frame arrives;
   * `onUpdate` fires for every frame. Rejects on `scene_error`, timeout, or no
   * transport. Dropped on disconnect (WorldSession re-subscribes on reconnect).
   */
  subscribeScene(
    channel: string,
    onUpdate: (frame: SceneFrame) => void,
    opts: { timeoutMs?: number; asUser?: string } = {},
  ): Promise<SceneSubscription> {
    const request_id = crypto.randomUUID();
    const timeoutMs = opts.timeoutMs ?? 10_000;
    return new Promise<SceneSubscription>((resolve, reject) => {
      if (!this.transport) {
        reject(new Error("not connected"));
        return;
      }
      this.sceneSubs.set(request_id, onUpdate);
      const timer = setTimeout(() => {
        this.scenePending.delete(request_id);
        this.sceneSubs.delete(request_id);
        reject(new Error("scene subscribe timeout"));
      }, timeoutMs);
      this.scenePending.set(request_id, { resolve, reject, timer });
      // `as_user` (GM-only see-as-player) is omitted unless set; the server gates + resolves it.
      this.send({ type: "scene_subscribe", request_id, channel, ...(opts.asUser ? { as_user: opts.asUser } : {}) });
    });
  }

  /**
   * Issue a correlated pathfind request and resolve with the computed path when
   * the matching `path_result` reply arrives. Rejects on a `path_error` frame or
   * after `timeoutMs`. The wire field is `footprint_radius`; the method param is
   * `footprintRadius` (camelCase per project convention).
   */
  pathfind(
    scene: string,
    start: [number, number],
    waypoints: [number, number][],
    footprintRadius: number,
    opts: { timeoutMs?: number } = {},
  ): Promise<PathResult> {
    const request_id = crypto.randomUUID();
    const timeoutMs = opts.timeoutMs ?? 10_000;
    return new Promise<PathResult>((resolve, reject) => {
      if (!this.transport) {
        reject(new Error("not connected"));
        return;
      }
      const timer = setTimeout(() => {
        this.pending.delete(request_id);
        reject(new Error("pathfind request timeout"));
      }, timeoutMs);
      this.pending.set(request_id, { resolve: resolve as (r: SearchPage | PathResult) => void, reject, timer });
      this.send({ type: "pathfind", request_id, scene, start, waypoints, footprint_radius: footprintRadius });
    });
  }

  private applyEvent(cmd: WireCommand): void {
    if (cmd.seq < this.nextExpected) return; // duplicate / already applied
    if (cmd.seq > this.nextExpected) {
      // Gap: request the missing prefix; the replay delivers it in order.
      this.send({ type: "resync_request", from_seq: this.nextExpected });
      return;
    }
    try {
      this.opts.handlers.onCommand(cmd);
    } catch (err) {
      // A failed apply (e.g. schema drift, itself a build-time failure via the
      // ts-rs CI sync) must not kill the socket loop. Surface it; still advance
      // so we don't resync-loop on an unrecoverable frame.
      this.opts.handlers.onError?.(err);
    }
    this.nextExpected = cmd.seq + 1;
  }
}
