// CLI entry point. No `isDirectEntry` guard: an `argv[1]`-vs-module-URL identity check goes
// silently to a no-op — exiting 0 with no output — whenever the two spellings fail to normalize
// identically, so running this file always executes the check instead.
// Cross-platform: node:path/node:fs only.
import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import { findSkillFiles, checkSkillApiRefs } from "./check-skill-api-refs.mjs";

const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
const skillsRoot = resolve(repo, ".claude", "skills");
const distDocsRoot = resolve(repo, "dist-docs");

if (!existsSync(distDocsRoot)) {
  console.error(`check-skill-api-refs: missing ${distDocsRoot} — run the full pnpm docs:build chain first`);
  process.exit(1);
}

// A scope matching zero files is a broken SCOPE, not a clean pass — the skill family is never
// legitimately empty, so this only fires if the scan is pointed at the wrong directory.
const scannedFiles = findSkillFiles(skillsRoot);
if (scannedFiles.length === 0) {
  console.error(`check-skill-api-refs: 0 SKILL.md files found under ${skillsRoot}`);
  process.exit(2);
}

const { filesScanned, refsChecked, broken } = checkSkillApiRefs(skillsRoot, distDocsRoot);

// A zero-of-zero is indistinguishable from success unless it fails loudly: if the citation
// FORMAT in the skills changes (e.g. skills stop wrapping paths in backticks, or start citing
// `/docs/api/...` instead of `/api/...`), the extraction regex silently stops matching anything
// and a naive "0 broken" report reads as a clean pass. `broken.length === 0` alone can never
// distinguish "every citation verified" from "no citation was ever extracted".
if (refsChecked === 0) {
  console.error(
    `check-skill-api-refs: 0 /api/... references extracted from ${filesScanned} skill file(s) — the extraction pattern likely stopped matching`,
  );
  process.exit(2);
}

if (broken.length > 0) {
  for (const { file, ref } of broken) console.error(`broken doc pointer: ${file} -> ${ref}`);
  process.exit(1);
}

console.log(
  `check-skill-api-refs: ${refsChecked} doc pointer(s) verified across ${filesScanned} skill file(s), 0 broken`,
);
