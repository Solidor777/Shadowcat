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
  silentLogger,
  type Connect,
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

  constructor(private readonly opts: WorldSessionOpts) {
    this.#optimistic = new OptimisticClient(opts.selfId);
    this.#modules = new ModuleRegistry({
      hooks: new HookBus(silentLogger),
      services: new ServiceRegistry(),
      middleware: new MiddlewareChain(),
      store: this.store,
      client: this.#optimistic,
      logger: silentLogger,
      contributions: this.contributions,
    });
  }

  async enter(worldId: string): Promise<void> {
    this.world = worldId;
    this.state = "connecting";
    this.#ws = new WsClient({
      connect: this.opts.connect,
      handlers: {
        onCommand: (cmd) => this.#optimistic.applyCommand(cmd),
        onReject: (id) => this.#optimistic.reject(id),
        onWelcome: (w) => {
          void this.#onWelcome(w);
        },
        onError: () => {},
      },
    });
    await this.#ws.start();
    this.state = "open";
  }

  async #onWelcome(w: WireWelcome): Promise<void> {
    this.role = w.actor_role;
    this.#modules.add(this.opts.coreUiModule);
    await this.#modules.activate();
    reconcileTopology(this.#modules.declarations(), w.contract_declarations, silentLogger);
  }

  leave(): void {
    this.#ws?.stop();
    this.#ws = null;
    this.state = "closed";
    this.role = null;
    this.world = null;
  }
}
