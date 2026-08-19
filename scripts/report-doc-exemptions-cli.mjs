// CLI entry point for the documentation-exemption report. Carries NO `isDirectEntry` guard:
// gating a CLI's whole body behind
// `resolve(process.argv[1]) === fileURLToPath(import.meta.url)` is an exact string comparison
// that goes silently to a no-op — printing nothing, exiting 0 — whenever `argv[1]` fails to
// normalize identically (a differently-cased Windows drive letter, a symlinked bin). Silence is
// then indistinguishable from "no exemptions", the exact failure this report exists to prevent.
// The reporting logic therefore lives in the importable `scanDocExemptions` module and runs
// unconditionally here — there is no comparison to fail.
// Cross-platform: node:path/node:fs only.
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { scanDocExemptions } from "./report-doc-exemptions.mjs";

const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
const { total, scanned, bySource } = scanDocExemptions(repo);
console.log(
  `typedoc: ${total} documentation exemption(s) active across ${scanned.length} typedoc*.json config(s) scanned`,
);
for (const { path, names } of bySource) {
  console.log(`  ${path}:`);
  for (const n of names) console.log(`    exempt: ${n}`);
}
