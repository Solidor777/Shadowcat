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
  resolveViewedScene,
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
  type PathResult,
  type MoveStream,
  type WireActorOwnerRef,
  type WireAudience,
  type SubscriptionHandle,
  type WireSearchHit,
  loadModules,
  type ModuleEntry,
  type ModuleManifest,
  listInstalledModules,
  getEnabledModules,
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
  /** Client-local GM override of the rendered/subscribed scene (M12d "GM roams"). Never set for
   * a player (they follow `world-settings.activeScene`). Overrides `viewedSceneId` for THIS
   * client's own render + vision + see-as channels only; the server is unaware of it. */
  #gmViewedScene = $state<string | null>(null);
  /** userId → username for the world's members, fetched on every role's Welcome
   * (chat author/whisper-recipient name resolution; the GM additionally uses it
   * for see-as labels). A stable reactive Map (mutated in place, never reassigned)
   * so the reference captured into AppContext at mount stays valid and consumers
   * re-render when it populates on (re)connect. */
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

  /** The scene THIS client renders + subscribes to (M12d). A GM's local roam
   * (`#gmViewedScene`) overrides; otherwise follows `world-settings.activeScene`, else the first
   * scene. Reads the optimistic view + `#gmViewedScene` $state, so Svelte deriveds that read it
   * (bridged through `documents.subscribe`) react to both scene-doc changes and roam changes. */
  get viewedSceneId(): string | null {
    return resolveViewedScene(this.#optimistic, { gmViewedScene: this.role === "gm" ? this.#gmViewedScene : null });
  }

  /** GM local roam (M12d): view any scene without moving players. Ignored (warned) for a non-GM —
   * players have no local override. `null` clears the roam (follow `activeScene`). */
  setGmViewedScene(id: string | null): void {
    if (this.role !== "gm") {
      this.#logger.warn("setGmViewedScene ignored: caller is not a GM");
      return;
    }
    this.#gmViewedScene = id;
  }

  /** Live full-text search over documents (M6c subscription seam). Ephemeral: NOT re-established
   * across reconnects (unlike `subscribeScene`) — the caller re-subscribes on the next query.
   * Rejects immediately when there is no live transport. */
  searchDocuments(
    query: string,
    opts: { limit?: number; timeoutMs?: number },
    onUpdate: (hits: WireSearchHit[]) => void,
  ): Promise<SubscriptionHandle> {
    if (!this.#ws) return Promise.reject(new Error("not connected"));
    return this.#ws.subscribeSearch(query, opts, onUpdate);
  }

  /** Advisory client-side mirror of the server's Update-path check, for showing/hiding write
   * controls. GM bypasses; the server remains authoritative and rejects a bypass at apply_intent.
   * Caveat: `#requirements` (from the Welcome union) mixes GM-authored world_cap_requirements
   * with module-declared manifest requirements. For GM-authored entries this mirror matches
   * server enforcement exactly. Module-published entries are advisory UX only — the server
   * does NOT reject a write solely because a module declared a requirement on that path, so
   * this gate can be stricter here than the server actually is for module-only requirements. */
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

  /** Broadcast a transient location ping at scene coords on the currently-viewed scene
   * (`viewedSceneId`: a GM's local roam override, else the followed `activeScene`). No-op when
   * disconnected or no scene exists; the server relays it back to all members (incl. us). */
  sendPing(x: number, y: number): void {
    const sceneId = this.viewedSceneId;
    if (!sceneId) return;
    this.#ws?.send({ type: "scene_ping", scene: sceneId, x, y });
  }

  /** Request a grid A* path on the server. Thin delegate to `WsClient.pathfind`;
   * rejects immediately when there is no live transport. */
  pathfind(
    scene: string,
    start: [number, number],
    waypoints: [number, number][],
    footprintRadius: number,
  ): Promise<PathResult> {
    if (!this.#ws) return Promise.reject(new Error("not connected"));
    return this.#ws.pathfind(scene, start, waypoints, footprintRadius);
  }

  /** Request server-authoritative move execution for `tokenId` along `path` on
   * `scene`. Resolves with the broadcast `MoveStream` when the server confirms;
   * rejects immediately when there is no live transport. Animation is broadcast-driven
   * via `onMoveStream` for all scene viewers; the resolve value signals success only. */
  moveRequest(
    scene: string,
    tokenId: string,
    path: [number, number][],
  ): Promise<MoveStream> {
    if (!this.#ws) return Promise.reject(new Error("not connected"));
    return this.#ws.moveRequest(scene, tokenId, path);
  }

  /** Send a chat message. Fire-and-forget; no-op when disconnected — thin delegate
   * to `WsClient.sendChatMessage`. */
  sendChatMessage(opts: {
    channel: string;
    content: string;
    actorOwner?: WireActorOwnerRef | null;
    audience?: WireAudience;
  }): void {
    this.#ws?.sendChatMessage(opts);
  }

  /** Edit an existing chat message. Fire-and-forget; no-op when disconnected. */
  editChatMessage(messageId: string, content: string): void {
    this.#ws?.editChatMessage(messageId, content);
  }

  /** Delete an existing chat message. Fire-and-forget; no-op when disconnected. */
  deleteChatMessage(messageId: string): void {
    this.#ws?.deleteChatMessage(messageId);
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
          // Cross-scene guard: a scene_ping broadcasts room-wide and must render only for
          // recipients currently viewing that scene (a GM roaming scene B must not surface a
          // ping for scene A superimposed on B's grid, and vice versa). Mirrors the onMoveStream
          // scene filter above.
          if (msg.scene !== this.viewedSceneId) return;
          for (const cb of this.#pingListeners) cb(msg);
        },
      },
    });
    // Broadcast-driven animation: drive all scene viewers (mover + observers) from the
    // MoveStream frame. serverNow() aligns startServerMs to local time for catch-up.
    // Coupling: sceneInteraction.animateSamples no-ops until Stage attaches the engine.
    const ws = this.#ws;
    // Unsub return discarded: the listener's lifetime equals this WsClient instance
    // (a fresh WsClient is created per enter() and discarded on leave()).
    this.#ws.onMoveStream((stream) => {
      // Cross-scene guard: a MoveStream broadcasts room-wide and is animated only if it targets the
      // scene THIS client is viewing (a GM roaming scene B must not animate scene A's move, and must
      // animate B's). `viewedSceneId` is the GM's local view when roaming, else the followed
      // `activeScene`. Fail-closed: a stream for any other scene is dropped (latent cross-scene
      // fog/animation leak, mirrors engine.ts's toVisibility scene filter).
      if (stream.scene !== this.viewedSceneId) return;
      this.sceneInteraction.animateSamples(
        stream.tokenId,
        stream.samples,
        stream.durationMs,
        stream.startServerMs,
        () => ws.serverNow(),
        stream.moverVision,
      );
    });
    await this.#ws.start();
    this.state = "open";
  }

  async #onWelcome(w: WireWelcome): Promise<void> {
    try {
      this.role = w.user_role;
      this.#worldGrants = w.world_default_grants;
      this.#requirements = w.capability_requirements;
      // Snapshot BEFORE any await below: a scene subscription added while this
      // Welcome's async chain is still in flight (module activation / external-module
      // load / member fetch) already self-establishes via `subscribeScene`'s own
      // `#establishScene` call — reconciling it again here too would double-send
      // `scene_subscribe` for the very sub this Welcome never actually dropped.
      const subsAtWelcome = [...this.#sceneSubs];
      // Activate modules BEFORE any await below (the member fetch) so the
      // layout module contributes Layout into the `root` surface the host renders
      // — the table chrome paints immediately on mount, never a blank frame during
      // the member-fetch round-trip. `#bootstrapped` set before the await so a
      // second Welcome (reconnect) cannot re-enter and double-add the modules.
      if (!this.#bootstrapped) {
        this.#bootstrapped = true;
        for (const m of this.opts.modules) this.#modules.add(m);
        await this.#modules.activate();
        await this.#loadExternalModules(w.world, w.server_version);
      }
      // Fetch member usernames: every role needs these to resolve chat author
      // names and whisper recipient labels; the GM additionally uses them for
      // see-as labels. Best-effort: a failure leaves those UIs on short-id
      // fallback. The members SvelteMap is mutated in place, so consumers
      // (already rendered after activation) populate reactively when this
      // resolves.
      try {
        const list = await listWorldMembers(w.world);
        // Mutate in place (not reassign) so the AppContext-captured reference
        // stays valid; reconnect re-populates the same Map.
        this.members.clear();
        for (const m of list) this.members.set(m.user, m.username);
      } catch (e) {
        this.#logger.warn("member list fetch failed", e);
      }
      reconcileTopology(this.#modules.declarations(), w.contract_declarations, this.#logger);
      // Scene subscriptions are dropped by the WS on disconnect; re-establish each
      // on every (re)connect so derived state (vision) survives a reconnect. No-op
      // on the first Welcome (none registered until the render engine subscribes).
      // Iterates the PRE-await snapshot (`subsAtWelcome`), not the live map, so a sub
      // added mid-flight (see snapshot comment above) is left to its own establish.
      for (const [id, rec] of subsAtWelcome) {
        // Liveness check: `subsAtWelcome` holds `[id, rec]` by reference, so a caller
        // that unsubscribes during this Welcome's (now-widened) await window has
        // already removed `id` from the live `#sceneSubs` map. Skip a torn-down entry
        // instead of resurrecting it with a spurious `scene_subscribe`.
        if (this.#sceneSubs.get(id) !== rec) continue;
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

  /** Fetch the world's enabled installed-module set + their (manifest,
   * entry_url) pairs and load them through the shared, per-module-contained
   * loader (M13-1 §3). Runs exactly once per WorldSession (called only inside
   * the `#bootstrapped` guard) — external modules never hot-reload across a
   * reconnect within one session (no hot unload, M13-1 §2); "next client load
   * of that world" means a fresh WorldSession (page load / re-enter), not a
   * WS reconnect. A discovery-level failure (network, malformed response)
   * degrades to a logged warning; the session still enters the world with
   * only its first-party modules active — a broken pipeline must never brick
   * a world (invariant 4). */
  async #loadExternalModules(world: string, serverVersion: string): Promise<void> {
    try {
      const [enabledIds, installed] = await Promise.all([
        getEnabledModules(world),
        listInstalledModules(),
      ]);
      const byId = new Map<string, (typeof installed)[number]>();
      for (const info of installed) {
        const id = (info.manifest as { id?: unknown }).id;
        if (typeof id === "string") byId.set(id, info);
      }
      const entries: ModuleEntry[] = [];
      for (const id of enabledIds) {
        const info = byId.get(id);
        if (!info) {
          this.#logger.warn(`enabled module ${id} is not installed; skipping`);
          continue;
        }
        entries.push({
          manifest: info.manifest as ModuleManifest,
          entry: info.entry_url,
        });
      }
      if (entries.length === 0) return;
      const result = await loadModules({
        entries,
        importFn: (url) => import(/* @vite-ignore */ url),
        registry: this.#modules,
        shadowcatVersion: serverVersion,
      });
      for (const f of result.failed) {
        this.#logger.warn(`external module ${f.id} (${f.entry}) failed to load: ${f.error}`);
      }
      if (result.loaded.length > 0) await this.#modules.activate();
    } catch (e) {
      this.#logger.warn("external module discovery failed", e);
    }
  }

  leave(): void {
    this.#ws?.stop();
    this.#ws = null;
    this.state = "closed";
    this.role = null;
    this.world = null;
    this.#gmViewedScene = null;
  }
}
