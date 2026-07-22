import { readFileSync, globSync } from "node:fs";
import { pathToFileURL } from "node:url";

const IMPORT_RE = /from\s+["'](svelte(?:\/[^"']+)?)["']/g;

export function findUnenumeratedSveltePaths(fileContentsByPath, knownEntries) {
  const flagged = [];
  for (const [file, content] of Object.entries(fileContentsByPath)) {
    for (const match of content.matchAll(IMPORT_RE)) {
      const specifier = match[1];
      if (!knownEntries.includes(specifier)) {
        flagged.push({ file, specifier });
      }
    }
  }
  return flagged;
}

// CLI entry point — only runs when invoked directly, not when imported by the test.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const { RUNTIME_ENTRIES } = await import("../src/client/shell/vite.config.ts");
  const knownEntries = Object.values(RUNTIME_ENTRIES);
  const files = globSync(["src/client/**/*.{ts,svelte}", "src/modules/**/*.{ts,svelte}"], {
    exclude: (path) => path.includes("node_modules"),
  });
  const contents = Object.fromEntries(files.map((f) => [f, readFileSync(f, "utf8")]));
  const flagged = findUnenumeratedSveltePaths(contents, knownEntries);
  if (flagged.length > 0) {
    console.error("Un-enumerated svelte/* imports found (add to RUNTIME_ENTRIES in src/client/shell/vite.config.ts):");
    for (const { file, specifier } of flagged) console.error(`  ${file}: ${specifier}`);
    process.exit(1);
  }
}
