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
  normalizeRequires,
  type ModuleManifest,
  type CapRequirement,
  type ContractDeclaration,
} from "./manifest";
import { ContributionRegistry, type Contribution, SYSTEM_CONTRACT } from "./contributions";
import { satisfies } from "./semver";
import type { I18n, Messages } from "./i18n";

/** Options for `ModuleContext.services.provide` — narrower than `ServiceProvideOptions`
 * (no `module` field: the wrapper stamps the calling module's id automatically). */
export interface ModuleServiceProvideOptions {
  /** The provided implementation's semver version. */
  version: string;
}

/** Options for `ModuleRegistry.unload`. */
export interface ModuleUnloadOptions {
  /** When `true`, first recursively unloads every currently active module that lists the
   * unloading module as a dependency; when omitted/`false`, unloading a module with active
   * dependents throws instead. */
  cascade?: boolean;
}

/** The capability-scoped surface a module's `register` receives — the trust chokepoint every
 * module is confined to (`contextFor` builds one per module, stamping `moduleId` onto each
 * registration so `unload` can find and strip them again). A module never reaches the host's
 * raw `HookBus`/`ServiceRegistry`/etc. singletons directly, only these wrappers. */
export interface ModuleContext {
  /** Hook registration/emission, scoped to this module for listener teardown. */
  hooks: {
    /** Defines a hook. Passes straight through to `HookBus.defineHook` unstamped — a
     * definition carries no module ownership, unlike a listener (see `contextFor`'s doc).
     * @param name The hook's name.
     * @param def The hook's definition. */
    defineHook(name: string, def: HookDefinition): void;
    /** Registers a listener, stamped with this module's id so `unload` can remove it.
     * @param name The hook's name.
     * @param handler Called when the hook fires.
     * @param opts Listener options.
     * @returns An unsubscribe function. */
    on(name: string, handler: Handler<unknown>, opts?: OnOptions): () => void;
    /** Emits an info-kind hook (fire-and-forget notification to listeners).
     * @param name The hook's name.
     * @param payload The value passed to every listener.
     * @returns Resolves once every listener has run. */
    emitInfo(name: string, payload: unknown): Promise<void>;
    /** Emits a mutate-kind hook (listeners may transform and return the payload).
     * @param name The hook's name.
     * @param payload The initial value, threaded through each listener in turn.
     * @returns The payload after every listener's transformation. */
    emitMutate<P>(name: string, payload: P): Promise<P>;
    /** Emits a cancel-kind hook (any listener may veto).
     * @param name The hook's name.
     * @param payload The value passed to every listener.
     * @returns Whether a listener vetoed, and which one. */
    emitCancel(name: string, payload: unknown): Promise<{
      /** `true` iff a listener vetoed. */
      cancelled: boolean;
      /** The vetoing listener's identity, when cancelled. */
      by?: string;
    }>;
  };
  /** Service provision/lookup, scoped to this module for registration teardown. */
  services: {
    /** Provides a named service implementation, stamped with this module's id.
     * @param name The service name.
     * @param impl The implementation to provide.
     * @param opts Provision options. */
    provide<T>(name: string, impl: T, opts: ModuleServiceProvideOptions): void;
    /** Looks up a named service; `undefined` if none is provided.
     * @param name The service name.
     * @returns The implementation, or `undefined` if none is provided. */
    get<T>(name: string): T | undefined;
    /** Whether a named service is currently provided.
     * @param name The service name.
     * @returns `true` iff a service is currently provided under `name`. */
    has(name: string): boolean;
  };
  /** Registers middleware into a pipeline, stamped with this module's id.
   * @param pipeline The pipeline name to register into.
   * @param mw The middleware to register. */
  use<C>(pipeline: PipelineName, mw: Middleware<C>): void;
  /** UI contribution registration, scoped to this module for teardown. */
  contributions: {
    /** Registers a contribution, stamped with this module's id.
     * @param c The contribution to register.
     * @returns A dispose function that removes it. */
    contribute(c: Contribution): () => void;
  };
  /** Module-supplied i18n catalog registration, scoped to this module for teardown. */
  i18n: {
    /** Merges `messages` into `locale`'s host catalog, stamped with this module's id so
     * `unload` can remove them again. See `I18n.addMessages` for the merge/collision semantics
     * (last write wins, including over a built-in key).
     * @param locale The locale to merge into.
     * @param messages The key→message fragment to merge in. */
    addMessages(locale: string, messages: Messages): void;
  };
  /** The shared document store every module reads through. */
  store: DocumentStore;
  /** The shared optimistic client every module dispatches intents through. */
  client: OptimisticClient;
  /** Logger scoped to this context (not itself module-tagged by `ModuleContext`; callers
   * needing per-module log attribution do so at the call site). */
  logger: Logger;
  /** This module's id, as declared in its manifest. */
  moduleId: string;
  /** Whether this module currently holds a `singleton` contract it claims in its own `provides` —
   * `true` for any contract it doesn't claim at all (nothing to lose), `false` only if it claims
   * the contract AND lost the collision to another module. Most modules never need this:
   * `provides` is purely declarative and nothing gates a module's own registrations against it —
   * this exists for a module whose `register()` does something contract-specific it must skip on
   * a loss (e.g. avoiding a double-mount into a shared surface).
   * @param contract The contract id to check.
   * @returns Whether this module currently holds it. */
  isSingletonProvider(contract: string): boolean;
}

/** A registrable unit: a manifest plus lifecycle hooks. */
export interface Module {
  /** The module's manifest (validated by `ModuleRegistry.add` via `parseManifest`). */
  manifest: ModuleManifest;
  /** Called once, in dependency order, when the module activates; receives its
   * capability-scoped `ModuleContext`.
   * @param ctx This module's capability-scoped context.
   * @returns Resolves (or returns) once registration is complete. */
  register(ctx: ModuleContext): void | Promise<void>;
  /** Called on unload, if the module is currently active; optional (a module with no
   * teardown logic beyond its registrations, which `unload` strips automatically, may omit it).
   * @returns Resolves (or returns) once teardown is complete. */
  unregister?(): void | Promise<void>;
}

/** A registered module's identity and activation state, as returned by `ModuleRegistry.list`. */
export interface ModuleInfo {
  /** The module's id. */
  id: string;
  /** The module's declared semver version. */
  version: string;
  /** Whether the module is currently active. */
  active: boolean;
}

/** The host-provided singletons a `ModuleRegistry` is constructed over. Not exported. */
interface Deps {
  /** The shared hook bus every module's `hooks` wrapper delegates to. */
  hooks: HookBus;
  /** The shared service registry every module's `services` wrapper delegates to. */
  services: ServiceRegistry;
  /** The shared middleware chain every module's `use` delegates to. */
  middleware: MiddlewareChain;
  /** The shared document store passed through to every `ModuleContext`. */
  store: DocumentStore;
  /** The shared optimistic client passed through to every `ModuleContext`. */
  client: OptimisticClient;
  /** The shared logger passed through to every `ModuleContext`. */
  logger: Logger;
  /** The shared contribution registry every module's `contributions` wrapper delegates to. */
  contributions: ContributionRegistry;
  /** The shared i18n instance every module's `i18n.addMessages` wrapper delegates to. */
  i18n: I18n;
}

/** Internal per-module bookkeeping: the module plus its current activation state. Not exported. */
interface ModuleRecord {
  /** The registered module. */
  module: Module;
  /** Whether this module is currently active. */
  active: boolean;
  /** Singleton contract ids this module's own `provides` claims but currently does NOT hold —
   * lost to another module's higher priority or earlier activation order. Recomputed fresh at the
   * top of every `activate()` pass (never accumulates stale state across calls). */
  demoted: Set<string>;
}

/** The import-agnostic module registry: validates manifests, resolves
 * dependencies (presence + semver, plus provides/requires contract edges),
 * activates in topological order, and tracks every registration per module so
 * unload is a clean teardown. The trust chokepoint — each module sees only the
 * capability-scoped `ModuleContext` `contextFor` builds for it. */
export class ModuleRegistry {
  /** Every registered module, keyed by id. */
  private records = new Map<string, ModuleRecord>();

  /** Builds a registry scoped over the given host dependencies.
   * @param deps The host-provided singletons (hook bus, service/contribution
   * registries, document store/client, logger) every activated module's
   * `ModuleContext` is scoped over.
   * @example
   * ```ts
   * import {
   *   ModuleRegistry,
   *   HookBus,
   *   ServiceRegistry,
   *   MiddlewareChain,
   *   DocumentStore,
   *   OptimisticClient,
   *   ContributionRegistry,
   *   I18n,
   *   silentLogger,
   * } from "@shadowcat/core";
   *
   * const registry = new ModuleRegistry({
   *   hooks: new HookBus(silentLogger),
   *   services: new ServiceRegistry(),
   *   middleware: new MiddlewareChain(),
   *   store: new DocumentStore(),
   *   client: new OptimisticClient("00000000-0000-0000-0000-000000000001"),
   *   logger: silentLogger,
   *   i18n: new I18n("en", { en: {} }),
   *   contributions: new ContributionRegistry(),
   * });
   * ```
   */
  constructor(private readonly deps: Deps) {}

  /** Registers a module. Validates its manifest (throws on an invalid one, e.g. a
   * malformed id or version) before recording it; the module stays inactive until
   * `activate()` runs.
   * @param module The module to register.
   * @example
   * ```ts
   * import { ModuleRegistry, type Module } from "@shadowcat/core";
   *
   * declare const registry: ModuleRegistry;
   * declare const module: Module;
   * registry.add(module);
   * ```
   */
  add(module: Module): void {
    parseManifest(module.manifest); // throws on invalid
    const id = module.manifest.id;
    if (this.records.has(id)) throw new Error(`module ${id} already added`);
    this.records.set(id, { module, active: false, demoted: new Set() });
  }

  /** Every registered module's id, version, and active state.
   * @returns One `ModuleInfo` per registered module, in registration order.
   * @example
   * ```ts
   * import { ModuleRegistry } from "@shadowcat/core";
   *
   * declare const registry: ModuleRegistry;
   * const modules = registry.list();
   * ```
   */
  list(): ModuleInfo[] {
    return [...this.records.values()].map((r) => ({
      id: r.module.manifest.id,
      version: r.module.manifest.version,
      active: r.active,
    }));
  }

  /** Every active module's declared capability requirements, concatenated. Has
   * no production caller today — unlike the structurally similar
   * `declarations()`, which `WorldSession.#onWelcome` wires into
   * `reconcileTopology`. See `CapRequirement` for how a module's `requirements`
   * reach the server's per-world `capability_requirements` record.
   * @returns The concatenated `requirements` of every currently active module.
   * @example
   * ```ts
   * import { ModuleRegistry } from "@shadowcat/core";
   *
   * declare const registry: ModuleRegistry;
   * const requirements = registry.collectRequirements();
   * ```
   */
  collectRequirements(): CapRequirement[] {
    const out: CapRequirement[] = [];
    for (const r of this.records.values()) {
      if (r.active) out.push(...(r.module.manifest.requirements ?? []));
    }
    return out;
  }

  /** Active modules projected to their UI contract declarations.
   * @returns One `ContractDeclaration` per active module.
   * @example
   * ```ts
   * import { ModuleRegistry } from "@shadowcat/core";
   *
   * declare const registry: ModuleRegistry;
   * const declarations = registry.declarations();
   * ```
   */
  declarations(): ContractDeclaration[] {
    return [...this.records.values()]
      .filter((r) => r.active)
      .map((r) => {
        const d = declarationOf(r.module.manifest);
        return { ...d, provides: d.provides.filter((p) => !r.demoted.has(p.contract)) };
      });
  }

  /** Activates every registered, not-yet-active module in dependency order
   * (`topoSort`: explicit `dependencies` plus provides/requires contract edges).
   * A module whose dependencies aren't satisfied is skipped (warned, left
   * inactive) rather than failing the batch. Per-module isolated beyond that: a
   * throwing `register()` is caught, logged, and left inactive WITHOUT aborting
   * the topological loop — one broken module (first-party or external) must
   * never brick the modules ordered after it. A losing singleton-contract
   * claimant is demoted (`ModuleRecord.demoted`) rather than aborted: it still
   * activates in full, it just doesn't become the active provider of the one
   * contract it lost (`computeSingletonWinners` decides the winner). An
   * already-active module is skipped as a no-op.
   * @returns Resolves once every module has been attempted (activated, skipped,
   * or rolled back on failure) — a single module's activation failure never
   * rejects the call. Rejects only if the dependency graph itself has a cycle
   * (`topoSort`, thrown before any module is touched).
   * @example
   * ```ts
   * import { ModuleRegistry } from "@shadowcat/core";
   *
   * declare const registry: ModuleRegistry;
   * await registry.activate();
   * ```
   */
  async activate(): Promise<void> {
    const order = this.topoSort(); // throws on cycle
    const singletonWinners = this.computeSingletonWinners(order);
    for (const id of order) {
      const r = this.records.get(id)!;
      if (r.active) continue;
      if (!this.depsSatisfied(r.module)) {
        this.deps.logger.warn(`module ${id} not activated: dependency unmet`);
        continue;
      }
      r.demoted = new Set();
      for (const p of r.module.manifest.provides ?? []) {
        if (p.cardinality === "singleton" && singletonWinners.get(p.contract) !== id) {
          const winner = singletonWinners.get(p.contract);
          this.deps.logger.warn(
            `module ${id} lost singleton contract ${p.contract} to ${winner}; ${id} still activates but does not provide it`,
          );
          r.demoted.add(p.contract);
        }
      }
      try {
        await r.module.register(this.contextFor(id));
        r.active = true;
      } catch (e) {
        this.deps.logger.warn(
          `module ${id} failed to activate: ${e instanceof Error ? e.message : String(e)}`,
        );
        // r.active is still false here (only set true after register() returns),
        // so unload's activeDependentsOf/unregister guards correctly no-op — this
        // only tears down whatever partial hooks/services/contributions register()
        // made before throwing. unload() itself is contained too: a throw here must
        // not propagate out of this per-module catch and abort the loop for the
        // modules still ordered after `id`.
        try {
          await this.unload(id);
        } catch (unloadErr) {
          this.deps.logger.warn(
            `module ${id} failed to unload during activation rollback: ${
              unloadErr instanceof Error ? unloadErr.message : String(unloadErr)
            }`,
          );
        }
      }
    }
  }

  /** Unloads a module: calls its `unregister()` (if active and provided), then
   * strips every hook/service/middleware/contribution registration it made, and
   * marks it inactive. A no-op if `id` isn't registered.
   * @param id The module id to unload.
   * @param opts Unload options.
   * @example
   * ```ts
   * import { ModuleRegistry } from "@shadowcat/core";
   *
   * declare const registry: ModuleRegistry;
   * await registry.unload("example-module", { cascade: true });
   * ```
   */
  async unload(id: string, opts: ModuleUnloadOptions = {}): Promise<void> {
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
    this.deps.i18n.removeModule(id);
    r.active = false;
  }

  /** Whether every explicit `dependencies` entry (present, active, and
   * satisfying the required semver range) and every `requires` contract (at
   * least one active provider, additionally satisfying `ContractRequire.version`
   * when set) of `m` is currently satisfied. Not exported —
   * folded into `activate`'s public surface.
   * @param m The module to check.
   * @returns `true` if `m` is eligible to activate right now.
   * @example
   * ```
   * // internal helper; not part of the public API
   * declare const module: Module;
   * this.depsSatisfied(module);
   * ```
   */
  private depsSatisfied(m: Module): boolean {
    for (const [depId, range] of Object.entries(m.manifest.dependencies)) {
      const dep = this.records.get(depId);
      if (!dep || !dep.active) return false;
      if (!satisfies(dep.module.manifest.version, range)) return false;
    }
    // Every required contract needs at least one active provider; a `ContractRequire.version`
    // range additionally requires at least one active provider whose own module `version`
    // satisfies it (mirrors `dependencies`' version matching above, against a provider's version
    // rather than a fixed dependency target).
    for (const req of normalizeRequires(m.manifest.requires)) {
      const providers = this.activeProvidersOf(req.contract);
      if (providers.length === 0) return false;
      if (req.version) {
        const anySatisfies = providers.some((id) => {
          const provider = this.records.get(id)?.module;
          return provider ? satisfies(provider.manifest.version, req.version!) : false;
        });
        if (!anySatisfies) return false;
      }
    }
    return true;
  }

  /** Active modules that list `id` in their `dependencies`. Not exported —
   * folded into `unload`'s public surface.
   * @param id The module id to find dependents of.
   * @returns The ids of every active module depending on `id`.
   * @example
   * ```
   * // internal helper; not part of the public API
   * this.activeDependentsOf("example-module");
   * ```
   */
  private activeDependentsOf(id: string): string[] {
    return [...this.records.values()]
      .filter((r) => r.active && id in r.module.manifest.dependencies)
      .map((r) => r.module.manifest.id);
  }

  /** Topologically orders every registered module over two edge kinds: explicit
   * `dependencies` and provides/requires contract edges (a requirer after every
   * active-or-not provider of a contract it `requires`). Not exported — folded
   * into `activate`'s public surface.
   * @returns Every registered module id, ordered so each comes after everything
   * it depends on (directly or via a contract edge).
   * @example
   * ```
   * // internal helper; not part of the public API
   * this.topoSort();
   * ```
   */
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
      // required contracts. Ordering only needs the contract id, not a version
      // range — version compatibility is checked later, in `depsSatisfied`,
      // once activation is actually attempted.
      for (const req of normalizeRequires(r.module.manifest.requires)) {
        for (const providerId of providersByContract.get(req.contract) ?? []) {
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

  /** Builds the capability-scoped `ModuleContext` passed to `Module.register`:
   * every host dependency, plus per-call wrappers over `hooks.on`/`services.provide`/
   * `use`/`contributions.contribute` that stamp `moduleId` onto each registration
   * so `unload` can find and strip them again. `hooks.defineHook` is the one
   * exception — it passes straight through to `HookBus.defineHook` unstamped,
   * since a hook DEFINITION (unlike a listener) carries no module ownership and
   * `HookBus.removeModule` never touches `defs`, only listeners. Not exported —
   * folded into `activate`'s public surface.
   * @param moduleId The id of the module this context is being built for.
   * @returns The `ModuleContext` to pass to that module's `register`.
   * @example
   * ```
   * // internal helper; not part of the public API
   * this.contextFor("example-module");
   * ```
   */
  private contextFor(moduleId: string): ModuleContext {
    const { hooks, services, middleware, store, client, logger, contributions, i18n } = this.deps;
    return {
      moduleId,
      store,
      client,
      logger,
      isSingletonProvider: (contract) => !this.records.get(moduleId)!.demoted.has(contract),
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
      i18n: {
        addMessages: (locale, messages) => i18n.addMessages(locale, messages, { module: moduleId }),
      },
    };
  }

  /** The active module currently holding the `SYSTEM_CONTRACT` singleton (the active game
   * system), or `undefined` if no active module provides it. Reads the same active/demoted
   * bookkeeping `isSingletonProvider`/`activeProvidersOf` use, so a losing claimant is excluded
   * exactly as it is everywhere else.
   * @returns The winning module, or `undefined`.
   * @example
   * ```ts
   * import { ModuleRegistry } from "@shadowcat/core";
   *
   * declare const registry: ModuleRegistry;
   * const system = registry.systemModule();
   * ```
   */
  systemModule(): Module | undefined {
    const id = this.activeProvidersOf(SYSTEM_CONTRACT)[0];
    return id ? this.records.get(id)?.module : undefined;
  }

  /** Active modules that provide `contract`. Not exported — folded into
   * `activate`'s and `depsSatisfied`'s public surface.
   * @param contract The contract id to find active providers of.
   * @returns The ids of every active module that provides `contract`.
   * @example
   * ```
   * // internal helper; not part of the public API
   * this.activeProvidersOf("shadowcat.panel");
   * ```
   */
  private activeProvidersOf(contract: string): string[] {
    return [...this.records.values()]
      .filter(
        (r) =>
          r.active &&
          !r.demoted.has(contract) &&
          (r.module.manifest.provides ?? []).some((p) => p.contract === contract),
      )
      .map((r) => r.module.manifest.id);
  }

  /** Computes the winning module id for every `singleton`-cardinality contract contested this
   * `activate()` pass. An already-ACTIVE provider always wins unconditionally (stability —
   * activation never unseats a live provider's registrations, regardless of a later-registered
   * higher-priority latecomer). Among modules not yet active, the highest `priority` (default `0`)
   * wins; ties break by earliest position in `order` (this method's own tie-break, matching the
   * pre-existing de facto behavior when no priority is set). Not exported — folded into `activate`'s
   * public surface.
   * @param order The dependency-ordered module ids `activate()` is about to process
   * (`topoSort`'s output).
   * @returns Contract id → winning module id, for every singleton contract with at least one
   * claimant.
   * @example
   * ```
   * // internal helper; not part of the public API
   * this.computeSingletonWinners(["a", "b"]);
   * ```
   */
  private computeSingletonWinners(order: string[]): Map<string, string> {
    const winners = new Map<string, string>();
    for (const r of this.records.values()) {
      if (!r.active) continue;
      for (const p of r.module.manifest.provides ?? []) {
        if (p.cardinality === "singleton" && !winners.has(p.contract)) {
          winners.set(p.contract, r.module.manifest.id);
        }
      }
    }
    const orderIndex = new Map(order.map((id, i) => [id, i]));
    const candidates = new Map<
      string,
      {
        /** The candidate module's id. */
        id: string;
        /** The candidate's `ContractProvide.priority`, defaulted to `0`. */
        priority: number;
      }[]
    >();
    for (const id of order) {
      const r = this.records.get(id);
      if (!r || r.active) continue;
      for (const p of r.module.manifest.provides ?? []) {
        if (p.cardinality !== "singleton" || winners.has(p.contract)) continue;
        const arr = candidates.get(p.contract) ?? [];
        arr.push({ id, priority: p.priority ?? 0 });
        candidates.set(p.contract, arr);
      }
    }
    for (const [contract, list] of candidates) {
      list.sort((a, b) => b.priority - a.priority || orderIndex.get(a.id)! - orderIndex.get(b.id)!);
      winners.set(contract, list[0].id);
    }
    return winners;
  }
}
