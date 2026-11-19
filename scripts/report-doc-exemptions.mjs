// Derives the active TypeDoc documentation-exemption set from every `typedoc*.json` in the
// repo (root, base, and each package config) instead of naming one hardcoded path — an
// exemption added to a config this scan does not read would otherwise be invisible to the
// count it reports, which is the exact backdoor the exemption accountability rule exists to
// close. Pure library: no top-level side effects — the CLI entry point imports `scanDocExemptions`
// and runs it unconditionally, with no entry-point guard, because an `argv[1]`-vs-module-URL
// identity check goes silently to a no-op whenever the two spellings fail to normalize
// identically, and silence is indistinguishable from "no exemptions".
// Cross-platform: node:path/node:fs only.
import { readFileSync } from "node:fs";
import { basename, join } from "node:path";
import { repoFiles } from "./lib/gate-corpus.mjs";

/**
 * Finds every `typedoc*.json` file in the repository corpus (`repoFiles`: tracked plus untracked,
 * non-ignored files). This COUNTS exemptions, so an untracked config is included: an exemption in
 * a file git has not yet seen would otherwise be an uncounted one. Ignored build output never
 * counts.
 * @param {string} root - The repository root to scan from.
 * @param {(line: string) => void} [log] - Receives the corpus size line; silent when omitted,
 *   because this is a library function (see `repoFiles`).
 * @returns {string[]} Absolute paths, sorted for deterministic output.
 * @example
 * const configs = findTypedocConfigs(process.cwd());
 */
export function findTypedocConfigs(root, log = () => {}) {
  return repoFiles({ cwd: root, exts: [".json"], label: "report-doc-exemptions", log })
    .filter((p) => /^typedoc.*\.json$/.test(basename(p)))
    .map((p) => join(root, ...p.split("/")))
    .sort();
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
 * @param {(line: string) => void} [log] - Receives the corpus size line; silent when omitted.
 * @returns {{ total: number, scanned: string[], bySource: { path: string, names: string[] }[] }}
 *   `total` — the sum of every exemption found; `scanned` — every `typedoc*.json` path visited,
 *   including ones carrying zero exemptions; `bySource` — a per-source breakdown listing only
 *   the configs that actually carry at least one exemption.
 */
export function scanDocExemptions(repoRoot, log = () => {}) {
  const scanned = findTypedocConfigs(repoRoot, log);
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
