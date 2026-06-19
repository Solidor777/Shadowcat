// Thin delivery adapter: turns discovered (manifest, entry) pairs into Module
// objects via an injectable importFn and hands them to the registry. Discovery
// (filesystem in Node, fetch in the browser) is the host's job; the adapter
// stays environment-neutral so a future sandboxed delivery is another importFn.
import { ModuleRegistry, type Module } from "./modules";
import { parseManifest, type ModuleManifest } from "./manifest";

export type ImportFn = (entry: string) => Promise<{ default: Module } | Module>;

export interface ModuleEntry {
  manifest: ModuleManifest;
  entry: string;
}

function normalize(imported: { default: Module } | Module): Module {
  return "default" in imported && (imported as { default: Module }).default
    ? (imported as { default: Module }).default
    : (imported as Module);
}

export async function loadModules(opts: {
  entries: ModuleEntry[];
  importFn: ImportFn;
  registry: ModuleRegistry;
}): Promise<void> {
  for (const { manifest, entry } of opts.entries) {
    parseManifest(manifest);
    const module = normalize(await opts.importFn(entry));
    if (module.manifest.id !== manifest.id) {
      throw new Error(
        `module at ${entry} declares id ${module.manifest.id}, manifest says ${manifest.id}`,
      );
    }
    opts.registry.add(module);
  }
}
