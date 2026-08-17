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

/**
 * Turns one gate result into the exact output and exit code the CLI produces, without touching
 * the console or the process. Keeping this pure is what makes the FATALITY testable: deleting an
 * exit branch changes a returned number a test asserts on, where a `process.exit` buried in an
 * I/O function can only be observed by spawning a child process.
 *
 * @param {ReturnType<import("./check-skill-symbol-refs.mjs").checkSkillSymbolRefs>} result - one
 *   completed gate run.
 * @returns {{ exitCode: number, banner: string, problems: string[] }} `exitCode` 0 clean, 1 a real
 *   finding, 2 an instrument failure (nothing was measured).
 */
export function classifySkillSymbolRun(result) {
  const {
    filesScanned,
    filesIndexed,
    symbolCount,
    candidatesChecked,
    verified,
    acknowledged,
    broken,
    nonCandidates,
    exampleExempt,
    crossRepo,
    filesWithNoCandidates,
    acknowledgedHits,
    crossRepoHits,
    unusedAcknowledgements,
    untrackedDirs,
  } = result;

  // A scan that touched no skill file, or extracted no citation-shaped candidate at all, is
  // indistinguishable from a clean pass by its own zero-broken count — exactly the "0 broken"
  // shape `check-skill-api-refs-cli.mjs` already refuses to treat as success. Fail loudly instead
  // of reporting a green run that verified nothing.
  const instrumentFailure = (message) => ({ exitCode: 2, banner: "", problems: [message] });
  if (filesScanned === 0) return instrumentFailure("0 tracked skill .md file(s) found");
  if (symbolCount === 0)
    return instrumentFailure(
      `symbol index is empty after scanning ${filesIndexed} source file(s) — the extraction pattern likely stopped matching`,
    );
  if (candidatesChecked === 0)
    return instrumentFailure(
      `0 citation-shaped candidate(s) extracted from ${filesScanned} skill file(s) — the extraction pattern likely stopped matching`,
    );

  const untracked =
    untrackedDirs.length > 0
      ? `${untrackedDirs.length} untracked skill directory(ies) excluded as not this repo's own ` +
        `prose (${untrackedDirs.join(", ")}); `
      : "";
  const banner =
    `check-skill-symbol-refs: symbol index ${symbolCount} name(s) from ${filesIndexed} source file(s); ` +
    `${candidatesChecked} citation-shaped candidate(s) in ${filesScanned} skill file(s) ` +
    `(${verified} verified, ${acknowledged} acknowledged non-symbol via ${acknowledgedHits.size} ` +
    `named entry(ies), ${crossRepo} cross-repo via ${crossRepoHits.size} named entry(ies), ` +
    `${broken.length} broken); ` +
    `${nonCandidates} code span(s) not citation-shaped; ` +
    `${exampleExempt} code span(s) EXAMPLE-exempt; ${untracked}` +
    `the ${crossRepo} cross-repo citation(s) name symbols the separate Nightfox repository ` +
    `declares — not present in this checkout, a standing review obligation (see RULE 15).`;

  // EVERY failure class is collected. Reporting only the first would let a run with a broken
  // citation hide every dead acknowledgement entry behind it, so the two could only ever be fixed
  // one round at a time.
  const problems = [];

  // A file that carries backticks and yields no classified span at all has silently left the gate:
  // one unpaired delimiter is enough, and every total stays healthy because the other files carry
  // them. This floor is what turns that class from silent into loud.
  if (filesWithNoCandidates.length > 0)
    problems.push(
      `\n${filesWithNoCandidates.length} skill file(s) contain backticks but yielded no classified span:\n` +
        filesWithNoCandidates.map((f) => `  ${f}`).join("\n") +
        "\n\nA file whose prose no longer produces spans is not being checked. Look for an " +
        "unpaired backtick run or a fence delimiter that stopped opening its line.",
    );

  if (broken.length > 0)
    problems.push(
      `\n${broken.length} broken symbol citation(s):\n` +
        broken
          .map(
            ({ file, line, token }) =>
              `  ${file}:${line}  \`${token}\` — no matching symbol in the tree`,
          )
          .join("\n"),
    );

  // An acknowledgement entry that absorbs nothing is a standing invitation to absorb a future
  // defect: the day a real, broken citation happens to spell that token, the gate reports it as a
  // known non-symbol. The list is therefore re-derived from what the corpus actually reaches on
  // every run, not maintained by hand and trusted.
  if (unusedAcknowledgements.length > 0)
    problems.push(
      `\n${unusedAcknowledgements.length} acknowledgement entry(ies) matched nothing in the corpus:\n` +
        unusedAcknowledgements.map((e) => `  ${e}`).join("\n") +
        "\n\nDelete each one. An entry no citation reaches cannot be justified by the corpus, and " +
        "it silently absorbs the first future citation that happens to spell it.",
    );

  return { exitCode: problems.length > 0 ? 1 : 0, banner, problems };
}

function main() {
  const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const { exitCode, banner, problems } = classifySkillSymbolRun(checkSkillSymbolRefs(repoRoot));
  if (banner === "") console.error(`check-skill-symbol-refs: ${problems.join("\n")}`);
  else {
    console.log(banner);
    for (const problem of problems) console.error(problem);
  }
  if (exitCode !== 0) process.exit(exitCode);
  console.log("0 broken symbol citations.");
}

if (isDirectEntry(import.meta.url)) main();
