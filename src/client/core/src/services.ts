// Named singletons modules provide for others to consume. Duplicate names are a
// hard error (no silent override); a module's services are removed on unload.
/** One registered service, tagged with its owning module for bulk teardown. */
interface Entry {
  /** The registered singleton, type-erased to `unknown` for uniform storage. */
  impl: unknown;
  /** The version string passed to `provide`, returned verbatim by `versionOf`. */
  version: string;
  /** The registering module's id, for `removeModule` teardown; absent for host-registered services. */
  module?: string;
}

/** Named-singleton service directory. Registration is exclusive (no override) and
 * module-scoped for bulk teardown on unload.
 * @example
 * ```ts
 * import { ServiceRegistry } from "@shadowcat/core";
 *
 * const services = new ServiceRegistry();
 * services.provide("dice-roller", { roll: () => 4 }, { version: "1.0.0" });
 * ```
 */
export class ServiceRegistry {
  /** Registered services, keyed by lookup name. */
  private entries = new Map<string, Entry>();

  /** Registers a named singleton.
   * @param name The service's lookup name.
   * @param impl The singleton instance.
   * @param opts Registration options.
   * @param opts.module The owning module id, for later `removeModule` teardown.
   * @param opts.version The service's version, returned by `versionOf`.
   * @example
   * ```ts
   * import { ServiceRegistry } from "@shadowcat/core";
   *
   * const services = new ServiceRegistry();
   * services.provide("dice-roller", { roll: () => 4 }, { module: "m1", version: "1.0.0" });
   * ```
   */
  provide<T>(
    name: string,
    impl: T,
    opts: {
      /** The owning module id, for later `removeModule` teardown. */
      module?: string;
      /** The service's version, returned by `versionOf`. */
      version: string;
    },
  ): void {
    if (this.entries.has(name)) {
      throw new Error(`service ${name} already provided`);
    }
    this.entries.set(name, { impl, version: opts.version, module: opts.module });
  }

  /** Looks up a registered service by name.
   * @param name The service's lookup name.
   * @returns The registered instance, or `undefined` if nothing provides `name`.
   * @example
   * ```ts
   * import { ServiceRegistry } from "@shadowcat/core";
   *
   * const services = new ServiceRegistry();
   * services.get<{ roll(): number }>("dice-roller");
   * ```
   */
  get<T>(name: string): T | undefined {
    return this.entries.get(name)?.impl as T | undefined;
  }

  /** Reports whether a service is currently registered.
   * @param name The service's lookup name.
   * @returns `true` if a provider is registered for `name`.
   * @example
   * ```ts
   * import { ServiceRegistry } from "@shadowcat/core";
   *
   * const services = new ServiceRegistry();
   * services.has("dice-roller");
   * ```
   */
  has(name: string): boolean {
    return this.entries.has(name);
  }

  /** Looks up the declared version of a registered service.
   * @param name The service's lookup name.
   * @returns The `version` passed to `provide`, or `undefined` if unregistered.
   * @example
   * ```ts
   * import { ServiceRegistry } from "@shadowcat/core";
   *
   * const services = new ServiceRegistry();
   * services.versionOf("dice-roller");
   * ```
   */
  versionOf(name: string): string | undefined {
    return this.entries.get(name)?.version;
  }

  /** Removes every service provided by `moduleId` (module unload teardown).
   * @param moduleId The unloading module's id.
   * @example
   * ```ts
   * import { ServiceRegistry } from "@shadowcat/core";
   *
   * const services = new ServiceRegistry();
   * services.removeModule("m1");
   * ```
   */
  removeModule(moduleId: string): void {
    for (const [name, e] of this.entries) {
      if (e.module === moduleId) this.entries.delete(name);
    }
  }
}
