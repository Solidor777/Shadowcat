// Thin delivery adapter: turns discovered (manifest, entry) pairs into Module
// objects via an injectable importFn and hands them to the registry. Discovery
// (filesystem in Node, fetch in the browser) is the host's job; the adapter
// stays environment-neutral so a future sandboxed delivery is another importFn.
// Every entry loads in isolation: a parse/compat/import/id-mismatch failure on
// one entry never aborts the batch — a broken community module must degrade to
// a reported failure, never brick every other module in the load list.
import { ModuleRegistry, type Module } from "./modules";
import { parseManifest, type ModuleManifest } from "./manifest";
import { satisfies } from "./semver";

/** The environment's dynamic import, resolving either a bare `Module` or an ESM
 * `{ default: Module }` shape (see `normalize`). */
export type ImportFn = (entry: string) => Promise<{
  /** The module, when the imported entry uses a default export. */
  default: Module;
} | Module>;

/** One discovered (manifest, entry) pair `loadModules` will attempt to load. */
export interface ModuleEntry {
  /** The entry's discovered manifest, re-validated by `loadModules` before import. */
  manifest: ModuleManifest;
  /** The importable specifier/URL passed to `ImportFn`. */
  entry: string;
}

/** One entry that failed to load, with its declared id and the failure reason. */
export interface ModuleLoadFailure {
  /** The failing entry's declared manifest id. */
  id: string;
  /** The failing entry's importable specifier/URL. */
  entry: string;
  /** The failure reason (an `Error`'s message, or `String(e)` for a non-`Error` throw). */
  error: string;
}

/** Options for `loadModules`. */
export interface LoadModulesOptions {
  /** The discovered manifest/entry pairs to load, in order. */
  entries: ModuleEntry[];
  /** The environment's dynamic import. */
  importFn: ImportFn;
  /** The registry successful imports are added to. */
  registry: ModuleRegistry;
  /** When provided, each entry's `engines.shadowcat` (if declared) is checked
   * against this version before import (load-time gate). */
  shadowcatVersion?: string;
}

/** The per-batch outcome of `loadModules` — every entry is contained; a batch never throws. */
export interface ModuleLoadResult {
  /** Module ids successfully imported and added to the registry. */
  loaded: string[];
  /** Entries that failed at any stage (manifest parse, engine compat, import, id mismatch). */
  failed: ModuleLoadFailure[];
}

/** The ESM default-export shape `normalize` unwraps. */
type DefaultExport = {
  /** The module, when the imported entry uses a default export. */
  default: Module;
};

/** Unwraps a default-exported module from a `ModuleEntry`'s raw import result.
 * @param imported The value resolved by `ImportFn` — either a `Module` or an `{ default: Module }` ESM shape.
 * @returns The `Module`.
 * @example
 * ```
 * normalize({ default: { manifest: { id: "example", version: "1.0.0", dependencies: {} }, register() {} } });
 * ```
 */
function normalize(imported: DefaultExport | Module): Module {
  return "default" in imported && (imported as DefaultExport).default
    ? (imported as DefaultExport).default
    : (imported as Module);
}

/** Throws when `manifest.engines.shadowcat` is set and `shadowcatVersion` does
 * not satisfy it. A missing `engines.shadowcat` is NOT an error here — the
 * field is optional on the shared manifest shape (first-party modules never
 * set it); the modules-folder pipeline's enable/load gate is what makes it
 * effectively required for community modules.
 * @param manifest The module's manifest.
 * @param shadowcatVersion The running host's version, checked against `manifest.engines.shadowcat`.
 * @example
 * ```
 * checkEngineCompat({ id: "example", version: "1.0.0", dependencies: {}, engines: { shadowcat: "^1.0.0" } }, "1.2.0");
 * ```
 */
function checkEngineCompat(manifest: ModuleManifest, shadowcatVersion: string): void {
  const range = manifest.engines?.shadowcat;
  if (!range) return;
  if (!satisfies(shadowcatVersion, range)) {
    throw new Error(
      `module ${manifest.id} requires shadowcat ${range}, running ${shadowcatVersion}`,
    );
  }
}

/** Imports and registers every discovered module entry, in order. Per-entry
 * contained: a manifest-parse, engine-compat, import, or id-mismatch failure
 * on one entry is collected in `failed` and never aborts the batch.
 * @param opts Load options.
 * @returns The ids that loaded, and the entries that failed with a reason.
 * @example
 * ```ts
 * import {
 *   loadModules,
 *   ModuleRegistry,
 *   HookBus,
 *   ServiceRegistry,
 *   MiddlewareChain,
 *   DocumentStore,
 *   OptimisticClient,
 *   ContributionRegistry,
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
 *   contributions: new ContributionRegistry(),
 * });
 * await loadModules({
 *   entries: [],
 *   importFn: async (entry) => ({
 *     manifest: { id: entry, version: "1.0.0", dependencies: {} },
 *     register() {},
 *   }),
 *   registry,
 * });
 * ```
 */
export async function loadModules(opts: LoadModulesOptions): Promise<ModuleLoadResult> {
  const loaded: string[] = [];
  const failed: ModuleLoadFailure[] = [];
  for (const { manifest, entry } of opts.entries) {
    try {
      // Validates the *discovered* manifest; ModuleRegistry.add re-parses the
      // module's *own* manifest. Two distinct sources, bridged by the id check
      // below — both parses are intentional.
      parseManifest(manifest);
      // typeof-check (not truthy-check): an empty-string shadowcatVersion must still
      // run the gate, failing closed via `satisfies("", range)`'s semver parse error,
      // rather than being treated as "omitted" and silently skipping the gate.
      if (typeof opts.shadowcatVersion === "string") {
        checkEngineCompat(manifest, opts.shadowcatVersion);
      }
      const module = normalize(await opts.importFn(entry));
      if (module.manifest.id !== manifest.id) {
        throw new Error(
          `module at ${entry} declares id ${module.manifest.id}, manifest says ${manifest.id}`,
        );
      }
      opts.registry.add(module);
      loaded.push(manifest.id);
    } catch (e) {
      failed.push({
        id: manifest.id,
        entry,
        error: e instanceof Error ? e.message : String(e),
      });
    }
  }
  return { loaded, failed };
}
