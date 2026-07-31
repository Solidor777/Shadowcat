// Thin delivery adapter: turns discovered (manifest, entry) pairs into Module
// objects via an injectable importFn and hands them to the registry. Discovery
// (filesystem in Node, fetch in the browser) is the host's job; the adapter
// stays environment-neutral so a future sandboxed delivery is another importFn.
// Every entry loads in isolation: a parse/compat/import/id-mismatch failure on
// one entry never aborts the batch — a broken community module must degrade to
// a reported failure, never brick every other module in the load list (M13-1 §3).
import { ModuleRegistry, type Module } from "./modules";
import { parseManifest, type ModuleManifest } from "./manifest";
import { satisfies } from "./semver";

export type ImportFn = (entry: string) => Promise<{ default: Module } | Module>;

export interface ModuleEntry {
  manifest: ModuleManifest;
  entry: string;
}

/** One entry that failed to load, with its declared id and the failure reason. */
export interface ModuleLoadFailure {
  id: string;
  entry: string;
  error: string;
}

export interface ModuleLoadResult {
  /** Module ids successfully imported and added to the registry. */
  loaded: string[];
  /** Entries that failed at any stage (manifest parse, engine compat, import, id mismatch). */
  failed: ModuleLoadFailure[];
}

/** Unwraps a default-exported module from a `ModuleEntry`'s raw import result.
 * @param imported The value resolved by `ImportFn` — either a `Module` or an `{ default: Module }` ESM shape.
 * @returns The `Module`.
 * @example
 * ```
 * normalize({ default: { manifest: { id: "example", version: "1.0.0", dependencies: {} }, register() {} } });
 * ```
 */
function normalize(imported: { default: Module } | Module): Module {
  return "default" in imported && (imported as { default: Module }).default
    ? (imported as { default: Module }).default
    : (imported as Module);
}

/** Throws when `manifest.engines.shadowcat` is set and `shadowcatVersion` does
 * not satisfy it. A missing `engines.shadowcat` is NOT an error here — the
 * field is optional on the shared manifest shape (first-party modules never
 * set it); the modules-folder pipeline's enable/load gate is what makes it
 * effectively required for community modules (T6).
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
 * @param opts.entries The discovered manifest/entry pairs.
 * @param opts.importFn The environment's dynamic import.
 * @param opts.registry The `ModuleRegistry` to add successful imports to.
 * @param opts.shadowcatVersion Optional running host version, for the T6 load-time engine-compat gate.
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
export async function loadModules(opts: {
  entries: ModuleEntry[];
  importFn: ImportFn;
  registry: ModuleRegistry;
  /** When provided, each entry's `engines.shadowcat` (if declared) is checked
   * against this version before import (T6 load-time gate). */
  shadowcatVersion?: string;
}): Promise<ModuleLoadResult> {
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
      // rather than being treated as "omitted" and silently skipping the T6 gate.
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
