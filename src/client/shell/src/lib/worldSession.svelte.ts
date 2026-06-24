import {
  WsClient,
  OptimisticClient,
  DocumentStore,
  ContributionRegistry,
  AssetResolver,
  ModuleRegistry,
  HookBus,
  ServiceRegistry,
  MiddlewareChain,
  reconcileTopology,
  buildSceneDoc,
  consoleLogger,
  resolveCaps,
  canWritePath,
  type Connect,
  type Logger,
  type Module,
  type WireWelcome,
  type WireOperation,
  type WireDocument,
  type WireCapabilityRequirement,
  type SceneFrame,
  type SceneSubscription,
} from "@shadowcat/core";
import type { WorldRole } from "@shadowcat/types";
import { SceneInteractionBridge, ActorSelection, TokenSelection } from "@shadowcat/ui-kit";
import { listWorldMembers } from "./api";
import { SvelteMap } from "svelte/reactivity";

export type ConnState = "connecting" | "open" | "closed";

export interface WorldSessionOpts {
  selfId: string;
  /** Browser: webSocketConnect(wsUrl). Tests: a mock connect. */
  connect: Connect;
  /** First-party default modules, in activation order (the layout/core-ui module
   *  first so its region surfaces exist before panel modules activate). */
  modules: Module[];
  /** Diagnostics sink; defaults to the leveled console logger. */
  logger?: Logger;
}

export class WorldSession {
  readonly store = new DocumentStore();
  readonly contributions = new ContributionRegistry();
  readonly assets = new AssetResolver();
  /** Canvas interaction bridge: the Stage attaches the engine; tool components reach
   * it via AppContext. Stable across Stage remount (M8d §16). */
  readonly sceneInteraction = new SceneInteractionBridge();
  /** The actor the place tool stamps; set by module-actors, read by scene-tools. Stable. */
  readonly actorSelection = new ActorSelection();
  /** Selected token ids for group-select; set by the factions panel, read by the select tool. Stable. */
  readonly tokenSelection = new TokenSelection();
  #assetListeners = new Set<(msg: { uuid: string; op: "replaced" | "deleted" }) => void>();
  #pingListeners = new Set<(msg: { scene: string; x: number; y: number; user: string }) => void>();
  #sceneSubs = new Map<
    string,
    { channel: string; onUpdate: (f: SceneFrame) => void; handle: SceneSubscription | null; gen: number }
  >();
  state = $state<ConnState>("closed");
  role = $state<WorldRole | null>(null);
  world = $state<string | null>(null);
  /** userId → username for see-as labels; fetched on a GM's Welcome (the members
   * endpoint is GM-gated, so this stays empty for players). A stable reactive Map
   * (mutated in place, never reassigned) so the reference captured into AppContext
   * at mount stays valid and consumers re-render when it populates on (re)connect. */
  readonly members = new SvelteMap<string, string>();
  /** World-default capability grants + declarative requirements from the latest Welcome; inputs
   * to the advisory `canEdit` gate. Re-set on every (re)connect. */
  #worldGrants: WireWelcome["world_default_grants"] = { by_role: {}, by_user: {} };
  #requirements: WireCapabilityRequirement[] = [];

  #ws: WsClient | null = null;
  #optimistic: OptimisticClient;
  /** Intents predicted while reconnecting (transport down but the client is still
   * `running`), queued to flush in FIFO order after the next resync completes. */
  #offlineQueue: { intentId: string; ops: WireOperation[] }[] = [];
  /** The optimistic (predicted) document view — the canvas render source, so a placed
   * or dragged document shows immediately. `store` stays the authoritative rollback base
   * (panels that want confirmed-only state read it). */
  get documents(): OptimisticClient {
    return this.#optimistic;
  }

  /** The current user's id (ownership checks). */
  get selfId(): string {
    return this.opts.selfId;
  }

  /** Advisory client-side mirror of the server's Update-path check, for showing/hiding write
   * controls. GM bypasses; the server remains authoritative and rejects a bypass at apply_intent. */
  canEdit(doc: WireDocument, path: string): boolean {
    if (this.role === "gm") return true;
    if (!this.role) return false;
    const caps = resolveCaps(doc.permissions, this.opts.selfId, this.role, this.#worldGrants);
    return canWritePath(path, caps, false, this.#requirements);
  }
  #modules: ModuleRegistry;
  #logger: Logger;
  /** One-time in-world bootstrap (module activation) guard — Welcome re-fires on
   * every reconnect, so adding/activating core-ui must not repeat. */
  #bootstrapped = false;

  constructor(private readonly opts: WorldSessionOpts) {
    this.#logger = opts.logger ?? consoleLogger();
    this.#optimistic = new OptimisticClient(opts.selfId);
    this.#modules = new ModuleRegistry({
      hooks: new HookBus(this.#logger),
      services: new ServiceRegistry(),
      middleware: new MiddlewareChain(),
      store: this.store,
      client: this.#optimistic,
      logger: this.#logger,
      contributions: this.contributions,
    });
  }

  /** Predict `ops` optimistically and transmit them as one correlated Intent. The
   * single `intent_id` ties the local prediction to the server echo/reject (FIFO
   * confirm). While reconnecting (transport down but `running`), predict AND queue:
   * every offline intent queues, so optimistic FIFO order equals the eventual send
   * order and the confirm-correlation contract holds. A flush happens after resync
   * (the optimistic view rebases onto authoritative state first). When stopped, drop
   * without an orphaned pending entry. */
  dispatchIntent(ops: WireOperation[]): void {
    const intentId = crypto.randomUUID();
    if (this.#ws?.connected) {
      this.#optimistic.applyIntent(intentId, ops);
      this.#ws.send({ type: "intent", intent_id: intentId, ops });
      return;
    }
    if (this.#ws?.running) {
      // Reconnecting: predict now (immediate feedback) and queue for FIFO replay.
      this.#optimistic.applyIntent(intentId, ops);
      this.#offlineQueue.push({ intentId, ops });
      return;
    }
    // Stopped (or no socket): no reconnect is coming, so drop without predicting —
    // an orphaned pending entry would mis-correlate the next live echo.
    this.#logger.warn("dropping intent: world session stopped");
  }

  /** Transmit intents queued while offline, in FIFO order. Called after a resync
   * completes (authoritative state is current and the optimistic view has rebased),
   * so the already-predicted intents converge as their echoes confirm them. */
  #flushOfflineQueue(): void {
    if (!this.#ws?.connected || this.#offlineQueue.length === 0) return;
    const queued = this.#offlineQueue;
    this.#offlineQueue = [];
    for (const { intentId, ops } of queued) {
      // Prediction was applied at dispatch; only transmit, preserving order.
      this.#ws.send({ type: "intent", intent_id: intentId, ops });
    }
  }

  /** Subscribe to asset replace/delete notices; returns an unsubscribe. */
  onAssetChanged(cb: (msg: { uuid: string; op: "replaced" | "deleted" }) => void): () => void {
    this.#assetListeners.add(cb);
    return () => this.#assetListeners.delete(cb);
  }

  /** Subscribe to relayed location pings (incl. our own echo); returns an unsubscribe. */
  onPing(cb: (msg: { scene: string; x: number; y: number; user: string }) => void): () => void {
    this.#pingListeners.add(cb);
    return () => this.#pingListeners.delete(cb);
  }

  /** Broadcast a transient location ping at scene coords on the active scene. No-op when
   * disconnected or no scene exists; the server relays it back to all members (incl. us). */
  sendPing(x: number, y: number): void {
    const scene = this.#optimistic.query("scene")[0];
    if (!scene) return;
    this.#ws?.send({ type: "scene_ping", scene: scene.id, x, y });
  }

  /** Subscribe to a SceneDerived channel. Returns a synchronous handle; the
   * underlying WS subscription is (re)established on every Welcome so derived state
   * survives a reconnect. */
  subscribeScene(
    channel: string,
    onUpdate: (f: SceneFrame) => void,
    opts: { asUser?: string } = {},
  ): SceneSubscription {
    const id = crypto.randomUUID();
    const rec = { channel, onUpdate, asUser: opts.asUser, handle: null as SceneSubscription | null, gen: 0 };
    this.#sceneSubs.set(id, rec);
    this.#establishScene(id, rec);
    return {
      unsubscribe: () => {
        this.#sceneSubs.delete(id);
        rec.gen++; // invalidate any in-flight establish for this record
        rec.handle?.unsubscribe();
        rec.handle = null;
      },
    };
  }

  #establishScene(
    id: string,
    rec: { channel: string; onUpdate: (f: SceneFrame) => void; asUser?: string; handle: SceneSubscription | null; gen: number },
  ): void {
    const ws = this.#ws;
    if (!ws) return;
    const gen = ++rec.gen; // this attempt's generation
    void ws
      .subscribeScene(rec.channel, rec.onUpdate, { asUser: rec.asUser })
      .then((h) => {
        // Keep the handle only if this record is still active AND this is still the
        // latest establish attempt; a superseded attempt (re-establish on a new
        // Welcome, or an unsubscribe) self-disposes so no duplicate sub leaks.
        if (this.#sceneSubs.get(id) === rec && rec.gen === gen) rec.handle = h;
        else h.unsubscribe();
      })
      .catch(() => {
        // Dropped (e.g. disconnect during connect); re-established on the next Welcome.
      });
  }

  async enter(worldId: string): Promise<void> {
    this.world = worldId;
    this.state = "connecting";
    this.#ws = new WsClient({
      connect: this.opts.connect,
      handlers: {
        // Feed both mirrors: the authoritative DocumentStore (exposed via
        // AppContext for document-reading panels) and the optimistic client
        // (base + pending view, given to modules as ctx.client).
        onCommand: (cmd) => {
          this.store.applyCommand(cmd);
          this.#optimistic.applyCommand(cmd);
        },
        onReject: (id) => this.#optimistic.reject(id),
        onWelcome: (w) => {
          void this.#onWelcome(w);
        },
        // After resync, authoritative state is current and the optimistic view has
        // rebased; replay any intents queued while offline so they converge.
        onResyncComplete: () => this.#flushOfflineQueue(),
        onError: (e) => this.#logger.error("world session ws error", e),
        onAssetChanged: (msg) => {
          // Bump the resolver first so a notified panel re-resolves the new URL.
          this.assets.onAssetChanged(msg);
          for (const cb of this.#assetListeners) cb(msg);
        },
        onScenePing: (msg) => {
          for (const cb of this.#pingListeners) cb(msg);
        },
      },
    });
    await this.#ws.start();
    this.state = "open";
  }

  async #onWelcome(w: WireWelcome): Promise<void> {
    try {
      this.role = w.user_role;
      this.#worldGrants = w.world_default_grants;
      this.#requirements = w.capability_requirements;
      // Activate modules BEFORE any await below (a GM's member fetch) so the
      // layout module contributes Layout into the `root` surface the host renders
      // — the table chrome paints immediately on mount, never a blank frame during
      // the GM member-fetch round-trip. `#bootstrapped` set before the await so a
      // second Welcome (reconnect) cannot re-enter and double-add the modules.
      if (!this.#bootstrapped) {
        this.#bootstrapped = true;
        for (const m of this.opts.modules) this.#modules.add(m);
        await this.#modules.activate();
      }
      // Fetch member usernames for see-as labels (GM only; the endpoint 403s
      // players). Best-effort: a failure leaves the picker on short-id fallback.
      // The members SvelteMap is mutated in place, so the see-as UI (already
      // rendered after activation) populates reactively when this resolves.
      if (w.user_role === "gm") {
        try {
          const list = await listWorldMembers(w.world);
          // Mutate in place (not reassign) so the AppContext-captured reference
          // stays valid; reconnect re-populates the same Map.
          this.members.clear();
          for (const m of list) this.members.set(m.user, m.username);
        } catch (e) {
          this.#logger.warn("member list fetch failed", e);
        }
      }
      reconcileTopology(this.#modules.declarations(), w.contract_declarations, this.#logger);
      // Scene subscriptions are dropped by the WS on disconnect; re-establish each
      // on every (re)connect so derived state (vision) survives a reconnect. No-op
      // on the first Welcome (none registered until the render engine subscribes).
      for (const [id, rec] of this.#sceneSubs) {
        // Tear down a live handle from a prior connect before re-subscribing; the
        // gen bump inside #establishScene invalidates any still-in-flight attempt,
        // so a flapping reconnect can't leak a duplicate server subscription.
        rec.handle?.unsubscribe();
        rec.handle = null;
        this.#establishScene(id, rec);
      }
      // M8d §15: ensure an active scene exists so the place tool has a parent to
      // attach tokens to. GM-only (players can't author the world's first scene);
      // guard on the optimistic view (includes the pending create) so a reconnect
      // Welcome — or a scene from another GM — does not double-create. The rare
      // multi-GM simultaneous-first-entry double-create is accepted (M12 dedupes).
      if (this.role === "gm" && this.world && this.#optimistic.query("scene").length === 0) {
        this.dispatchIntent([{ op: "create", doc: buildSceneDoc(this.world) }]);
      }
    } catch (e) {
      this.#logger.error("world session welcome handling failed", e);
    }
  }

  leave(): void {
    this.#ws?.stop();
    this.#ws = null;
    this.state = "closed";
    this.role = null;
    this.world = null;
  }
}
