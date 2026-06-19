// Named singletons modules provide for others to consume. Duplicate names are a
// hard error (no silent override); a module's services are removed on unload.
interface Entry {
  impl: unknown;
  version: string;
  module?: string;
}

export class ServiceRegistry {
  private entries = new Map<string, Entry>();

  provide<T>(name: string, impl: T, opts: { module?: string; version: string }): void {
    if (this.entries.has(name)) {
      throw new Error(`service ${name} already provided`);
    }
    this.entries.set(name, { impl, version: opts.version, module: opts.module });
  }

  get<T>(name: string): T | undefined {
    return this.entries.get(name)?.impl as T | undefined;
  }

  has(name: string): boolean {
    return this.entries.has(name);
  }

  versionOf(name: string): string | undefined {
    return this.entries.get(name)?.version;
  }

  removeModule(moduleId: string): void {
    for (const [name, e] of this.entries) {
      if (e.module === moduleId) this.entries.delete(name);
    }
  }
}
