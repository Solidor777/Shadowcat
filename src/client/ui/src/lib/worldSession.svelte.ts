import {
  WsClient,
  OptimisticClient,
  DocumentStore,
  ContributionRegistry,
  ModuleRegistry,
  HookBus,
  ServiceRegistry,
  MiddlewareChain,
  reconcileTopology,
  consoleLogger,
  type Connect,
  type Logger,
  type Module,
  type WireWelcome,
} from "@shadowcat/core";
import type { WorldRole } from "@shadowcat/types";

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
