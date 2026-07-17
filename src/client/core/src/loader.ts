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

function normalize(imported: { default: Module } | Module): Module {
  return "default" in imported && (imported as { default: Module }).default
    ? (imported as { default: Module }).default
    : (imported as Module);
}

/** Throws when `manifest.engines.shadowcat` is set and `shadowcatVersion` does
 * not satisfy it. A missing `engines.shadowcat` is NOT an error here — the
 * field is optional on the shared manifest shape (first-party modules never
 * set it); the modules-folder pipeline's enable/load gate is what makes it
 * effectively required for community modules (T6). */
function checkEngineCompat(manifest: ModuleManifest, shadowcatVersion: string): void {
  const range = manifest.engines?.shadowcat;
  if (!range) return;
  if (!satisfies(shadowcatVersion, range)) {
    throw new Error(
      `module ${manifest.id} requires shadowcat ${range}, running ${shadowcatVersion}`,
    );
  }
}

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
      if (opts.shadowcatVersion) checkEngineCompat(manifest, opts.shadowcatVersion);
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
