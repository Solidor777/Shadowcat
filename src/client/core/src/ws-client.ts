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
  type WireActorOwnerRef,
  type WireAudience,
} from "./wire";

/** A resolved page of search results (Core.search). */
export interface SearchPage {
  hits: WireSearchHit[];
  nextCursor?: string;
}

/** A resolved pathfind result (WsClient.pathfind). `arrested` is true when the route was cut
 * short by a visible arrest region (spec §5). */
export interface PathResult {
  path: [number, number][];
  cost: number;
  arrested: boolean;
}

/** A single position sample in a MoveStream, with elapsed-ms origin at startServerMs. */
export interface MoveSample {
  tMs: number;
  pos: [number, number];
}

/** A vision-polygon sample paired with a MoveSample by tMs (mover-only; null for observers). */
export interface MoveVisionSample {
  tMs: number;
  polygons: [number, number][][];
}

/** Broadcast animation frame delivered to every scene viewer (mover + observers).
 * Wire snake_case fields are mapped to camelCase. Mover receives the full trajectory +
 * moverVision; observers receive server-clipped position samples, moverVision=null. */
export interface MoveStream {
  requestId: string;
  tokenId: string;
  mover: string;
  scene: string;
  startServerMs: number;
  durationMs: number;
  stop: [number, number];
  samples: MoveSample[];
  moverVision: MoveVisionSample[] | null;
  /** Total terrain-weighted movement cost for this move (M10g). Informational.
   * Null for a clipped observer (mirrors moverVision) — the authoritative cost may reflect
   * secret-region terrain the observer's clipped samples don't reveal. */
  cost: number | null;
}

/** The union of results a correlated request in `pending` can resolve to. */
export type PendingResult = SearchPage | PathResult | MoveStream;

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
  /** Terminal eviction (world/account deleted). The client has already
   * stopped (no reconnect) when this fires; route the user out of the world. */
  onEvicted?: () => void;
}

export interface WsClientOptions {
  connect: Connect;
  handlers: WsClientHandlers;
  now?: () => number;
  sleep?: (ms: number) => Promise<void>;
  backoffBaseMs?: number;
  backoffMaxMs?: number;
  /** Ms to wait for the server's Welcome after a transport opens before the
   * connection is treated as dead (closed → normal reconnect/backoff). The
   * browser's socket `open` fires at HTTP 101, BEFORE the server's Welcome
   * preamble, so "open but never welcomed" is otherwise an unbounded silent
   * wait no reconnect machinery can see. Same 10s convention as the
   * correlated-request timeouts below. */
  welcomeTimeoutMs?: number;
}

const defaultSleep = (ms: number): Promise<void> =>
  new Promise((r) => setTimeout(r, ms));

/** How long a correlated chat op waits for a `chat_error` before assuming
 * success. Comfortably beyond the server's synchronous link-preview deadline
 * (5s) so a slow-but-successful send never resolves ahead of a would-be error. */
const CHAT_ERROR_WINDOW_MS = 15_000;

/**
 * Client-side WebSocket connection to one world: owns the transport lifecycle (connect,
 * Welcome watchdog, exponential-backoff reconnect), the ordered application watermark
 * (`nextExpected`) with gap-triggered resync, correlated one-shot requests (search/pathfind/
 * moveRequest/scene-subscribe), the asymmetric chat-op reply protocol, and broadcast
 * `MoveStream`/`onScenePing`/`onAssetChanged` fan-out. One instance per world connection;
 * `start()`/`stop()` bracket its lifetime.
 * @example
 * ```ts
 * import { WsClient, webSocketConnect } from "@shadowcat/core";
 *
 * const client = new WsClient({
 *   connect: webSocketConnect("wss://example.test/ws"),
 *   handlers: { onCommand: (cmd) => console.log(cmd.seq) },
 * });
 * await client.start();
 * ```
 */
export class WsClient {
  private transport: Transport | null = null;
  private running_ = false;
  private reconnectAttempt = 0;
  /** Next seq to apply; the client-side ordering watermark. Persists across
   * reconnects so resync resumes from where application left off. */
  private nextExpected = 1;
  private serverOffsetMs = 0;
  /** In-flight correlated requests (search, pathfind, moveRequest), keyed by request_id. */
  private pending = new Map<
    string,
    {
      resolve: (result: PendingResult) => void;
      reject: (e: Error) => void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();
  /** Persistent listeners for broadcast MoveStream frames (mover + all observers). */
  private moveStreamListeners = new Set<(s: MoveStream) => void>();
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
  /** In-flight chat ops (send/edit/delete), keyed by request_id. Chat is
   * asymmetric: ONLY a rejection replies (a `chat_error` frame); a successful
   * op is confirmed by the broadcast Event echo, with no direct reply. So the
   * timer RESOLVES (success-assumed) and cleans the entry up when no error
   * arrives within the window. */
  private chatPending = new Map<
    string,
    { resolve: () => void; reject: (e: Error) => void; timer: ReturnType<typeof setTimeout> }
  >();

  private readonly now: () => number;
  private readonly sleep: (ms: number) => Promise<void>;
  private readonly backoffBaseMs: number;
  private readonly backoffMaxMs: number;
  private readonly welcomeTimeoutMs: number;
  private welcomeTimer: ReturnType<typeof setTimeout> | null = null;
  /** Bumped on every `open()` attempt; each connection's `onMessage` closure
   * captures its own value. A frame delivered after the client has already
   * moved on to a later connection (e.g. a Welcome already queued as a
   * message task when the watchdog closed this connection) carries a stale
   * generation and must not act on the CURRENT connection's state — notably
   * it must not disarm the current watchdog, which would silently reopen the
   * hang this task exists to close. */
  private connGeneration = 0;
  /** The `connGeneration` that has been welcomed, or `-1` if none yet.
   * `connGeneration` starts at 0 and is pre-incremented before assignment, so
   * `-1` never collides with a real generation. Guards `armWelcomeWatchdog`
   * against the out-of-order case where a `Connect` delivers Welcome (via a
   * microtask) BEFORE its own `await` continuation runs: without this check
   * the watchdog would arm anyway once the continuation resumes, and nothing
   * would ever disarm it (Welcome already came and went) — the healthy
   * connection would be closed at the watchdog window, forever. */
  private welcomedGeneration = -1;

  /**
   * Construct a client bound to one connection factory + handler set; call `start()` to open it.
   * @param opts Connection factory + handlers + timing knobs (`backoffBaseMs`/`backoffMaxMs`/
   * `welcomeTimeoutMs` all default per `WsClientOptions`'s own field docs).
   * @example
   * ```ts
   * import { WsClient, webSocketConnect } from "@shadowcat/core";
   *
   * const client = new WsClient({
   *   connect: webSocketConnect("wss://example.test/ws"),
   *   handlers: { onCommand: () => {} },
   * });
   * ```
   */
  constructor(private readonly opts: WsClientOptions) {
    this.now = opts.now ?? Date.now;
    this.sleep = opts.sleep ?? defaultSleep;
    this.backoffBaseMs = opts.backoffBaseMs ?? 250;
    this.backoffMaxMs = opts.backoffMaxMs ?? 10_000;
    this.welcomeTimeoutMs = opts.welcomeTimeoutMs ?? 10_000;
  }

  /** Arm the Welcome watchdog for the currently-open transport. Identity guard:
   * close only the transport this timer was armed for — a stale timer
   * surviving into a successor connection must no-op.
   * @example
   * ```
   * // called from open() once opts.connect resolves; not part of the public API
   * this.armWelcomeWatchdog();
   * ```
   */
  private armWelcomeWatchdog(): void {
    this.clearWelcomeWatchdog();
    if (this.welcomedGeneration === this.connGeneration) {
      // Welcome for THIS connection already arrived (out-of-order Connect
      // delivery) before this arm call ran — arming now would create a
      // watchdog nothing will ever disarm.
      return;
    }
    const armed = this.transport;
    this.welcomeTimer = setTimeout(() => {
      this.welcomeTimer = null;
      if (this.running_ && armed !== null && this.transport === armed) {
        // Treat as a dead link: close() fires the transport's onClose →
        // handleClose → failPending + scheduleReconnect. Self-healing.
        armed.close();
      }
    }, this.welcomeTimeoutMs);
  }

  /** Disarm the Welcome watchdog, if one is currently armed. Called on Welcome
   * receipt, on `stop()`, and before re-arming (`armWelcomeWatchdog`'s own
   * leading call) so at most one watchdog timer is ever live.
   * @example
   * ```
   * // called from handleFrame's "welcome" case and from stop(); not public API
   * this.clearWelcomeWatchdog();
   * ```
   */
  private clearWelcomeWatchdog(): void {
    if (this.welcomeTimer !== null) {
      clearTimeout(this.welcomeTimer);
      this.welcomeTimer = null;
    }
  }

  /** Open the connection (and keep it open across drops until `stop`).
   * @example
   * ```ts
   * import { WsClient, webSocketConnect } from "@shadowcat/core";
   *
   * const client = new WsClient({
   *   connect: webSocketConnect("wss://example.test/ws"),
   *   handlers: { onCommand: () => {} },
   * });
   * await client.start();
   * ```
   */
  async start(): Promise<void> {
    this.running_ = true;
    await this.open();
  }

  /** Stop reconnecting and close the live transport (if any); fails every
   * in-flight correlated request/subscription with "client stopped". Terminal
   * for this instance's connection attempts — a subsequent `start()` begins a
   * fresh cycle (`connGeneration` keeps counting up, so a stray frame from the
   * stopped connection cannot be mistaken for the new one).
   * @example
   * ```ts
   * import { WsClient, webSocketConnect } from "@shadowcat/core";
   *
   * const client = new WsClient({
   *   connect: webSocketConnect("wss://example.test/ws"),
   *   handlers: { onCommand: () => {} },
   * });
   * client.stop();
   * ```
   */
  stop(): void {
    this.running_ = false;
    this.clearWelcomeWatchdog();
    this.transport?.close();
    this.transport = null;
    this.failPending("client stopped");
  }

  /** Reject every in-flight correlated request (e.g. on disconnect/stop): the
   * request was sent on a socket that will not answer it, so fail fast rather
   * than wait out the timeout (which would also outlive the connection).
   * @param reason Message for the `Error` every pending promise rejects with.
   * @example
   * ```
   * // called from handleClose() / stop(); not part of the public API
   * this.failPending("connection closed");
   * ```
   */
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
    // Chat ops were sent on a socket that will not answer; whether the op landed
    // is unknown, so reject rather than silently resolve.
    for (const p of this.chatPending.values()) {
      clearTimeout(p.timer);
      p.reject(new Error(reason));
    }
    this.chatPending.clear();
  }

  /** Run a consumer callback in isolation: a throw is routed to `onError` and
   * never propagates into the socket message pump. A throw from `onError`
   * itself is swallowed so the pump cannot die.
   * @param fn The handler-invoking callback to run in isolation.
   * @example
   * ```
   * // wraps every handler dispatch in handleFrame(); not part of the public API
   * this.safeEmit(() => this.opts.handlers.onWelcome?.(msg));
   * ```
   */
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

  /** Send a client frame (no-op if currently disconnected). `ClientMsg` is a plain TS union
   * (outgoing frames are not runtime-validated, unlike `ServerMsgSchema` on the inbound side —
   * `SendMessageSchema`/`PathfindSchema` in `wire.ts` are standalone opt-in mirrors for callers
   * that want to validate before sending, not enforced here).
   * @param msg The frame to serialize (`JSON.stringify`) and send.
   * @example
   * ```ts
   * import { WsClient, webSocketConnect } from "@shadowcat/core";
   *
   * const client = new WsClient({
   *   connect: webSocketConnect("wss://example.test/ws"),
   *   handlers: { onCommand: () => {} },
   * });
   * client.send({ type: "time_ping", client_t0: Date.now() });
   * ```
   */
  send(msg: ClientMsg): void {
    this.transport?.send(JSON.stringify(msg));
  }

  /** The highest authoritative seq applied.
   * @returns `nextExpected - 1`; the watermark `OptimisticClient`/`DocumentStore` key their
   * rebase against (see `render-from-optimistic-view`). */
  get appliedSeq(): number {
    return this.nextExpected - 1;
  }

  /** True when a live transport is attached, so `send` will actually transmit.
   * @returns `true` between a successful `open()` and the next `handleClose`/`stop()`. */
  get connected(): boolean {
    return this.transport !== null;
  }

  /** True between `start` and `stop`: a dropped transport will reconnect. Lets a
   * caller distinguish "reconnecting" (queue + retry) from "stopped" (give up).
   * @returns `true` from `start()` until the matching `stop()`, regardless of the transport's
   * own connected state. */
  get running(): boolean {
    return this.running_;
  }

  /** Estimated server clock.
   * @returns `this.now() + serverOffsetMs`, where `serverOffsetMs` is set from the `welcome`
   * frame's `server_time` (and refreshed on every `time_pong`).
   * @example
   * ```ts
   * import { WsClient, webSocketConnect } from "@shadowcat/core";
   *
   * const client = new WsClient({
   *   connect: webSocketConnect("wss://example.test/ws"),
   *   handlers: { onCommand: () => {} },
   * });
   * const t: number = client.serverNow();
   * ```
   */
  serverNow(): number {
    return this.now() + this.serverOffsetMs;
  }

  /** One connection attempt: bumps `connGeneration`, awaits `opts.connect`, and on success arms
   * the Welcome watchdog; on failure (or if `stop()` ran while the connect was pending — see
   * the `running_`-not-rechecked-post-await gotcha, `docs/TODO.md`) schedules a reconnect
   * instead. Called by `start()` and by `scheduleReconnect`'s resolved sleep. Not exported.
   * @returns Resolves once this attempt has either armed the watchdog or scheduled a retry.
   * @example
   * ```
   * // called from start() and scheduleReconnect(); not part of the public API
   * await this.open();
   * ```
   */
  private async open(): Promise<void> {
    if (!this.running_) return;
    const gen = ++this.connGeneration;
    try {
      this.transport = await this.opts.connect({
        onMessage: (d) => this.handleFrame(d, gen),
        onClose: () => this.handleClose(),
      });
      // reconnectAttempt resets on WELCOME (handleFrame's "welcome" case), not
      // here: a server that accepts the socket but never sends Welcome must
      // keep backing off on each watchdog-close/reconnect cycle, not retry at
      // the base delay forever (that would amplify load against exactly the
      // degraded server the watchdog exists to escape).
      this.armWelcomeWatchdog();
    } catch {
      this.scheduleReconnect();
    }
  }

  /** `Transport.onClose` handler: disarms the watchdog, clears `this.transport`, fails every
   * in-flight correlated request with "connection closed", and — if still `running_` (not
   * a `stop()`-triggered close) — schedules a reconnect. Not exported.
   * @example
   * ```
   * // wired as onClose: this.opts.connect({ onMessage, onClose: () => this.handleClose() })
   * this.handleClose();
   * ```
   */
  private handleClose(): void {
    this.clearWelcomeWatchdog();
    this.transport = null;
    // In-flight requests were sent on the now-dead socket; a reconnect will not
    // replay them, so reject rather than leave them hanging until timeout.
    this.failPending("connection closed");
    if (this.running_) this.scheduleReconnect();
  }

  /** Schedule the next `open()` attempt after an exponential-with-full-jitter delay:
   * `min(backoffMaxMs, backoffBaseMs * 2**attempt) * (0.5 + random()*0.5)`. `reconnectAttempt`
   * increments on every call and resets ONLY on a successful Welcome (`handleFrame`'s
   * `"welcome"` case), not on socket `open` — so a server that accepts the socket but never
   * welcomes keeps backing off across watchdog-close/reconnect cycles instead of retrying at
   * the base delay forever. Not exported.
   * @example
   * ```
   * // called from open()'s catch and from handleClose(); not part of the public API
   * this.scheduleReconnect();
   * ```
   */
  private scheduleReconnect(): void {
    const attempt = this.reconnectAttempt++;
    const ceiling = Math.min(
      this.backoffMaxMs,
      this.backoffBaseMs * 2 ** attempt,
    );
    const delay = ceiling * (0.5 + Math.random() * 0.5); // full jitter (half..full)
    void this.sleep(delay).then(() => this.open());
  }

  /** Parse (`parseServerMsg`, Zod-validated against `ServerMsgSchema` — malformed/unknown JSON
   * is silently dropped) and dispatch one inbound text frame by `msg.type`. `gen` is the
   * `connGeneration` this connection's `onMessage` closure was created with (stamped in
   * `open()` before the connect `await`); ONLY the `"welcome"` and `"resync_end"` cases compare
   * `gen !== this.connGeneration` and bail — every other frame type (`event`, `reject`,
   * `move_stream`, chat/search/pathfind replies, etc.) acts regardless of generation, because
   * those either carry their own `request_id` correlation (dropped harmlessly if the requester
   * already gave up) or are idempotent/order-tolerant on the applied-seq watermark.
   * @param text The raw frame text off the socket.
   * @param gen This connection's `connGeneration`, captured when its `onMessage` closure was
   * created.
   * @example
   * ```
   * // wired as onMessage in open(): this.opts.connect({ onMessage: (d) => this.handleFrame(d, gen), ... })
   * this.handleFrame(text, gen);
   * ```
   */
  private handleFrame(text: string, gen: number): void {
    const msg = parseServerMsg(text);
    if (!msg) return;
    switch (msg.type) {
      case "welcome":
        // Generation guard: a Welcome queued on a since-superseded connection
        // (e.g. already in flight when the watchdog closed it) must not act
        // on the CURRENT connection's state — in particular it must not
        // disarm the current connection's own watchdog.
        if (gen !== this.connGeneration) break;
        this.clearWelcomeWatchdog();
        this.welcomedGeneration = gen;
        // "Connection established" means welcomed, not merely socket-opened —
        // reset here so an accepted-but-degraded server (open, never welcomes)
        // keeps backing off instead of retrying at the base delay forever.
        this.reconnectAttempt = 0;
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
        // Generation guard (mirrors "welcome"): a superseded connection's
        // queued resync_end must not fire onResyncComplete on the successor.
        if (gen !== this.connGeneration) break;
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
      case "evicted":
        // Terminal: the server is deleting this world or account. Stop first
        // (running=false → the onClose path will not schedule a reconnect),
        // then let the shell route away.
        this.stop();
        this.safeEmit(() => this.opts.handlers.onEvicted?.());
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
          (p.resolve as (r: PathResult) => void)({ path: msg.path, cost: msg.cost, arrested: msg.arrested });
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
      case "move_stream": {
        // Map wire snake_case to camelCase MoveStream for all consumers.
        const stream: MoveStream = {
          requestId: msg.request_id,
          tokenId: msg.token_id,
          mover: msg.mover,
          scene: msg.scene,
          startServerMs: msg.start_server_ms,
          durationMs: msg.duration_ms,
          stop: msg.stop,
          samples: msg.samples.map((s) => ({ tMs: s.t_ms, pos: s.pos })),
          moverVision: msg.mover_vision
            ? msg.mover_vision.map((v) => ({ tMs: v.t_ms, polygons: v.polygons as [number, number][][] }))
            : null,
          cost: msg.cost,
        };
        // Resolve the mover's pending promise (if request_id matches).
        const p = this.pending.get(msg.request_id);
        if (p) {
          clearTimeout(p.timer);
          this.pending.delete(msg.request_id);
          (p.resolve as (r: MoveStream) => void)(stream);
        }
        // Broadcast to all registered listeners (mover + observers).
        for (const cb of this.moveStreamListeners) {
          this.safeEmit(() => cb(stream));
        }
        break;
      }
      case "move_error": {
        const p = this.pending.get(msg.request_id);
        if (p) {
          clearTimeout(p.timer);
          this.pending.delete(msg.request_id);
          p.reject(new Error(msg.message));
        }
        break;
      }
      case "chat_error": {
        // A rejected send/edit/delete: reject the correlated op so the composer
        // surfaces the reason (already classified server-side — no leak).
        const p = this.chatPending.get(msg.request_id);
        if (p) {
          clearTimeout(p.timer);
          this.chatPending.delete(msg.request_id);
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
   * @param query The full-text query string.
   * @param opts Search options.
   * @param opts.limit Max hits per page, sent on the wire as `limit: opts.limit ?? 20`
   * (client-side default only — the server's `Search` frame field is mandatory, no server
   * default; `ws/protocol.rs`'s `ClientMsg::Search.limit: u32`).
   * @param opts.cursor Opaque pagination cursor from a prior `SearchPage.nextCursor`.
   * @param opts.timeoutMs How long to wait for `search_result`/`search_error` before rejecting
   * (default 10000).
   * @returns The resolved page of hits + an optional `nextCursor`.
   * @example
   * ```ts
   * import { WsClient, webSocketConnect } from "@shadowcat/core";
   *
   * const client = new WsClient({
   *   connect: webSocketConnect("wss://example.test/ws"),
   *   handlers: { onCommand: () => {} },
   * });
   * const page = await client.search("goblin", { limit: 10 });
   * ```
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
      this.pending.set(request_id, { resolve: resolve as (r: PendingResult) => void, reject, timer });
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
   * @param query The full-text query string.
   * @param opts Live-search options.
   * @param opts.limit Max hits tracked, sent on the wire as `limit: opts.limit ?? 20`
   * (client-side default only — the server's `Search` frame field is mandatory, no server
   * default; `ws/protocol.rs`'s `ClientMsg::Search.limit: u32`).
   * @param opts.timeoutMs How long to wait for the initial result before rejecting
   * (default 10000).
   * @param onUpdate Fires with the current top-N hits on the initial result and every
   * subsequent `search_update` push.
   * @returns A handle whose `unsubscribe()` stops updates and sends `{ type: "unsubscribe" }`.
   * @example
   * ```ts
   * import { WsClient, webSocketConnect } from "@shadowcat/core";
   *
   * const client = new WsClient({
   *   connect: webSocketConnect("wss://example.test/ws"),
   *   handlers: { onCommand: () => {} },
   * });
   * const handle = await client.subscribeSearch("goblin", {}, (hits) => console.log(hits.length));
   * handle.unsubscribe();
   * ```
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
   * @param channel The SceneDerived channel name to subscribe to.
   * @param onUpdate Fires with `{ payload, computedAtSeq }` for every frame delivered on this
   * subscription.
   * @param opts Subscription options.
   * @param opts.timeoutMs How long to wait for the first `scene_derived`/`scene_error` before
   * rejecting (default 10000).
   * @param opts.asUser GM-only see-as-player override; sent as `as_user` only when set (the
   * server gates and resolves it — an unauthorized value is the server's rejection to make).
   * @returns A handle whose `unsubscribe()` drops the channel and sends `scene_unsubscribe`.
   * @example
   * ```ts
   * import { WsClient, webSocketConnect } from "@shadowcat/core";
   *
   * const client = new WsClient({
   *   connect: webSocketConnect("wss://example.test/ws"),
   *   handlers: { onCommand: () => {} },
   * });
   * const sub = await client.subscribeScene("vision", (frame) => console.log(frame.computedAtSeq));
   * sub.unsubscribe();
   * ```
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
   *
   * `token`, when given, names the token the route is for: the server derives the footprint from
   * it and IGNORES `footprintRadius` entirely — the client value is honored only when `token` is
   * omitted (an explicitly hypothetical preview with no preview-equals-execution guarantee).
   * @param scene The scene id to route through.
   * @param start Starting `[x, y]` grid/world coordinate.
   * @param waypoints Ordered `[x, y]` waypoints the route must pass through.
   * @param footprintRadius Hypothetical mover footprint radius; ignored server-side when
   * `token` is given.
   * @param token Optional token id the route is for; when present the server derives the
   * footprint from the token and this method's `footprintRadius` argument is not honored.
   * @param opts Request options.
   * @param opts.timeoutMs How long to wait for `path_result`/`path_error` before rejecting
   * (default 10000).
   * @returns The computed path, cost, and whether it was cut short by a visible arrest region.
   * @example
   * ```ts
   * import { WsClient, webSocketConnect } from "@shadowcat/core";
   *
   * const client = new WsClient({
   *   connect: webSocketConnect("wss://example.test/ws"),
   *   handlers: { onCommand: () => {} },
   * });
   * const result = await client.pathfind("scene-1", [0, 0], [[5, 5]], 0.5);
   * ```
   */
  pathfind(
    scene: string,
    start: [number, number],
    waypoints: [number, number][],
    footprintRadius: number,
    token?: string,
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
      this.pending.set(request_id, { resolve: resolve as (r: PendingResult) => void, reject, timer });
      this.send({
        type: "pathfind",
        request_id,
        scene,
        start,
        waypoints,
        footprint_radius: footprintRadius,
        ...(token ? { token } : {}),
      });
    });
  }

  /**
   * Issue a correlated move-execution request; resolves with the broadcast `MoveStream` when
   * the matching `move_stream` frame arrives (mover's request_id correlates). Rejects on a
   * `move_error` frame or after `timeoutMs`. Pure transport mirror — no client-side movement
   * logic. All scene viewers (mover + observers) also receive the frame via `onMoveStream`.
   * @param scene The scene id the token is on.
   * @param tokenId The token to move.
   * @param path Ordered `[x, y]` waypoints for the requested move.
   * @param opts Request options.
   * @param opts.timeoutMs How long to wait for the mover's own `move_stream`/`move_error`
   * before rejecting (default 10000).
   * @returns The mover's `MoveStream` (full trajectory + `moverVision`); observers instead
   * receive their own clipped copy via `onMoveStream`, never through this promise.
   * @example
   * ```ts
   * import { WsClient, webSocketConnect } from "@shadowcat/core";
   *
   * const client = new WsClient({
   *   connect: webSocketConnect("wss://example.test/ws"),
   *   handlers: { onCommand: () => {} },
   * });
   * const stream = await client.moveRequest("scene-1", "token-1", [[1, 1]]);
   * ```
   */
  moveRequest(
    scene: string,
    tokenId: string,
    path: [number, number][],
    opts: { timeoutMs?: number } = {},
  ): Promise<MoveStream> {
    const request_id = crypto.randomUUID();
    const timeoutMs = opts.timeoutMs ?? 10_000;
    return new Promise<MoveStream>((resolve, reject) => {
      if (!this.transport) {
        reject(new Error("not connected"));
        return;
      }
      const timer = setTimeout(() => {
        this.pending.delete(request_id);
        reject(new Error("move_request timeout"));
      }, timeoutMs);
      this.pending.set(request_id, { resolve: resolve as (r: PendingResult) => void, reject, timer });
      this.send({ type: "move_request", request_id, scene, token_id: tokenId, path });
    });
  }

  /**
   * Subscribe to broadcast MoveStream frames. Called for every recipient (mover + observers)
   * whenever a token's server-authoritative move completes. Returns an unsubscribe function.
   * Listeners survive reconnects; a caller that subscribes once keeps receiving across drops.
   * @param cb Fires with every `MoveStream` frame delivered to this connection, from ANY scene
   * in the world — `WsClient` is a per-world connection with no notion of a "current scene",
   * and the server's per-recipient egress clip (`clip_move_stream`, `ws/conn.rs`) filters only
   * by vision/GM-trust, never by scene (a GM with no active see-as gets the full unclipped
   * stream regardless of scene). Filtering to a viewed scene via `MoveStream.scene` is the
   * CALLER's responsibility (see `worldSession`'s `onMoveStream` handler).
   * @returns An unsubscribe function that removes `cb` from the listener set.
   * @example
   * ```ts
   * import { WsClient, webSocketConnect } from "@shadowcat/core";
   *
   * const client = new WsClient({
   *   connect: webSocketConnect("wss://example.test/ws"),
   *   handlers: { onCommand: () => {} },
   * });
   * const off = client.onMoveStream((s) => console.log(s.tokenId));
   * off();
   * ```
   */
  onMoveStream(cb: (s: MoveStream) => void): () => void {
    this.moveStreamListeners.add(cb);
    return () => this.moveStreamListeners.delete(cb);
  }

  /** Register a correlated chat op: resolves (success-assumed) when no
   * `chat_error` arrives within the window, rejects with the server's reason
   * when one does. The timer is unref'd where supported so it never keeps a
   * test's event loop alive.
   * @param request_id The op's correlation id (already sent on the wire by the caller).
   * @returns Resolves (void) after `CHAT_ERROR_WINDOW_MS` with no error; rejects immediately
   * if a matching `chat_error` arrives first.
   * @example
   * ```
   * // called from sendChatMessage/editChatMessage/deleteChatMessage; not part of the public API
   * const p = this.trackChatOp(request_id);
   * ```
   */
  private trackChatOp(request_id: string): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.chatPending.delete(request_id);
        resolve();
      }, CHAT_ERROR_WINDOW_MS);
      (timer as unknown as { unref?: () => void }).unref?.();
      this.chatPending.set(request_id, { resolve, reject, timer });
    });
  }

  /**
   * Send a chat message. Resolves when the send is accepted (confirmed by the
   * broadcast Event echo; assumed after `CHAT_ERROR_WINDOW_MS` with no error),
   * rejects with the server's player-presentable reason on a correlated
   * `chat_error`. The composer surfaces the rejection instead of it vanishing.
   * @param opts Send options.
   * @param opts.channel The chat channel to post into.
   * @param opts.content The message body (server-sanitized; see `shadowcat-codebase-chat`).
   * @param opts.actorOwner Optional actor attribution for the message; sent as `null` when
   * omitted.
   * @param opts.audience Optional recipient scoping; defaults to `{ kind: "public" }`.
   * @returns Resolves (void) once the send is accepted; rejects with the server's
   * player-presentable reason otherwise.
   * @example
   * ```ts
   * import { WsClient, webSocketConnect } from "@shadowcat/core";
   *
   * const client = new WsClient({
   *   connect: webSocketConnect("wss://example.test/ws"),
   *   handlers: { onCommand: () => {} },
   * });
   * await client.sendChatMessage({ channel: "main", content: "hello" });
   * ```
   */
  sendChatMessage(opts: {
    channel: string;
    content: string;
    actorOwner?: WireActorOwnerRef | null;
    audience?: WireAudience;
  }): Promise<void> {
    const request_id = crypto.randomUUID();
    const p = this.trackChatOp(request_id);
    this.send({
      type: "send_message",
      request_id,
      channel: opts.channel,
      content: opts.content,
      actor_owner: opts.actorOwner ?? null,
      audience: opts.audience ?? { kind: "public" },
    });
    return p;
  }

  /** Edit an existing chat message. Resolves/rejects like `sendChatMessage`; the
   * server enforces edit ownership and rejects via a correlated `chat_error`.
   * @param messageId The message document id to edit.
   * @param content The replacement content.
   * @returns Resolves (void) once the edit is accepted; rejects with the server's reason
   * otherwise.
   * @example
   * ```ts
   * import { WsClient, webSocketConnect } from "@shadowcat/core";
   *
   * const client = new WsClient({
   *   connect: webSocketConnect("wss://example.test/ws"),
   *   handlers: { onCommand: () => {} },
   * });
   * await client.editChatMessage("msg-1", "corrected text");
   * ```
   */
  editChatMessage(messageId: string, content: string): Promise<void> {
    const request_id = crypto.randomUUID();
    const p = this.trackChatOp(request_id);
    this.send({ type: "edit_message", request_id, message_id: messageId, content });
    return p;
  }

  /** Delete an existing chat message. Resolves/rejects like `sendChatMessage`; the
   * server enforces delete ownership and rejects via a correlated `chat_error`.
   * @param messageId The message document id to delete.
   * @returns Resolves (void) once the delete is accepted; rejects with the server's reason
   * otherwise.
   * @example
   * ```ts
   * import { WsClient, webSocketConnect } from "@shadowcat/core";
   *
   * const client = new WsClient({
   *   connect: webSocketConnect("wss://example.test/ws"),
   *   handlers: { onCommand: () => {} },
   * });
   * await client.deleteChatMessage("msg-1");
   * ```
   */
  deleteChatMessage(messageId: string): Promise<void> {
    const request_id = crypto.randomUUID();
    const p = this.trackChatOp(request_id);
    this.send({ type: "delete_message", request_id, message_id: messageId });
    return p;
  }

  /** Apply one authoritative `event` frame's `WireCommand` against the ordering watermark:
   * a duplicate (`seq < nextExpected`) is dropped, a gap (`seq > nextExpected`) triggers a
   * `resync_request` from the current watermark instead of applying out of order, and an
   * in-order command is dispatched to `handlers.onCommand` (a thrown apply error is surfaced
   * via `onError`, never left to kill the socket loop) before `nextExpected` advances past it.
   * @param cmd The command to apply, from `ServerMsgSchema`'s `"event"` variant's `command`
   * field (`CommandSchema`: `seq`/`world_id`/`author`/`ts`/`ops`).
   * @example
   * ```
   * // called from handleFrame's "event" case; not part of the public API
   * this.applyEvent(msg.command);
   * ```
   */
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
