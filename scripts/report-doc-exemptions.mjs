// Derives the active TypeDoc documentation-exemption set from every `typedoc*.json` in the
// repo (root, base, and each package config) instead of naming one hardcoded path — an
// exemption added to a config this scan does not read would otherwise be invisible to the
// count it reports, which is the exact backdoor the exemption accountability rule exists to
// close. Pure library: no top-level side effects. `report-doc-exemptions-cli.mjs` is the
// executable entry point that imports and runs this unconditionally (see that file's header
// for why the CLI carries no `isMain`-style guard).
// Cross-platform: node:path/node:fs only.
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

// Build/vendor subtrees a `typedoc*.json` never legitimately lives under; also keeps the walk
// from descending into node_modules (which can itself contain files matching the name pattern).
const IGNORED_DIRS = new Set([
  "node_modules",
  ".git",
  "dist",
  "dist-docs",
  "target",
  ".docs-tmp",
  "coverage",
]);

/**
 * Recursively finds every `typedoc*.json` file under `root`, skipping build/vendor directories.
 * @param {string} root - Directory to scan from.
 * @returns {string[]} Absolute paths, sorted for deterministic output.
 */
export function findTypedocConfigs(root) {
  const out = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (entry.isDirectory()) {
        if (IGNORED_DIRS.has(entry.name)) continue;
        walk(join(dir, entry.name));
      } else if (entry.isFile() && /^typedoc.*\.json$/.test(entry.name)) {
        out.push(join(dir, entry.name));
      }
    }
  };
  walk(root);
  return out.sort();
}

/**
 * Reads the enumerated documentation exemptions off a parsed TypeDoc config.
 * @param {{ intentionallyNotDocumented?: string[] }} config - a parsed TypeDoc config object.
 * @returns {{ count: number, names: string[] }} the number of exempted reflection names and
 *   the names themselves, in the order they appear in the config.
 */
export function reportDocExemptions(config) {
  const names = config.intentionallyNotDocumented ?? [];
  return { count: names.length, names };
}

/**
 * Scans every `typedoc*.json` under `repoRoot` for `intentionallyNotDocumented` entries,
 * stamping the result with what was scanned rather than returning a bare count.
 * @param {string} repoRoot - The repository root to scan from.
 * @returns {{ total: number, scanned: string[], bySource: { path: string, names: string[] }[] }}
 *   `total` — the sum of every exemption found; `scanned` — every `typedoc*.json` path visited,
 *   including ones carrying zero exemptions; `bySource` — a per-source breakdown listing only
 *   the configs that actually carry at least one exemption.
 */
export function scanDocExemptions(repoRoot) {
  const scanned = findTypedocConfigs(repoRoot);
  const bySource = [];
  let total = 0;
  for (const path of scanned) {
    const config = JSON.parse(readFileSync(path, "utf8"));
    const { count, names } = reportDocExemptions(config);
    if (count > 0) {
      bySource.push({ path, names });
      total += count;
    }
  }
  return { total, scanned, bySource };
}
