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
 * Every bucket a code span can end up in, as the label the banner prints for it, paired with the
 * count field it reads. This array is the ONE enumeration: the banner is generated from it, and a
 * test asserts that both prose statements of the rule list exactly these labels. A sentence
 * claiming "every span lands in exactly one printed bucket" is a claim ABOUT this list, and a
 * claim about code that is neither derived from it nor tested against it drifts the first time a
 * bucket is added.
 */
export const SPAN_BUCKETS = [
  { key: "verified", label: "verified" },
  { key: "acknowledged", label: "acknowledged non-symbol" },
  { key: "crossRepo", label: "cross-repo" },
  { key: "broken", label: "broken" },
  { key: "exampleExempt", label: "EXAMPLE-exempt" },
  { key: "nonCandidates", label: "not citation-shaped" },
  { key: "emptySpans", label: "empty" },
];

/**
 * Every way a backtick RUN leaves the pipeline without ever becoming a span: blanked with the code
 * block around it, or paired with nothing at one of two levels. All are printed for the same reason
 * every bucket is — block stripping has the largest blast radius of any exclusion here, so a fence
 * misdetection must move a number someone can see. Like `SPAN_BUCKETS`, this array is the ONE
 * enumeration: the banner is generated from it and a test asserts both prose statements list
 * exactly these labels, in this order.
 *
 * The two unpaired levels are split because they mean opposite things. A run left unpaired INSIDE a
 * span is ordinary — a longer-run span exists to quote a shorter delimiter — while one left
 * unpaired at TOP level shifts pairing across its whole paragraph, so the real citations land in
 * the gaps between spans and the prose between them becomes the spans. Merged into one count, the
 * fatal case cannot be told from the ordinary one, and the file-level floor stays silent whenever
 * any other paragraph in the file still yields a checked citation.
 */
export const RUN_EXCLUSIONS = [
  { key: "blankedRuns", label: "block-blanked" },
  { key: "nestedUnpairedRuns", label: "unpaired inside a span" },
  { key: "topLevelUnpairedRuns", label: "unpaired at top level" },
];

/**
 * Turns one gate result into the exact output and exit code the CLI produces, without touching
 * the console or the process.
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
    broken,
    filesWithNoCandidates,
    filesWithUnterminatedFence,
    acknowledgedHits,
    crossRepoHits,
    unusedAcknowledgements,
    untrackedDirs,
    indexedAcknowledgements,
    accounting,
    conservationDelta,
    conservationFailures,
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
      ? ` ${untrackedDirs.length} untracked skill directory(ies) excluded as not this repo's own ` +
        `prose (${untrackedDirs.join(", ")}).`
      : "";
  const namedEntries = { acknowledged: acknowledgedHits.size, crossRepo: crossRepoHits.size };
  const buckets = SPAN_BUCKETS.map(({ key, label }) => {
    const via = namedEntries[key] === undefined ? "" : ` (${namedEntries[key]} named entry(ies))`;
    return `${accounting[key]} ${label}${via}`;
  }).join(", ");
  const excluded = RUN_EXCLUSIONS.map(
    ({ key, label }) => `${accounting[key]} ${label}`,
  ).join(", ");
  const banner =
    `check-skill-symbol-refs: symbol index ${symbolCount} name(s) from ${filesIndexed} source file(s); ` +
    `${accounting.rawRuns} backtick run(s) in ${filesScanned} skill file(s) account for ` +
    `${accounting.spansEmitted} code span(s) — ${buckets}; ` +
    `run(s) leaving before classification: ${excluded} ` +
    `(from ${accounting.blankedBlockLines} code-block line(s)); conservation delta ` +
    `${conservationDelta}.${untracked}` +
    ` The ${accounting.crossRepo} cross-repo citation(s) name symbols verified absent from this ` +
    `tree and acknowledged as the separate Nightfox repository's, which nothing here can check — ` +
    `a standing review obligation (see RULE 15).`;

  // EVERY failure class is collected. Reporting only the first would let a run with a broken
  // citation hide every dead acknowledgement entry behind it, so the two could only ever be fixed
  // one run at a time.
  const problems = [];

  // The conservation invariant, first and loudest. Every widening of this gate's reach creates a
  // new early-exit path, so the identity is what makes an uncounted path impossible rather than
  // what makes the next instance findable.
  if (conservationDelta !== 0 || conservationFailures.length > 0)
    problems.push(
      `\nspan conservation FAILED by ${conservationDelta} backtick run(s) in aggregate, across ` +
        `${conservationFailures.length} file(s):\n` +
        conservationFailures
          .map(({ file, delta }) => `  ${file}  delta ${delta}`)
          .join("\n") +
        "\n\nEvery backtick run must be block-blanked, unpaired, or one of the two delimiters of a " +
        "span that reached exactly one bucket. A non-zero delta means spans are leaving the " +
        "pipeline uncounted — find the exclusion that does not declare itself.",
    );

  // A top-level unpaired run is always a prose defect, and it is the ONLY signal of the failure
  // paragraph bounding leaves behind: bounding caps the shift to one paragraph, so conservation
  // still balances (the shifted spans land in the not-citation-shaped bucket) and the file-level
  // floor stays silent as long as any other paragraph in the file still yields a checked citation.
  if (accounting.topLevelUnpairedRuns > 0)
    problems.push(
      `\n${accounting.topLevelUnpairedRuns} backtick run(s) paired with nothing at top level.\n\n` +
        "A stray delimiter shifts pairing across its whole paragraph: the real citations fall into " +
        "the gaps between spans and the prose between them becomes the spans, which leaves that " +
        "paragraph unchecked while every total stays healthy. Close or remove the delimiter.",
    );

  // An unclosed fence is a document defect with no terminator to recover on: every line after it
  // is blanked to end of file, so the document's remaining citations leave the gate while
  // conservation still balances and `bodyRuns` falls to 0, which is exactly the measurement the
  // per-file floor below reads. Failing on the fence itself closes that path and the mid-file
  // variant of it at once, and names the defect rather than a symptom of it.
  if (filesWithUnterminatedFence.length > 0)
    problems.push(
      `\n${filesWithUnterminatedFence.length} skill file(s) end inside an unclosed code fence:\n` +
        filesWithUnterminatedFence.map((f) => `  ${f}`).join("\n") +
        "\n\nEvery line after an unclosed fence is stripped as code, so the rest of the document " +
        "is not checked at all. Close the fence.",
    );

  // A file that carries backticks and yields no CHECKED citation has silently left the gate: one
  // unpaired delimiter is enough, and every total stays healthy because the other files carry
  // them. What the file's spans did land in is printed with it, so the finding states the file's
  // real situation instead of a claim that could be false about it.
  if (filesWithNoCandidates.length > 0)
    problems.push(
      `\n${filesWithNoCandidates.length} skill file(s) contain backticks but yielded no checked citation:\n` +
        filesWithNoCandidates
          .map(
            (f) =>
              `  ${f.file}  (${f.nonCandidates} not citation-shaped, ${f.exampleExempt} ` +
              `EXAMPLE-exempt, ${f.emptySpans} empty, ${f.unpairedRuns} unpaired run(s))`,
          )
          .join("\n") +
        "\n\nA file whose prose yields no checked citation is not being checked. Look " +
        "for an unpaired backtick run or a fence delimiter that stopped opening its line.",
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

  // An acknowledgement entry the INDEX also declares names the cause the zero-hit rule can only
  // ever name by its symptom. For a whole-token list the two overlap; for
  // `ACKNOWLEDGED_EXTERNAL_PREFIX` they do not, because that list matches on the head segment — a
  // prefix `P` that becomes indexed still leaves `P.unknownMember` failing resolution and bumping
  // `P`, so the entry stays alive while the index has quietly swallowed the name it acknowledges.
  if (indexedAcknowledgements.length > 0)
    problems.push(
      `\n${indexedAcknowledgements.length} acknowledgement entry(ies) are ALSO declared in the tree:\n` +
        indexedAcknowledgements.map((e) => `  ${e}`).join("\n") +
        "\n\nAn acknowledgement names something this tree does NOT declare. Either the index has " +
        "grown a name it should not carry, or the entry is no longer true.",
    );

  return { exitCode: problems.length > 0 ? 1 : 0, banner, problems };
}

/**
 * Writes one classified run to the given sinks and returns the process exit code. The exit itself
 * is the caller's single assignment, so no branch that decides FATALITY lives in an untested entry
 * point: deleting any part of this function changes a value a test asserts on, and the spawned-CLI
 * test pins the one remaining assignment end to end.
 *
 * @param {ReturnType<import("./check-skill-symbol-refs.mjs").checkSkillSymbolRefs>} result - one
 *   completed gate run.
 * @param {{log: (s: string) => void, error: (s: string) => void}} io - output sinks.
 * @returns {number} the exit code the process must carry.
 */
export function report(result, io) {
  const { exitCode, banner, problems } = classifySkillSymbolRun(result);
  if (banner === "") io.error(`check-skill-symbol-refs: ${problems.join("\n")}`);
  else {
    io.log(banner);
    for (const problem of problems) io.error(problem);
    if (exitCode === 0) io.log("0 broken symbol citations.");
  }
  return exitCode;
}

function main() {
  // The repository root defaults to this script's own parent; an explicit argument is the seam the
  // spawned-CLI test uses to point the gate at a fixture checkout with a seeded failure.
  const repoRoot =
    process.argv[2] === undefined
      ? resolve(dirname(fileURLToPath(import.meta.url)), "..")
      : resolve(process.argv[2]);
  process.exitCode = report(checkSkillSymbolRefs(repoRoot), console);
}

if (isDirectEntry(import.meta.url)) main();
