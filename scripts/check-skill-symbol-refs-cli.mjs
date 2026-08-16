// CLI entry point. Guarded by `isDirectEntry` — the shared entry-point check
// (`scripts/lib/is-main.mjs`) rather than a hand-rolled `argv[1]`-vs-module-URL comparison, whose
// silent-no-op failure mode is exactly what this repo's doc gates cannot tolerate: every doc gate
// here is fatal, so a guard that goes quiet is the worst available shape.
// Cross-platform: node:path/node:fs only.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";
import { checkSkillSymbolRefs } from "./check-skill-symbol-refs.mjs";

function main() {
  const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const skillsRoot = resolve(repoRoot, ".claude", "skills");

  const {
    filesScanned,
    filesIndexed,
    symbolCount,
    candidatesChecked,
    verified,
    acknowledged,
    broken,
    ambiguous,
    nonCandidates,
    nightfoxExcludedFiles,
    nightfoxExcludedBroken,
  } = checkSkillSymbolRefs(skillsRoot, repoRoot);

  // A scan that touched no skill file, or extracted no citation-shaped candidate at all, is
  // indistinguishable from a clean pass by its own zero-broken count — exactly the "0 broken"
  // shape `check-skill-api-refs-cli.mjs` already refuses to treat as success. Fail loudly instead
  // of reporting a green run that verified nothing.
  if (filesScanned === 0) {
    console.error(`check-skill-symbol-refs: 0 skill .md file(s) found under ${skillsRoot}`);
    process.exit(2);
  }
  if (symbolCount === 0) {
    console.error(
      `check-skill-symbol-refs: symbol index is empty after scanning ${filesIndexed} source file(s) — the extraction pattern likely stopped matching`,
    );
    process.exit(2);
  }
  if (candidatesChecked === 0) {
    console.error(
      `check-skill-symbol-refs: 0 citation-shaped candidate(s) extracted from ${filesScanned} skill file(s) — the extraction pattern likely stopped matching`,
    );
    process.exit(2);
  }

  console.log(
    `check-skill-symbol-refs: symbol index ${symbolCount} name(s) from ${filesIndexed} source file(s); ` +
      `${candidatesChecked} citation-shaped candidate(s) in ${filesScanned} skill file(s) ` +
      `(${verified} verified, ${acknowledged} acknowledged non-symbol, ${broken.length} broken); ` +
      `${ambiguous} flat bare-lowercase/camelCase/wire-path token(s) excluded from resolution ` +
      `(review obligation, see RULE 15); ${nonCandidates} backtick span(s) not citation-shaped; ` +
      `${nightfoxExcludedFiles} Nightfox skill file(s) excluded from this gate (cross-repo, ` +
      `structurally unresolvable — review obligation, see RULE 15), carrying ` +
      `${nightfoxExcludedBroken} unresolved candidate(s).`,
  );

  if (broken.length > 0) {
    console.error(`\n${broken.length} broken symbol citation(s):`);
    for (const { file, line, token } of broken)
      console.error(`  ${file}:${line}  \`${token}\` — no matching symbol in the tree`);
    process.exit(1);
  }

  console.log("0 broken symbol citations.");
}

if (isDirectEntry(import.meta.url)) main();
