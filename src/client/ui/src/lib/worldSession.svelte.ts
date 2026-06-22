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
  type Connect,
  type Logger,
  type Module,
  type WireWelcome,
  type WireOperation,
  type SceneFrame,
  type SceneSubscription,
} from "@shadowcat/core";
import type { WorldRole } from "@shadowcat/types";
import { SceneInteractionBridge } from "./sceneInteraction";

export type ConnState = "connecting" | "open" | "closed";

export interface WorldSessionOpts {
  selfId: string;
  /** Browser: webSocketConnect(wsUrl). Tests: a mock connect. */
  connect: Connect;
  /** The first-party shell module providing region surfaces. */
  coreUiModule: Module;
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
  #assetListeners = new Set<(msg: { uuid: string; op: "replaced" | "deleted" }) => void>();
  #sceneSubs = new Map<
    string,
    { channel: string; onUpdate: (f: SceneFrame) => void; handle: SceneSubscription | null; gen: number }
  >();
  state = $state<ConnState>("closed");
  role = $state<WorldRole | null>(null);
  world = $state<string | null>(null);

  #ws: WsClient | null = null;
  #optimistic: OptimisticClient;
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

  /** Predict `ops` optimistically AND transmit them as one correlated Intent. The
   * single `intent_id` ties the local prediction to the server echo/reject (FIFO
   * confirm). The send is a no-op while disconnected (WsClient guards transport);
   * the prediction still shows locally and is reconciled on the next resync. */
  dispatchIntent(ops: WireOperation[]): void {
    const intentId = crypto.randomUUID();
    this.#optimistic.applyIntent(intentId, ops);
    this.#ws?.send({ type: "intent", intent_id: intentId, ops });
  }

  /** Subscribe to asset replace/delete notices; returns an unsubscribe. */
  onAssetChanged(cb: (msg: { uuid: string; op: "replaced" | "deleted" }) => void): () => void {
    this.#assetListeners.add(cb);
    return () => this.#assetListeners.delete(cb);
  }

  /** Subscribe to a SceneDerived channel. Returns a synchronous handle; the
   * underlying WS subscription is (re)established on every Welcome so derived state
   * survives a reconnect. */
  subscribeScene(channel: string, onUpdate: (f: SceneFrame) => void): SceneSubscription {
    const id = crypto.randomUUID();
    const rec = { channel, onUpdate, handle: null as SceneSubscription | null, gen: 0 };
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
    rec: { channel: string; onUpdate: (f: SceneFrame) => void; handle: SceneSubscription | null; gen: number },
  ): void {
    const ws = this.#ws;
    if (!ws) return;
    const gen = ++rec.gen; // this attempt's generation
    void ws
      .subscribeScene(rec.channel, rec.onUpdate)
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
        onError: (e) => this.#logger.error("world session ws error", e),
        onAssetChanged: (msg) => {
          // Bump the resolver first so a notified panel re-resolves the new URL.
          this.assets.onAssetChanged(msg);
          for (const cb of this.#assetListeners) cb(msg);
        },
      },
    });
    await this.#ws.start();
    this.state = "open";
  }

  async #onWelcome(w: WireWelcome): Promise<void> {
    try {
      this.role = w.actor_role;
      if (!this.#bootstrapped) {
        // Set before the await so a second Welcome (reconnect) cannot re-enter
        // and double-add the module.
        this.#bootstrapped = true;
        this.#modules.add(this.opts.coreUiModule);
        await this.#modules.activate();
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
