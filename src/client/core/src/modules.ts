// Import-agnostic module registry: validates manifests, resolves dependencies
// (presence + semver), activates in topological order, and tracks every
// registration per module so unload is a clean teardown. This is the trust
// chokepoint — each module sees only the capability-scoped ModuleContext. How a
// Module object is produced (dynamic import, static host wiring, future sandbox)
// is the loader adapter's concern, never the registry's.
import { HookBus, type HookDefinition, type Handler, type OnOptions } from "./hooks";
import { ServiceRegistry } from "./services";
import { MiddlewareChain, type Middleware, type PipelineName } from "./middleware";
import { DocumentStore } from "./store";
import { OptimisticClient } from "./optimistic";
import type { Logger } from "./logger";
import {
  parseManifest,
  declarationOf,
  type ModuleManifest,
  type CapRequirement,
  type ContractDeclaration,
} from "./manifest";
import { ContributionRegistry, type Contribution } from "./contributions";
import { satisfies } from "./semver";

export interface ModuleContext {
  hooks: {
    defineHook(name: string, def: HookDefinition): void;
    on(name: string, handler: Handler<unknown>, opts?: OnOptions): () => void;
    emitInfo(name: string, payload: unknown): Promise<void>;
    emitMutate<P>(name: string, payload: P): Promise<P>;
    emitCancel(name: string, payload: unknown): Promise<{ cancelled: boolean; by?: string }>;
  };
  services: {
    provide<T>(name: string, impl: T, opts: { version: string }): void;
    get<T>(name: string): T | undefined;
    has(name: string): boolean;
  };
  use<C>(pipeline: PipelineName, mw: Middleware<C>): void;
  contributions: {
    contribute(c: Contribution): () => void;
  };
  store: DocumentStore;
  client: OptimisticClient;
  logger: Logger;
  moduleId: string;
}

export interface Module {
  manifest: ModuleManifest;
  register(ctx: ModuleContext): void | Promise<void>;
  unregister?(): void | Promise<void>;
}

export interface ModuleInfo {
  id: string;
  version: string;
  active: boolean;
}

interface Deps {
  hooks: HookBus;
  services: ServiceRegistry;
  middleware: MiddlewareChain;
  store: DocumentStore;
  client: OptimisticClient;
  logger: Logger;
  contributions: ContributionRegistry;
}

interface ModuleRecord {
  module: Module;
  active: boolean;
}

export class ModuleRegistry {
  private records = new Map<string, ModuleRecord>();

  constructor(private readonly deps: Deps) {}

  add(module: Module): void {
    parseManifest(module.manifest); // throws on invalid
    const id = module.manifest.id;
    if (this.records.has(id)) throw new Error(`module ${id} already added`);
    this.records.set(id, { module, active: false });
  }

  list(): ModuleInfo[] {
    return [...this.records.values()].map((r) => ({
      id: r.module.manifest.id,
      version: r.module.manifest.version,
      active: r.active,
    }));
  }

  collectRequirements(): CapRequirement[] {
    const out: CapRequirement[] = [];
    for (const r of this.records.values()) {
      if (r.active) out.push(...(r.module.manifest.requirements ?? []));
    }
    return out;
  }

  /** Active modules projected to their UI contract declarations. */
  declarations(): ContractDeclaration[] {
    return [...this.records.values()]
      .filter((r) => r.active)
      .map((r) => declarationOf(r.module.manifest));
  }

  // A singleton-conflict throw (below) is non-recoverable: it aborts the pass
  // with earlier modules already active, so the registry must be discarded, not
  // retried.
  async activate(): Promise<void> {
    const order = this.topoSort(); // throws on cycle
    for (const id of order) {
      const r = this.records.get(id)!;
      if (r.active) continue;
      if (!this.depsSatisfied(r.module)) {
        this.deps.logger.warn(`module ${id} not activated: dependency unmet`);
        continue;
      }
      // Loud-fail: a singleton contract must have exactly one active provider.
      for (const p of r.module.manifest.provides ?? []) {
        if (p.cardinality === "singleton") {
          const others = this.activeProvidersOf(p.contract).filter((x) => x !== id);
          if (others.length > 0) {
            throw new Error(
              `singleton contract ${p.contract} already provided by ${others[0]}`,
            );
          }
        }
      }
      await r.module.register(this.contextFor(id));
      r.active = true;
    }
  }

  async unload(id: string, opts: { cascade?: boolean } = {}): Promise<void> {
    const r = this.records.get(id);
    if (!r) return;
    const dependents = this.activeDependentsOf(id);
    if (dependents.length > 0) {
      if (!opts.cascade) {
        throw new Error(`cannot unload ${id}: modules depend on it: ${dependents.join(", ")}`);
      }
      for (const dep of dependents) await this.unload(dep, { cascade: true });
    }
    if (r.active && r.module.unregister) await r.module.unregister();
    this.deps.hooks.removeModule(id);
    this.deps.services.removeModule(id);
    this.deps.middleware.removeModule(id);
    this.deps.contributions.removeModule(id);
    r.active = false;
  }

  private depsSatisfied(m: Module): boolean {
    for (const [depId, range] of Object.entries(m.manifest.dependencies)) {
      const dep = this.records.get(depId);
      if (!dep || !dep.active) return false;
      if (!satisfies(dep.module.manifest.version, range)) return false;
    }
    // Every required contract needs at least one active provider.
    for (const contract of m.manifest.requires ?? []) {
      if (this.activeProvidersOf(contract).length === 0) return false;
    }
    return true;
  }

  private activeDependentsOf(id: string): string[] {
    return [...this.records.values()]
      .filter((r) => r.active && id in r.module.manifest.dependencies)
      .map((r) => r.module.manifest.id);
  }

  private topoSort(): string[] {
    // Index providers by contract so a requirer can be ordered after them.
    const providersByContract = new Map<string, string[]>();
    for (const r of this.records.values()) {
      for (const p of r.module.manifest.provides ?? []) {
        const arr = providersByContract.get(p.contract) ?? [];
        arr.push(r.module.manifest.id);
        providersByContract.set(p.contract, arr);
      }
    }
    const visited = new Set<string>();
    const onstack = new Set<string>();
    const out: string[] = [];
    const visit = (id: string, path: string[]): void => {
      if (visited.has(id)) return;
      if (onstack.has(id)) {
        throw new Error(`dependency cycle: ${[...path, id].join(" -> ")}`);
      }
      const r = this.records.get(id);
      if (!r) return; // missing dep handled by depsSatisfied
      onstack.add(id);
      for (const depId of Object.keys(r.module.manifest.dependencies)) {
        visit(depId, [...path, id]);
      }
      // Contract edges: a requirer is ordered after every provider of its
      // required contracts.
      for (const contract of r.module.manifest.requires ?? []) {
        for (const providerId of providersByContract.get(contract) ?? []) {
          visit(providerId, [...path, id]);
        }
      }
      onstack.delete(id);
      visited.add(id);
      out.push(id);
    };
    for (const id of this.records.keys()) visit(id, []);
    return out;
  }

  private contextFor(moduleId: string): ModuleContext {
    const { hooks, services, middleware, store, client, logger, contributions } = this.deps;
    return {
      moduleId,
      store,
      client,
      logger,
      hooks: {
        defineHook: (name, def) => hooks.defineHook(name, def),
        on: (name, handler, opts) => hooks.on(name, handler, { ...opts, module: moduleId }),
        emitInfo: (name, p) => hooks.emitInfo(name, p),
        emitMutate: (name, p) => hooks.emitMutate(name, p),
        emitCancel: (name, p) => hooks.emitCancel(name, p),
      },
      services: {
        provide: (name, impl, opts) => services.provide(name, impl, { ...opts, module: moduleId }),
        get: (name) => services.get(name),
        has: (name) => services.has(name),
      },
      use: (pipeline, mw) => middleware.use(pipeline, mw, { module: moduleId }),
      contributions: {
        contribute: (c) => contributions.contribute(c, { module: moduleId }),
      },
    };
  }

  /** Active modules that provide `contract`. */
  private activeProvidersOf(contract: string): string[] {
    return [...this.records.values()]
      .filter(
        (r) =>
          r.active &&
          (r.module.manifest.provides ?? []).some((p) => p.contract === contract),
      )
      .map((r) => r.module.manifest.id);
  }
}
