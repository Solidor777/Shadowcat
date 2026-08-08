// CLI entry point for the documentation-exemption report. Carries NO `isMain`-style guard: the
// prior single-file version gated its whole body behind
// `resolve(process.argv[1]) === fileURLToPath(import.meta.url)`, an exact string comparison
// that goes silently to a no-op — printing nothing, exiting 0 — whenever `argv[1]` fails to
// normalize identically (a differently-cased Windows drive letter, a symlinked bin). Silence is
// then indistinguishable from "no exemptions", the exact failure this report exists to prevent.
// Splitting the reporting logic into the importable `report-doc-exemptions.mjs` module and
// running it unconditionally here removes the guard rather than hardening it — invoking this
// file (`node scripts/report-doc-exemptions-cli.mjs`) always executes the report; there is no
// comparison left to fail.
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
