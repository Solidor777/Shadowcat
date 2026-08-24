import { readFileSync } from "node:fs";
import { test, expect } from "vitest";
import {
  scanContent,
  scanCandidates,
  collectFiles,
  gateFileSet,
  residueReport,
  unusedAcknowledgements,
  separatorFlexible,
  separatorOnlyClass,
  bannedMatchesIn,
  assertNoGlobalPatterns,
  instrumentComponents,
  instrumentFingerprint,
  instrumentHashInput,
  BANNED,
  SKILL_BANNED,
} from "./check-comment-refs.mjs";
import { GENERATED_ROOT, MD_EXTS, defaultSkillsRoot, norm } from "./lib/gate-corpus.mjs";
import { existsSync } from "node:fs";

// The skill corpus is a standalone plugin checkout since the shadowcat-codebase migration, not
// part of this repo — CI never has it, so every test below that depends on the REAL skill corpus
// (as opposed to a fixture it seeds itself) is skipped when no checkout is present locally.
const SKILLS_ROOT = defaultSkillsRoot();
const HAS_SKILLS_CHECKOUT = existsSync(SKILLS_ROOT);

// Every fixture string below is a SPECIMEN whose exact wording is the thing under test — an id, a
// date, a filename these fixtures must reproduce verbatim to prove the pattern matches it. Each such
// LINE carries the scanner's own EXAMPLE: marker (the exemption is per-line, so it rides on the
// same line as the fixture, never a comment above it), the same exemption BANNED's own entries
// use to describe themselves without becoming a hit.

// Code-file mode (isMd: false) — unchanged behaviour, re-verified here so a future edit to the
// shared BANNED list is caught by the same suite that guards the skill subset.
test("code mode flags a milestone id in a line comment", () => {
  const { hits } = scanContent("// Kept minimal for M8c-1.\n", { isMd: false }); // EXAMPLE:
  expect(hits.map((h) => h.kind)).toEqual(["milestone/task id"]);
});

test("code mode does not flag ordinary commentary", () => {
  const { hits } = scanContent("// Uses a Set for O(1) lookups.\n", {
    isMd: false,
  });
  expect(hits).toEqual([]);
});

test("code mode EXAMPLE: marker exempts a specimen line from every pattern", () => {
  const { hits, exempted } = scanContent(
    "// EXAMPLE: `M13-0` in a doctest string.\n",
    {
      isMd: false,
    },
  );
  expect(hits).toEqual([]);
  expect(exempted).toBe(1);
});

// Skill-file mode (isMd: true) — the ruling's narrowed subset.
test("skill mode flags a milestone id", () => {
  const fixture = "Ships in M13-0 as a three-band envelope.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["milestone/task id"]);
});

test("skill mode flags a task/phase id", () => {
  const fixture = "Fixed in D9 alongside the traversal gate.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual([
    "phase / workstream / invariant id",
  ]);
});

// The separated writers name the same artifact as the unspaced one, and neither hits a live line
// today — so the only evidence they are reached at all is this control. Revert direction: drop
// either alternative from the entry and its row here turns green-to-red.
test("skill mode flags a task/phase id written with a hyphen or an underscore", () => {
  for (const fixture of ["Fixed in D-9 today.\n", "Landed under W_2.\n"]) { // EXAMPLE:
    const { hits } = scanContent(fixture, { isMd: true });
    expect(hits.map((h) => h.kind)).toEqual([
      "phase / workstream / invariant id",
    ]);
  }
});

// The counter-case that decides the spelling. A one-member class would be widened to the SPACE
// writer, where a single capital plus a space plus a quantity is ordinary English rather than an
// id; the alternatives above are written so the widening cannot reach them. Revert direction:
// respell the separators as `[DITW][-_]?\d+` and this line starts failing.
test("skill mode does NOT read a capital, a space and a quantity as a phase id", () => {
  const fixture = "Each T 4 cells wide, and I 2 rows deep.\n";
  expect(scanContent(fixture, { isMd: true }).hits).toEqual([]);
});

test("skill mode flags a sweep/round/review marker", () => {
  const fixture = "Docs-ratchet is live here (docs sweep 2b).\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["sweep / round / review marker"]);
});

// Two rules govern this one span, and both are reported: a group carrying two violations must not
// be fixable one gate run at a time.
test("skill mode flags a dated plan filename under every rule it breaks", () => {
  const fixture =
    "Plan: `docs/superpowers/plans/2026-07-15-m13a-formula-library.md`.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["dated plan/spec file", "date stamp"]);
});

// A subject is a GROUP, so a wrapped phrase is attributed to the line it STARTS on - a line whose
// text does not contain the phrase. The matched words travel with the finding for that reason.
test("a hit carries the matched text alongside the source line", () => {
  const fixture = "// the comparator is evaluated once, unlike before the\n// fix it replaces\n"; // EXAMPLE:
  const [hit] = scanContent(fixture, { isMd: false }).hits;
  expect(hit.kind).toBe("history narration");
  expect(hit.text).not.toContain(hit.match);
  expect(hit.match).toContain("before the");
  expect(hit.match).toContain("fix");
});

test("skill mode flags a bare date narrative form the code pattern would miss", () => {
  const fixture =
    "No data migrations pre-customers (user directive 2026-07-30).\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["date stamp"]);
});

// Positive control for the standing carve-out: a durable document cited by path + section anchor
// must pass, even though "repo document pointer" would flag the same shape in code.
test("skill mode does NOT flag a durable path + section anchor citation", () => {
  const fixture =
    "Rationale: `docs/design/ARCHITECTURE.md` §2 (invariants 3, 5, 6).\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits).toEqual([]);
});

test("skill mode does NOT flag a durable design-doc citation naming a rule by heading", () => {
  const fixture =
    "Full rule: `docs/design/doc-sweep-truthfulness-rules.md` RULE 16.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits).toEqual([]);
});

// Negative control, restated from the ruling in its own terms: a milestone id in a skill must
// fail even when it rides alongside an otherwise-durable citation on the same line.
test("skill mode flags an m-id even beside a durable citation on the same line", () => {
  const fixture =
    "See `docs/design/ARCHITECTURE.md` §2 for the M10e-4 exception.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["milestone/task id"]);
});

test("skill mode EXAMPLE: marker still exempts a specimen line", () => {
  const { hits, exempted } = scanContent(
    "- EXAMPLE: `M13-0`, `Task 14j`, `D9`.\n",
    {
      isMd: true,
    },
  );
  expect(hits).toEqual([]);
  expect(exempted).toBe(1);
});

// Three additional categories the owner ruled in for skills, on top of the marker subset above.
test("skill mode flags an ephemeral doc pointer", () => {
  const fixture = "Deferred work is tracked in `docs/TODO.md`.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["ephemeral doc pointer"]);
});

test("skill mode flags every named churn tracker", () => {
  for (const file of [
    "TODO.md",
    "PLAN.md",
    "OPEN_BUGS.md",
    "CLOSED_BUGS.md",
    "POST_WORK_FINDINGS.md",
  ]) {
    const { hits } = scanContent(`See \`docs/${file}\` for the backlog.\n`, {
      isMd: true,
    });
    expect(hits.map((h) => h.kind)).toEqual(["ephemeral doc pointer"]);
  }
});

test("skill mode does NOT flag a durable architecture doc as an ephemeral doc pointer", () => {
  const fixture = "See `docs/design/ARCHITECTURE.md` for the invariant list.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits).toEqual([]);
});

test("skill mode flags history narration", () => {
  const fixture = "Previously this returned a bare list; now it returns a map.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["history narration"]);
});

test("skill mode does NOT flag `no longer` as history narration", () => {
  const fixture = "A revoked token is no longer present in the recipient mask.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits).toEqual([]);
});

// Positive control: the allusion construction itself, in a plain comment. Carries no word the
// entry above ("history narration") anchors on, so this fails ONLY under the new entry — proving
// it is a real addition, not a restatement of an existing hit.
test("code mode flags a definite reference to an incident by allusion", () => {
  const fixture = "// Regression test for the reported panic in the move handler.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: false });
  expect(hits.map((h) => h.kind)).toEqual(["history narration by allusion"]);
});

// Positive control for the plural noun form: "bugs" carries a trailing word character after the
// singular stem, so a boundary assertion placed right after the stem alone would miss it.
test("code mode flags the allusion construction with a plural incident noun", () => {
  const fixture = "// A regression suite for the documented bugs in the retry path.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: false });
  expect(hits.map((h) => h.kind)).toEqual(["history narration by allusion"]);
});

// Reaches CODE-FACING STRINGS, not just comments — the logged instance was a test name.
test("code mode flags a definite reference to an incident by allusion inside a test name", () => {
  const fixture =
    'it("does not repeat the observed crash when the queue drains twice", () => {});\n'; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: false });
  expect(hits.map((h) => h.kind)).toEqual(["history narration by allusion"]);
});

// Negative controls for the collision this pattern is designed against: the same nouns naming a
// CODE CONSTRUCT rather than an event. None carries a reporting participle between the determiner
// and the noun, and the four here are the exact examples the collision was defined against.
test("code mode does NOT flag a code-construct name sharing the incident-noun vocabulary", () => {
  for (const fixture of [
    "// Walks the panic path before returning the fallback value.\n", // EXAMPLE:
    "// The crash handler restores the last committed frame.\n", // EXAMPLE:
    "// This failure mode is covered by the retry loop above.\n", // EXAMPLE:
    "// An error type distinguishes a parse failure from a resource cap.\n", // EXAMPLE:
  ]) {
    const { hits } = scanContent(fixture, { isMd: false });
    expect(hits.map((h) => h.kind)).not.toContain("history narration by allusion");
  }
});

// A reporting participle in front of a construct-noun compound is the harder collision: the
// EXAMPLE: participle alone would otherwise fire on "the observed failure" inside "the observed
// EXAMPLE: failure mode", which describes the mode's observability, not a reported incident.
test("code mode does NOT flag a reporting participle in front of a construct-noun compound", () => {
  const fixture = "// The observed failure mode matches the one predicted by the model.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: false });
  expect(hits.map((h) => h.kind)).not.toContain("history narration by allusion");
});

test("skill mode flags a definite reference to an incident by allusion", () => {
  const fixture = "Guards against the documented deadlock in the write queue.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["history narration by allusion"]);
});

test("skill mode flags an unnamed spec reference", () => {
  const fixture = "Field ordering follows the spec'd default.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["unnamed spec reference"]);
});

// Instrument defect A: a milestone id inside a durable design-doc FILENAME must not fire, but a
// bare milestone id elsewhere on the same line still must.
test("skill mode does NOT flag a milestone id embedded in a design-doc filename", () => {
  const fixture =
    "Rationale in `docs/design/M2-data-foundation.md` covers the schema.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits).toEqual([]);
});

test("skill mode still flags a bare milestone id beside a design-doc filename citation", () => {
  const fixture =
    "M9 changed the shape now described in `docs/design/M2-data-foundation.md`.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["milestone/task id"]);
});

// Instrument defect B: the sweep marker must catch the plural form too.
test("skill mode flags a plural sweep marker", () => {
  const fixture = "Ephemeral refs were burned down across sweeps 2a+2b.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["sweep / round / review marker"]);
});

// Code-file mode must be unaffected by the skill-only categories and carve-out.
test("code mode does not gain the ephemeral-doc-pointer category as a distinct kind", () => {
  const { hits } = scanContent("// See docs/TODO.md for the backlog.\n", { // EXAMPLE:
    isMd: false,
  });
  expect(hits.map((h) => h.kind)).toEqual(["repo document pointer"]);
});

test("code mode still flags the singular sweep marker unchanged", () => {
  const { hits } = scanContent("// Cleaned up in sweep 12.\n", { // EXAMPLE:
    isMd: false,
  });
  expect(hits.map((h) => h.kind)).toEqual(["sweep / round / review marker"]);
});

// Spelled-out task-id form (the corpus's coverage gap this pattern update closes) — both file
// classes, since the entry is shared by reference and must not diverge between them.
test("code mode flags a spelled-out task id", () => {
  const { hits } = scanContent("// Fixed a defect Task 14j introduced.\n", { // EXAMPLE:
    isMd: false,
  });
  expect(hits.map((h) => h.kind)).toEqual(["milestone/task id"]);
});

test("skill mode flags a spelled-out task id", () => {
  const fixture = "The hex audit landed in Task 14e-8 and generalized the gate.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["milestone/task id"]);
});

// Hyphenated sweep-marker form — the same coverage-scan pass surfaced this second gap in the
// already-shared sweep entry.
test("skill mode flags a hyphenated sweep marker", () => {
  const fixture = "Caught by review (Sweep-1 lesson): the doc claim was wrong.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["sweep / round / review marker"]);
});

// The local-marker categories are shared by both file classes: an initial-plus-number naming a
// review finding or a numbered fix pass points outside the code in a comment exactly as it does in
// skill prose, so these cases fail together if either class ever stops carrying the entry.
test("skill mode flags a bare local letter+digit marker", () => {
  const fixture = "documented as the C1 no-pool-query-on-the-hot-path property.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["local letter+digit marker"]);
});

test("code mode flags a bare local letter+digit marker in a comment", () => {
  const { hits } = scanContent("// the C1 no-pool-query-on-the-hot-path property.\n", { // EXAMPLE:
    isMd: false,
  });
  expect(hits.map((h) => h.kind)).toEqual(["local letter+digit marker"]);
});

// The hyphenated writer is the same id as the bare one, so requiring the unhyphenated form alone
// would read a genuine marker as clean.
test("code mode flags the hyphenated local letter+digit marker", () => {
  const { hits } = scanContent("// reachable via the C-2 frame's audience field.\n", { // EXAMPLE:
    isMd: false,
  });
  expect(hits.map((h) => h.kind)).toEqual(["local letter+digit marker"]);
});

// A version label written as an initial plus its number is the same unresolvable local marker as a
// review finding's, so `V` sits in the same letter set. The two cases below pin the letter and the
// word boundary that keeps a versioned SYMBOL out of the ban.
test("code mode flags a version-label local marker", () => {
  const { hits } = scanContent("// V1 desaturate approximation of the wash.\n", { // EXAMPLE:
    isMd: false,
  });
  expect(hits.map((h) => h.kind)).toEqual(["local letter+digit marker"]);
});

test("a versioned symbol name is not a version-label marker", () => {
  const { hits } = scanContent("// `PanelLayoutV1` is the persisted shape; `Vec2` is its unit.\n", {
    isMd: false,
  });
  expect(hits).toEqual([]);
});

// The scope of the entry is the COMMENT and prose-shaped literals, never program data: a
// whitespace-free literal outside an explanatory context is a value the program acts on, where a
// marker-shaped collision refers to nothing. This is what lets the entry govern code at all.
test("a token-shaped string literal is program data, not a local marker", () => {
  const { hits } = scanContent('const label = "C1"; // a real identifier, not prose\n', { // EXAMPLE:
    isMd: false,
  });
  expect(hits).toEqual([]);
});

// A numbered severity word and a numbered remedy are the marker vocabulary a reviewer writes; the
// entry already carrying the sweep/fix-pass/finding forms is where they belong, not a new category.
test("code mode flags a numbered severity marker", () => {
  const { hits } = scanContent("// Critical 2 regression: the guard must stay.\n", { // EXAMPLE:
    isMd: false,
  });
  expect(hits.map((h) => h.kind)).toEqual(["sweep / round / review marker"]);
});

test("code mode flags a numbered fix-pass marker", () => {
  const { hits } = scanContent("// FIX 2: an inline roll still stores kind Normal.\n", { // EXAMPLE:
    isMd: false,
  });
  expect(hits.map((h) => h.kind)).toEqual(["sweep / round / review marker"]);
});

// The bare severity word is untouched: it is the NUMBER that makes the phrase name a finding
// somebody filed rather than describe the code. A category that flagged the word alone would fire
// on a roll tier label and on any doc comment calling a defect critical.
test("an unnumbered severity word is not a marker", () => {
  const { hits } = scanContent("// A critical failure here fails closed.\n", {
    isMd: false,
  });
  expect(hits).toEqual([]);
});

test("skill mode flags a numbered constraint reference", () => {
  const fixture = "requires exactly one runtime instance (Global Constraint 1).\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["numbered constraint"]);
});

// A numbered section of a plan document is the same unresolvable pointer in a code comment as in
// a skill, so the entry is shared by reference rather than duplicated: these two cases fail
// together if either file class ever stops carrying it.
test("code mode flags a numbered constraint reference in a comment", () => {
  const { hits } = scanContent("// Global Constraint 1: one runtime instance.\n", { // EXAMPLE:
    isMd: false,
  });
  expect(hits.map((h) => h.kind)).toEqual(["numbered constraint"]);
});

test("code mode flags a numbered constraint reference in an explanatory string", () => {
  const { hits } = scanContent('it("Constraint 1: single runtime instance", () => {});\n', { // EXAMPLE:
    isMd: false,
  });
  expect(hits.map((h) => h.kind)).toEqual(["numbered constraint"]);
});

// Coverage control (`scanCandidates`): a genuine miss, an acknowledged legitimate token, and a
// BANNED-shadowed candidate must resolve to exactly the right bucket.
test("scanCandidates reports a genuinely novel shape as residue", () => {
  const { residue, acknowledged } = scanCandidates(
    "Fixed in Sprint 4 without a regression.\n", // EXAMPLE:
  );
  expect(residue.map((r) => r.token)).toEqual(["Sprint 4"]); // EXAMPLE:
  expect(acknowledged).toEqual([]);
});

test("scanCandidates does not re-report a token an existing BANNED pattern already catches", () => {
  const { residue } = scanCandidates("Landed in Task 6, a real bug.\n"); // EXAMPLE:
  expect(residue).toEqual([]);
});

test("scanCandidates names and counts a legitimate acknowledged token", () => {
  const { acknowledged, residue } = scanCandidates(
    "Uses Svelte 5 (Runes) for the client shell.\n",
  );
  expect(residue).toEqual([]);
  expect(acknowledged).toEqual([
    {
      line: 1,
      token: "Svelte 5",
      reason: "product, protocol or algorithm name carrying a version-like number",
    },
  ]);
});

// The collision the coverage control found live: `WORD_NARRATION_TOKEN`'s six banned words are
// also PascalCase enum-variant names, cited inline in backticks as wire-protocol values rather
// than narration of the code's own past.
test("scanCandidates acknowledges a PascalCase enum-variant word cited inline in backticks", () => {
  const { residue, acknowledged } = scanCandidates(
    "REQUIRED for both ops: the bumped, authoritative value for `Replaced`, and the version the row\n",
  );
  expect(residue).toEqual([]);
  expect(acknowledged).toContainEqual({
    line: 1,
    token: "Replaced",
    reason:
      "an enum-variant name (PascalCase) cited inline in backticks as a wire-protocol/code value, not narration of the code's own past",
  });
});

test("scanCandidates acknowledges a second PascalCase enum-variant word cited inline in backticks", () => {
  const { residue, acknowledged } = scanCandidates(
    "the wire type carries a `Renamed` variant for this transition.\n",
  );
  expect(residue).toEqual([]);
  expect(
    acknowledged.some(
      (a) =>
        a.token === "Renamed" &&
        a.reason ===
          "an enum-variant name (PascalCase) cited inline in backticks as a wire-protocol/code value, not narration of the code's own past",
    ),
  ).toBe(true);
});

// Negative control: lowercase prose narration, not backtick-quoted, must still flag as residue —
// the new entry must not accidentally widen coverage to ordinary narration.
test("scanCandidates still flags lowercase prose narration with no backtick as residue", () => {
  const { residue } = scanCandidates("this endpoint was replaced last cycle.\n");
  expect(residue.map((r) => r.token)).toEqual(["replaced"]);
});

// Negative control, explicit decision: the same word backtick-quoted but LOWERCASE (a literal
// string value, not a PascalCase enum variant) is NOT acknowledged and still flags as residue —
// case sensitivity is what keeps the exemption narrow to the code-symbol form.
test("scanCandidates still flags a lowercase word cited in backticks as residue", () => {
  const { residue } = scanCandidates("the field stores the literal string `replaced` on disk.\n");
  expect(residue.map((r) => r.token)).toEqual(["replaced"]);
});

test("scanCandidates respects the EXAMPLE: exemption", () => {
  const { residue, acknowledged, exempted } = scanCandidates(
    "- EXAMPLE: `Sprint 4` is not a real reference.\n",
  );
  expect(residue).toEqual([]);
  expect(acknowledged).toEqual([]);
  expect(exempted).toBe(1);
});

// scanCandidates in code-file mode: comment/string-literal extraction and shadowing against
// BANNED (not SKILL_BANNED) must mirror scanContent's own code-mode behaviour exactly.
test("scanCandidates in code mode reads only the comment span, not program identifiers", () => {
  const { residue } = scanCandidates('const Vec2 = 4; // ordinary code, not prose\n', {
    isMd: false,
  });
  expect(residue).toEqual([]);
});

test("scanCandidates in code mode reports a novel unspaced shape found in a comment", () => {
  // The spaced "Word Number" candidate sub-form is skill-only (see CANDIDATE_TOKEN_WORD's own
  // comment), so a code-mode fixture must use the unspaced label form to exercise this path.
  const { residue } = scanCandidates("// Fixed the Bump7 regression.\n", { // EXAMPLE:
    isMd: false,
  });
  expect(residue.map((r) => r.token)).toEqual(["Bump7"]); // EXAMPLE:
});

test("scanCandidates in code mode shadows a candidate the ban list already catches", () => {
  // A candidate the gate itself would fail is not a coverage gap, so it must not be re-reported as
  // an unrecognised shape.
  const { residue } = scanCandidates("// documented as the C1 property\n", { isMd: false }); // EXAMPLE:
  expect(residue).toEqual([]);
});

// WHICH list that shadowing consults is pinned by injection rather than by a specimen: every
// code-list entry a candidate pattern can reach is also reachable from the skill list, so any real
// token is classified identically under both and a fixture-based test would pass regardless of the
// selection. Two fabricated lists that overlap in nothing make the selection observable — each case
// fails if the mapping is inverted or hardcoded to one side.
// The two tokens are interpolated rather than written into the fixture text, so the fixture stays
// program data under the scanner's own whitespace rule and reports no residue of its own when the
// coverage control scans it.
const CODE_TOKEN = "CodeOnly1";
const SKILL_TOKEN = "SkillOnly2";
const PROBE_BAN_LISTS = {
  code: [{ name: "code-list probe", re: /\bCodeOnly1\b/ }],
  md: [{ name: "skill-list probe", re: /\bSkillOnly2\b/ }],
};

test("scanCandidates checks a code file against the code ban list", () => {
  const { residue } = scanCandidates(`// ${CODE_TOKEN} and ${SKILL_TOKEN} both appear.\n`, {
    isMd: false,
    banLists: PROBE_BAN_LISTS,
  });
  expect(residue.map((r) => r.token)).toEqual([SKILL_TOKEN]);
});

test("scanCandidates checks a skill file against the skill ban list", () => {
  const { residue } = scanCandidates(`${CODE_TOKEN} and ${SKILL_TOKEN} both appear.\n`, {
    isMd: true,
    banLists: PROBE_BAN_LISTS,
  });
  expect(residue.map((r) => r.token)).toEqual([CODE_TOKEN]);
});

// The two ban lists are selected per file class, and the discriminator has to be a shape that one
// list carries and the other does not. A bare ISO date is that shape: a skill's dates are narrative
// ("user directive <date>") so the skill entry matches the bare form, while the code entry requires
// a parenthesised or "as of" writer because a bare date in code also appears as program data. These
// two cases fail in opposite directions if the selection is ever inverted.
test("skill mode flags a bare ISO date", () => {
  const { hits } = scanContent("Ruled in on 2026-07-30 and unchanged since.\n", { // EXAMPLE:
    isMd: true,
  });
  expect(hits.map((h) => h.kind)).toEqual(["date stamp"]);
});

test("code mode does not flag a bare ISO date, which is also program data", () => {
  const { hits } = scanContent('const cutoff = "2026-07-30"; // the retention boundary\n', {
    isMd: false,
  });
  expect(hits).toEqual([]);
});

// The acknowledgement lists are HIT-COUNTED and a zero-hit entry fails the gate, the same rule
// `check-skill-symbol-refs.mjs` applies to its own lists. Without it an entry stays alive after the
// sites that justified it are gone, and absorbs the first future candidate that happens to spell
// it with nothing in any output moving. Six entries died silently when the corpus stopped
// including vendored skill prose, which is the instance this rule was written from.
test("an acknowledgement entry the corpus never reaches is named, and a reached one is not", () => {
  const live = "product, protocol or algorithm name carrying a version-like number";
  expect(unusedAcknowledgements(new Map([[live, 3]]))).not.toContain(live);
  expect(unusedAcknowledgements(new Map([[live, 3]])).length).toBeGreaterThan(0);
  expect(unusedAcknowledgements(new Map())).toContain(live);
});

// The live-corpus assertion, which is where the rule has teeth: every entry currently on either
// list is reached by something. Revert direction: re-add any entry whose sites are gone and this
// turns red naming it.
test("every acknowledgement entry is reached by the live corpus", () => {
  expect(residueReport([]).unused).toEqual([]);
});

// The soundness condition. On a subset a zero hit says the scope did not reach the token, not that
// the entry is dead, so a scoped run must produce no finding at all rather than a list of entries
// the scope simply never covered.
test("a SCOPED run makes no zero-hit claim about an acknowledgement entry", () => {
  const scoped = residueReport(["src/server"]);
  expect(scoped.filesScanned).toBeGreaterThan(0);
  expect(scoped.unused).toEqual([]);
});

// Reach equality: the coverage control (`--residue`, backed by `residueReport`/`gateFileSet`)
// must examine exactly the file set the gate itself scans — both corpora, not a filtered subset
// of one. A control whose reach is narrower than the gate's reports clean over what it never read,
// and nothing in its output distinguishes that from a corpus it actually checked.
test.skipIf(!HAS_SKILLS_CHECKOUT)("residue control's file set is identical to the gate's file set", () => {
  const gate = gateFileSet([]);
  const residue = residueReport([]);
  expect(residue.filesScanned).toBe(gate.scanned.length);

  const { codeFiles, mdFiles } = collectFiles();
  expect(gate.scanned.length).toBe(codeFiles.length + mdFiles.length);
  expect(codeFiles.length).toBeGreaterThan(0);
  expect(mdFiles.length).toBeGreaterThan(0);

  // Every code file scanned is Rust/TS/Svelte/etc under a code root, never a skill markdown file,
  // and vice versa — the coverage control cannot silently collapse to markdown-only the way the
  // original defect did.
  for (const f of gate.scanned) {
    if (gate.isMdFile.has(f)) {
      expect(MD_EXTS.some((e) => f.endsWith(e))).toBe(true);
    } else {
      expect(f.endsWith(".md")).toBe(false);
    }
  }
});

// Positive controls, one per axis of the gate's corpus. A scope that reaches nothing and a scope
// that is genuinely clean both report zero, so each axis is pinned by a file the gate must
// actually be reading — membership in `gateFileSet`'s `scanned` array is reach, because that array
// is exactly what the scan iterates. Detection is pinned separately below, since a corpus the
// scanner reads but cannot lex is the same false negative one step later.
test("the gate's corpus reaches stylesheet sources", () => {
  const { scanned } = gateFileSet([]);
  const styles = scanned.filter((p) => p.endsWith(".scss"));
  expect(styles.length, "no stylesheet reached the scan").toBeGreaterThan(0);
});

test("the gate's corpus reaches the examples workspace", () => {
  const { scanned } = gateFileSet([]);
  const examples = scanned.filter((p) => p.startsWith("examples/"));
  expect(examples.length, "no example package file reached the scan").toBeGreaterThan(0);
});

// ts-rs output is out of scope by owner ruling — generated files are never hand-written comments.
// The exclusion is a path prefix, not a directory-name skip and not a content heuristic: it must
// remove exactly `GENERATED_ROOT`, nothing broader and nothing narrower.
test("the gate excludes ts-rs generated output under GENERATED_ROOT", () => {
  const { codeFiles, generatedFiles } = collectFiles();
  // Positive control: the directory holds real ts-rs output, so a clean result here is a genuine
  // exclusion, not a corpus that never reached the directory in the first place.
  expect(generatedFiles.length, "no ts-rs output found — the positive control is void").toBeGreaterThan(0);
  for (const p of codeFiles) {
    expect(p.startsWith(`${GENERATED_ROOT}/`)).toBe(false);
  }
  const { scanned } = gateFileSet([]);
  for (const p of scanned) {
    expect(p.startsWith(`${GENERATED_ROOT}/`)).toBe(false);
  }
});

// The exclusion must stop at the directory boundary: a hand-written sibling one level up
// (`src/types/index.ts`) shares the `src/types/` prefix with the generated directory but is not
// itself generated, and must stay in scope. A widened exclusion (e.g. matching on the ts-rs
// banner text, or on the bare "generated" substring) would silently drop it too.
test("the exclusion does not widen past the generated directory to its hand-written siblings", () => {
  const { codeFiles } = collectFiles();
  expect(
    codeFiles.includes("src/types/index.ts"),
    "a hand-written file next to the generated directory was excluded along with it",
  ).toBe(true);
});

// Stylesheet comment syntax, both forms. A `//` line comment and a `/* */` block are the only two
// a stylesheet has, and the block form carries its state across lines, so the id here sits on the
// continuation line rather than the opener.
test("a stylesheet line comment is lexed as comment text", () => {
  const { hits } = scanContent("--slate-950: #16161f; // re-audited at M8\n", { // EXAMPLE:
    isMd: false,
  });
  expect(hits.map((h) => h.kind)).toEqual(["milestone/task id"]);
});

test("a stylesheet block comment is lexed as comment text across its lines", () => {
  const { hits } = scanContent("/* Tier 1 raw primitives.\n   Re-audited at M12. */\n", { // EXAMPLE:
    isMd: false,
  });
  expect(hits.map((h) => h.line)).toEqual([2]);
  expect(hits.map((h) => h.kind)).toEqual(["milestone/task id"]);
});

// The grouped subject. A multi-word ban pattern is defeated by an ordinary line wrap at one of its
// spaces, and the two halves are individually innocent — so these cases pin the unit of the scan,
// not just its vocabulary. Each fixture writes the phrase with the break where prose wrapping would
// put it.
test("a milestone id split across a line wrap is one subject, not two clean halves", () => {
  const fixture =
    "silently truncate the chain. Fixed during Task\n6 as a real bug; the distinction is a no-op.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["milestone/task id"]);
  // Attribution stays per-line: the hit names the line the phrase STARTS on, not the group.
  expect(hits.map((h) => h.line)).toEqual([1]);
});

test("a history-narration phrase split across a line wrap is one subject", () => {
  const fixture = "// the comparator is evaluated once, unlike before the\n// fix it replaces\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: false });
  expect(hits.map((h) => h.kind)).toEqual(["history narration"]);
});

// A blank line ends a Markdown paragraph, so two innocent halves in DIFFERENT paragraphs stay
// innocent. Without this the grouping would run to the end of the file and manufacture phrases.
test("a paragraph break stops the join", () => {
  const fixture = "a sentence ending in Task\n\n6 opens the next paragraph.\n"; // EXAMPLE:
  expect(scanContent(fixture, { isMd: true }).hits).toEqual([]);
});

// The boundary that makes grouping safe. A doc comment's last words joined to the declaration
// beneath it manufactures a phrase nobody wrote out of a trailing sentence and a field name, which
// converts a false negative into a false positive.
test("a comment block is never joined to the code line beneath it", () => {
  const fixture = "/// Rolls each group deterministically.\npub spec: RollSpec,\n"; // EXAMPLE:
  expect(scanContent(fixture, { isMd: false }).hits).toEqual([]);
});

// The same boundary, from the other side: a comment TRAILING a statement is written against that
// statement, not as a continuation of the block above it. Joining the two splices a doc comment's
// last word onto the trailing comment's first and manufactures a marker phrase nobody wrote.
test("a trailing comment neither extends a comment block nor is extended by one", () => {
  const fixture = "// a trailing clause about Task\nlet n = count(); // 6 of them arrive\n"; // EXAMPLE:
  expect(scanContent(fixture, { isMd: false }).hits).toEqual([]);
});

test("a prose string literal neither extends a comment block nor is extended by one", () => {
  const fixture =
    '// a trailing clause about Task\nassert!(ok, "a message that mentions 6 of them");\n'; // EXAMPLE:
  expect(scanContent(fixture, { isMd: false }).hits).toEqual([]);
});

// Separator spelling. A marker's words may be joined by a hyphen, an underscore or a space, and the
// widening is derived from the pattern's own source rather than enumerated — so the hyphen-only
// spelling in the pattern must reach all three in the corpus.
test("a hyphen-only marker pattern matches its spaced and underscored spellings", () => {
  for (const written of ["buddy-check", "buddy check", "buddy_check"]) { // EXAMPLE:
    const { hits } = scanContent(`Found during a multi-round ${written} of the branch.\n`, {
      isMd: true,
    });
    expect(hits.map((h) => h.kind), written).toEqual(["sweep / round / review marker"]);
  }
});

// The widening rewrites the PATTERN, never the subject. A subject-side rewrite retokenizes the text
// — `_` is a word character while `-` and a space are not — which exposes boundaries the patterns
// rely on: a versioned constant splits into words whose suffix reads as a local marker, and a
// quantity fuses into a token that reads as one too. Both are ordinary prose this corpus carries.
test("a versioned constant is not retokenized into a local marker", () => {
  const fixture = "/// Engine-owned schema-vocabulary version (`SCHEMA_FORMAT_V1`).\n";
  expect(scanContent(fixture, { isMd: false }).hits).toEqual([]);
});

test("a spaced quantity is not fused into a local marker", () => {
  const fixture = "// A 1-hex token spans the hex it sits in.\n";
  expect(scanContent(fixture, { isMd: false }).hits).toEqual([]);
});

// A `-` inside a character class is a RANGE, and widening it corrupts the pattern outright: the
// rewrite turns the range into a three-character alternation, so the pattern silently stops
// matching the characters it governs. A BARE separator is widened only between two literal
// alphabetic characters.
test("separatorFlexible widens a word separator and leaves a character-class range alone", () => {
  // Three separators, one per outcome: the first joins two literal words and widens; the second is
  // a range inside a class; the third follows a class, so no literal word precedes it.
  const widened = separatorFlexible(/\bmarker-word[a-z]-x/);
  expect(widened.source).toBe("\\bmarker[-_\\s]word[a-z]-x");
});

test("separatorFlexible returns null for a pattern with no widenable separator", () => {
  expect(separatorFlexible(/\b[DITW]\d+\b/)).toBe(null);
});

// A character class whose every member is a separator needs no neighbour context to be recognised
// as one, so it is widened wherever it sits. A class carrying any non-separator member is a RANGE
// or a real alternation and is emitted untouched.
test("separatorFlexible widens a separator-only class and leaves every other class alone", () => {
  expect(String(separatorFlexible(/\bfix[- ]round/))).toBe(String(/\bfix[-_\s]round/));
  expect(String(separatorFlexible(/\b[Ss]weeps?[ -]\d+/))).toBe(String(/\b[Ss]weeps?[-_\s]\d+/));
  expect(separatorFlexible(/\bB[0-3]\b/)).toBeNull();
  expect(separatorFlexible(/docs\/[\w./-]+\.md/)).toBeNull();
});

// `[ -_]` is a RANGE spanning every character from space to underscore, not two separators.
test("separatorOnlyClass accepts only members that are separators, and never a range", () => {
  expect(separatorOnlyClass(" -")).toBe(true);
  expect(separatorOnlyClass("- ")).toBe(true);
  expect(separatorOnlyClass("\\s-")).toBe(true);
  expect(separatorOnlyClass("_")).toBe(true);
  expect(separatorOnlyClass(" -_")).toBe(false);
  expect(separatorOnlyClass("a-z")).toBe(false);
  expect(separatorOnlyClass("^ -")).toBe(false);
  expect(separatorOnlyClass("")).toBe(false);
});

// Widening inside a NEGATIVE lookaround widens an EXCLUSION: the pattern then matches strictly
// less and the gate reports strictly cleaner, which is the one direction nothing in the output
// distinguishes from a clean corpus. A live entry carries a separator class inside a lookbehind.
test("separatorFlexible widens nothing inside a negative lookaround, at any depth", () => {
  expect(separatorFlexible(/(?<!\bRFC[\s-]?\d{1,5})§\s*\d/)).toBeNull();
  expect(separatorFlexible(/(?!fix[- ]round)/)).toBeNull();
  expect(separatorFlexible(/(?!a(?:fix[- ]round))/)).toBeNull();
  // A POSITIVE lookaround and a named group are ordinary: widening there adds matches.
  expect(String(separatorFlexible(/(?=fix[- ]round)/))).toBe(String(/(?=fix[-_\s]round)/));
  expect(String(separatorFlexible(/(?<n>fix[- ]round)/))).toBe(String(/(?<n>fix[-_\s]round)/));
  // The exclusion is scoped to the lookaround, not to the rest of the pattern after it.
  expect(String(separatorFlexible(/(?!x[- ]y)fix[- ]round/))).toBe(
    String(/(?!x[- ]y)fix[-_\s]round/),
  );
});

// End to end through the ban list: the corpus spelling of a marker whose pattern carries a class.
test("a class-spelled marker pattern matches all three of its separator spellings", () => {
  for (const written of ["fix-round", "fix round", "fix_round"]) { // EXAMPLE:
    const { hits } = scanContent(`Converted during the ${written} that follows.\n`, {
      isMd: true,
    });
    expect(
      hits.map((h) => h.kind),
      written,
    ).toEqual(["sweep / round / review marker"]);
  }
});

// The coverage control itself, wired into `pnpm test:scripts` so an unrecognised form in the
// governed skill corpus fails CI instead of passing silently.
// Every acknowledged match must be real (present in ACKNOWLEDGED with a reason) and
// every remaining candidate must be empty — a red run here means a real corpus token nobody has
// looked at, not a flaky test.
test.skipIf(!HAS_SKILLS_CHECKOUT)("coverage control: the governed skill corpus has no unrecognised candidate tokens", () => {
  // Read from `collectFiles`, not a second walk of the skill root: an independent walk would
  // include the untracked vendored directories the gate excludes, and the control would then be
  // measuring a corpus the gate never scans.
  const files = collectFiles().mdFiles;
  const residue = [];
  let acknowledgedTotal = 0;
  for (const path of files) {
    const content = readFileSync(path, "utf8");
    const result = scanCandidates(content);
    acknowledgedTotal += result.acknowledged.length;
    for (const r of result.residue) residue.push({ path, ...r });
  }
  expect(files.length).toBeGreaterThan(0);
  expect(residue, JSON.stringify(residue, null, 2)).toEqual([]);
  // The acknowledged list is a live exemption, not dead code — this fails if every entry in
  // ACKNOWLEDGED ever stops matching anything in the corpus, so a future edit that empties a
  // reason out is caught here rather than left as a silent uncounted carve-out.
  expect(acknowledgedTotal).toBeGreaterThan(0);
});

// ---------------------------------------------------------------------------
// Separator respelling: the writers a bare separator could not reach.
//
// The bare-separator widening requires a LITERAL ALPHABETIC neighbour on both sides, so a separator
// beside a group boundary or an escape kept its single spelling and the marker's other two writers
// passed the gate clean. Each case below is a spelling that was reachable BEFORE the affected
// separator was respelled as a one-member character class; every one of them must now fail.
// ---------------------------------------------------------------------------

// One row per respelled separator, naming the neighbour that defeated the bare widening. A row's
// three writers must all reach the same entry: that is the property the widening exists to give,
// and a row failing on one writer is the entry reachable under two spellings out of three.
const RESPELLED_SEPARATORS = [
  ["history narration phrase, group boundary", "history narration",
    ["before the fix", "before-the-fix", "before_the_fix"]], // EXAMPLE:
  ["history narration phrase, past tense writer", "history narration",
    ["after the rewrite", "after-the-rewrite", "after_the_rewrite"]], // EXAMPLE:
  ["history narration compound, group boundary", "history narration",
    ["pre-refactor", "pre refactor", "pre_refactor"]], // EXAMPLE:
  ["process marker, group boundary", "process marker",
    ["dispatch brief", "dispatch-brief", "dispatch_brief"]], // EXAMPLE:
  ["process marker, second writer of the qualifier", "process marker",
    ["task brief", "task-brief", "task_brief"]], // EXAMPLE:
  ["sweep marker finding form, escape neighbour", "sweep / round / review marker",
    ["finding 3", "finding-3", "finding_3"]], // EXAMPLE:
  ["sweep marker severity form, escape neighbour", "sweep / round / review marker",
    ["critical 2", "critical-2", "critical_2"]], // EXAMPLE:
  ["milestone spelled-out form, escape neighbour", "milestone/task id",
    ["Task 4", "Task-4", "Task_4"]], // EXAMPLE:
  ["numbered constraint, escape neighbour", "numbered constraint",
    ["Constraint 5", "Constraint-5", "Constraint_5"]], // EXAMPLE:
  ["unnamed brief pointer, escape neighbour", "unnamed brief pointer",
    ["the brief requires", "the brief-requires", "the-brief_requires"]], // EXAMPLE:
];

test("every respelled separator reaches all three of its writers", () => {
  for (const [label, kind, writers] of RESPELLED_SEPARATORS)
    for (const written of writers) {
      const { hits } = scanContent(`// A comment mentioning ${written} in passing.\n`, {
        isMd: false,
      });
      expect(hits.map((h) => h.kind), `${label}: ${written}`).toContain(kind);
    }
});

// The date-stamp entry's qualifying phrase is its own case: it takes a following NUMBER rather
// than a following word, and only the separator inside the phrase passed the neighbour test, so a
// half-hyphenated writer matched while a fully hyphenated one did not.
test("the date-stamp qualifier reaches its fully separated writers", () => {
  for (const written of ["as of 2026", "as-of-2026", "as_of_2026"]) { // EXAMPLE:
    const { hits } = scanContent(`// Counted ${written} for the release window.\n`, {
      isMd: false,
    });
    expect(hits.map((h) => h.kind), written).toContain("date stamp");
  }
});

// The section-anchor writer, through the skill ruleset where the entry governs.
test("the section-pointer separator reaches all three of its writers", () => {
  for (const written of ["Section 3", "Section-3", "Section_3"]) { // EXAMPLE:
    const { hits } = scanContent(`Described in ${written} of the reference.\n`, { isMd: true });
    expect(hits.map((h) => h.kind), written).toContain("unnamed section pointer");
  }
});

// The measured counter-case, and the reason the neighbour test is not simply loosened. Respelling
// the local marker's optional hyphen would admit the SPACE writer, and an initial followed by a
// space and a digit is ordinary English — an indefinite article in front of a quantity. The
// hyphenated and unhyphenated writers must still fire; the spaced one must not.
test("the local letter+digit separator is deliberately not widened to a space", () => {
  for (const written of ["C-2", "C2"]) { // EXAMPLE:
    const { hits } = scanContent(`// Raised as ${written} during the pass.\n`, { isMd: false });
    expect(hits.map((h) => h.kind), written).toContain("local letter+digit marker");
  }
  const quantity = scanContent("// A 1-hex token spans the hex it sits in.\n", { isMd: false });
  expect(quantity.hits.map((h) => h.kind)).not.toContain("local letter+digit marker");
});

// The other measured counter-case: hyphenating that reference turns a noun into an adjectival
// compound naming a code symbol's scope, which is not a document pointer at all.
test("a hyphenated spec compound is not an unnamed spec reference", () => {
  const bare = scanContent("// Resolved per spec for this field.\n", { isMd: false }); // EXAMPLE:
  expect(bare.hits.map((h) => h.kind)).toContain("unnamed spec reference");
  const compound = scanContent("// A per-spec field, not a per-group one.\n", { isMd: false });
  expect(compound.hits.map((h) => h.kind)).not.toContain("unnamed spec reference");
});

// The widened "unnamed brief pointer" construction: a deferring verb reachable under "this" as
// well as "the", matching the determiner set "unnamed spec reference" already carries for "spec".
test("a determiner-plus-brief phrase deferring to it is an unnamed brief pointer", () => {
  const the = scanContent("// Not exactly the fixture the brief calls for.\n", { isMd: false }); // EXAMPLE:
  expect(the.hits.map((h) => h.kind)).toContain("unnamed brief pointer");
  const thisOne = scanContent("// This brief specifies the fixture shape.\n", { isMd: false }); // EXAMPLE:
  expect(thisOne.hits.map((h) => h.kind)).toContain("unnamed brief pointer");
});

// The ordinary-adjective collision "brief" is designed against: neither uses the determiner-plus-
// deferring-verb construction, so both stay unmatched.
test("'brief' as an ordinary adjective is not an unnamed brief pointer", () => {
  const pause = scanContent("// Logs a brief pause before the retry.\n", { isMd: false }); // EXAMPLE:
  expect(pause.hits.map((h) => h.kind)).not.toContain("unnamed brief pointer");
  const keepIt = scanContent("// Keep the summary short; keep it brief.\n", { isMd: false }); // EXAMPLE:
  expect(keepIt.hits.map((h) => h.kind)).not.toContain("unnamed brief pointer");
});

// The real collision found by reading `check-brief-rules.mjs`'s own prose: "brief" as the deferring
// verb's OBJECT describes the category of dispatch briefs, not one specific document, and must stay
// unmatched even though "the brief" appears immediately before a comma the way a genuine pointer
// does.
test("'brief' as a deferring verb's object describing the category is not an unnamed brief pointer", () => {
  const { hits } = scanContent(
    "// An implementer obeys the brief, not the guidance.\n", // EXAMPLE:
    { isMd: false },
  );
  expect(hits.map((h) => h.kind)).not.toContain("unnamed brief pointer");
});

// A date's hyphens are its FORMAT, not a word separator: widening them would read a
// space-separated triple of numbers as a date.
test("a date's own hyphens are not widened into other separators", () => {
  const iso = scanContent("Superseded by 2026-08-17-plan.md in the archive.\n", { isMd: true }); // EXAMPLE:
  expect(iso.hits.map((h) => h.kind)).toContain("dated plan/spec file");
  const spaced = scanContent("Sized 2026 08 17.md across the three columns.\n", { isMd: true });
  expect(spaced.hits.map((h) => h.kind)).not.toContain("dated plan/spec file");
});

// ---------------------------------------------------------------------------
// The instrument fingerprint.
// ---------------------------------------------------------------------------

// What the fingerprint HASHES is what makes a printed count comparable to an earlier one. A
// component left out produces a perfectly stable hash that fails to move when the rule it governs
// does, and the banner then offers a comparison between two runs measured by different rulers —
// an omission invisible from the fingerprint itself, which is why nothing detected the last one.
//
// The function names are read off the function VALUES, so renaming a hashed function fails here
// rather than silently drifting from a hand-written label.
test("the instrument fingerprint hashes exactly the components that decide a count", () => {
  expect(instrumentComponents().functions).toEqual([
    "splitLine",
    "lineSubject",
    "subjectGroups",
    "scanContent",
    "scanCandidates",
    "bannedMatchesIn",
    "separatorFlexible",
    "separatorOnlyClass",
    "under",
  ]);
  expect(instrumentComponents().patterns).toEqual([
    "BANNED",
    "SKILL_BANNED",
    "STRING_LITERAL",
    "EXPLANATORY_STRING",
    "PROSE_LITERAL",
    "EXAMPLE_EXEMPT",
    "DESIGN_DOC_CITATION",
    "CANDIDATE_TOKEN_LABEL",
    "CANDIDATE_TOKEN_WORD",
    "PRE_POST_NARRATION_TOKEN",
    "WORD_NARRATION_TOKEN",
  ]);
  expect(instrumentComponents().values).toEqual([
    "SEPARATOR_CLASS",
    "SEPARATOR_CHARS",
    "WHITESPACE_ESCAPES",
    "ROOTS",
    "EXTS",
    "MD_EXTS",
    "SKIP_DIRS",
    "GENERATED_ROOT",
    "ACKNOWLEDGED",
    "ACKNOWLEDGED_NARRATION",
  ]);
});

// The fingerprint is a pure function of the rules in force, so two calls in one process must agree.
// A hash that varied per call would refuse every comparison and read as constant instrument drift.
test("the instrument fingerprint is stable across calls", () => {
  expect(instrumentFingerprint()).toBe(instrumentFingerprint());
  expect(instrumentFingerprint()).toMatch(/^[0-9a-f]{8}$/);
});

// The report and the hash must enumerate the SAME components in the same order. They derive from
// one record today, so this holds by construction — and that is the point: the test is what fails
// if a second hand-written enumeration is reintroduced into either one, which is how the pattern
// and value groups came to be typed twice while the fingerprint quietly stopped covering them.
test("the hash input and the reported component list are the same enumeration", () => {
  const parts = instrumentComponents();
  const reported = [...parts.functions, ...parts.patterns, ...parts.values];
  expect(instrumentHashInput().map(([name]) => name)).toEqual(reported);
});

// A component whose hashed value is undefined serializes to null, so its changes stop moving the
// fingerprint while its name still appears in the report — the same silent hole, one level down.
test("every hashed component contributes a defined value", () => {
  for (const [name, hashed] of instrumentHashInput()) {
    expect(hashed, `${name} hashed to undefined`).toBeDefined();
  }
});

// ---------------------------------------------------------------------------
// Match iteration, and the flag that would corrupt it.
// ---------------------------------------------------------------------------

// Two instances of the SAME pattern in one subject are two violations. Reporting one leaves the
// block fixable only a gate run at a time, and the run reporting the second reads as a regression
// introduced by the fix for the first — the symptom the one-hit-per-group rule was already written
// against for two DIFFERENT patterns.
test("a group carrying two instances of one pattern reports both", () => {
  const fixture = "// Raised in sweep 2 and again in sweep 5 of the same branch.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: false });
  const sweeps = hits.filter((h) => h.kind === "sweep / round / review marker");
  expect(sweeps.map((h) => h.match)).toEqual(["sweep 2", "sweep 5"]); // EXAMPLE:
});

// One marker matched by both the direct and the widened spelling at one offset is ONE violation.
// De-duplicating by offset is what keeps the per-match iteration above from double-counting every
// hit whose separator the widening also matches.
test("both spellings matching at one offset report a single violation", () => {
  // A fabricated pattern, not a live entry: the property under test is de-duplication by offset,
  // which needs only a widenable separator, and a real marker would make the fixture a specimen.
  const matches = bannedMatchesIn(/\bwidget[ ]part/i, "a widget part here");
  expect(matches).toEqual([{ index: 2, text: "widget part" }]);
});

// A global ban pattern carries `lastIndex` state between subjects, so it would silently skip real
// hits. The refusal happens at construction, where it names the entry, rather than at report time,
// where it is indistinguishable from a clean corpus.
test("a ban entry carrying the global flag is refused at construction", () => {
  expect(() =>
    assertNoGlobalPatterns({ code: [{ name: "fabricated", re: /x/g }] }),
  ).toThrow(/fabricated.*global flag/s);
  // The lists actually in force must pass the same check they are constructed under.
  expect(() => assertNoGlobalPatterns({ code: BANNED, md: SKILL_BANNED })).not.toThrow();
});

// ---------------------------------------------------------------------------
// `separatorOnlyClass`: the property the prose states is the property the code tests.
// ---------------------------------------------------------------------------

// A separator written as an ESCAPE is still a separator. Rejecting one left the class unwidened —
// the safe direction, but it made three prose statements of the property false, and it is a trap
// that gets likelier as one-member classes become the way a word separator is marked.
test("separatorOnlyClass accepts a separator written as an escape", () => {
  expect(separatorOnlyClass("\\-")).toBe(true);
  expect(separatorOnlyClass("\\_")).toBe(true);
  expect(separatorOnlyClass("\\ ")).toBe(true);
  expect(separatorOnlyClass("\\t")).toBe(true);
  expect(separatorOnlyClass("\\-\\_\\t")).toBe(true);
  // An escaped `-` is never a range operator, so it carries no position requirement.
  expect(separatorOnlyClass("\\-_")).toBe(true);
  // Still rejected: a non-separator escape, and a trailing backslash with nothing after it.
  expect(separatorOnlyClass("\\w")).toBe(false);
  expect(separatorOnlyClass("\\d")).toBe(false);
  expect(separatorOnlyClass("\\")).toBe(false);
});

// The docblock's rejected/accepted specimens, pinned. A doc naming an ACCEPTED spelling as
// rejected sends an author to loosen the neighbour test instead of respelling the site — the
// repair `separatorFlexible` explicitly calls the wrong one, and the way a matcher acquires
// silent over-widening.
test("separatorOnlyClass rejects a numeric-escape separator and accepts a literal one", () => {
  expect(separatorOnlyClass("\\x20")).toBe(false);
  expect(separatorOnlyClass("\\u0020")).toBe(false);
  expect(separatorOnlyClass(" ")).toBe(true);
});

// A one-member class is how a source marks a separator as a word separator rather than regex
// punctuation. It must derive to the full separator class, or the respellings above do nothing.
test("a one-member separator class widens to the full separator class", () => {
  expect(String(separatorFlexible(/\bfinding[ ]\d+/))).toBe(String(/\bfinding[-_\s]\d+/));
  expect(String(separatorFlexible(/\bpre[-]fix\b/))).toBe(String(/\bpre[-_\s]fix\b/));
  expect(String(separatorFlexible(/\bTask[\s]+\d/))).toBe(String(/\bTask[-_\s]+\d/));
});

// ---------------------------------------------------------------------------
// The skill carve-out, implemented as a guard rather than as a dropped check.
// ---------------------------------------------------------------------------

// A skill may name a durable design document, so a bare architecture reference, a bare numbered
// invariant and a bare section anchor are permitted exactly when the line carries the path saying
// WHICH document. Both directions are controlled: a guard nobody can see fail is a dropped check.
test("skill mode permits a pathless anchor only beside a durable design-doc path", () => {
  const guarded = "- Rationale: `docs/design/ARCHITECTURE.md` §2 invariant 6 (three bands).\n"; // EXAMPLE:
  expect(scanContent(guarded, { isMd: true }).hits).toEqual([]);
  const bare = "Hidden fields are stripped before transmission (ARCHITECTURE §2 invariant 4).\n"; // EXAMPLE:
  const kinds = scanContent(bare, { isMd: true }).hits.map((h) => h.kind);
  expect(kinds).toContain("pathless durable document reference");
  expect(kinds).toContain("unnamed section pointer");
});

// The guard's unit is the LINE. Several permitted citations name the path once and carry a second
// anchor later on the same line joined by a `+`; a guard keyed to immediate adjacency would admit
// the first anchor and flag the second.
test("a permitted citation covers a second anchor later on the same line", () => {
  const fixture = "- Rationale: `docs/design/ARCHITECTURE.md` §2 (invariants 1-4) + §3 (stack).\n"; // EXAMPLE:
  expect(scanContent(fixture, { isMd: true }).hits).toEqual([]);
});

// The guard reads the RAW line, never the subject: the Markdown branch of `lineSubject` replaces
// every design-doc citation with the empty string before matching, which would delete exactly the
// evidence the guard needs and make every guarded citation fail.
test("the guard survives the design-doc citation strip applied to the subject", () => {
  // The digits in the filename are why the strip exists; the guard must still see the path.
  const fixture = "- Rationale: `docs/design/M2-data-foundation.md` §4 (document bands).\n"; // EXAMPLE:
  expect(scanContent(fixture, { isMd: true }).hits).toEqual([]);
});

// The guard is scoped to the skill corpus. Code has no durable-citation carve-out at all, so the
// same line in a comment fails whether or not it names a path.
test("code mode has no durable-citation carve-out", () => {
  const fixture = "// Rationale: `docs/design/ARCHITECTURE.md` §2 invariant 6.\n"; // EXAMPLE:
  const kinds = scanContent(fixture, { isMd: false }).hits.map((h) => h.kind);
  expect(kinds).toContain("repo document pointer");
  expect(kinds).toContain("pathless durable document reference");
});

// ---------------------------------------------------------------------------
// The two spellings the reference entries did not reach.
// ---------------------------------------------------------------------------

// A comment naming a codebase skill points at a knowledge artifact outside the code whose identity
// a process assigns. Scoped to CODE: a skill naming a sibling skill is the documented structure of
// that knowledge layer, and the core skill's own subsystem list is written that way — so both
// directions are controlled, because a ban that fired on the skill corpus would fail the corpus it
// is meant to describe.
test("a source comment naming a codebase skill is a pointer", () => {
  const fixture = "// Broadcast filtering lives here (see `shadowcat-codebase-chat`).\n"; // EXAMPLE:
  expect(scanContent(fixture, { isMd: false }).hits.map((h) => h.kind)).toEqual([
    "codebase skill pointer",
  ]);
});

test("a skill naming a sibling skill is not a pointer", () => {
  const fixture = "Invoke `shadowcat-codebase-core` first, then `shadowcat-codebase-chat`.\n"; // EXAMPLE:
  expect(scanContent(fixture, { isMd: true }).hits).toEqual([]);
});

// The tracker entry required the literal extension, so the identical pointer read as clean with
// four characters dropped. A POINTER CONSTRUCTION is required rather than a bare occurrence: the
// bare marker form is deliberately PERMITTED, and these names occur as ordinary prose.
test("a tracker named without its extension is still a pointer", () => {
  for (const written of ["see TODO", "logged in TODO", "in the OPEN_BUGS", "per PLAN"]) { // EXAMPLE:
    const fixture = `// Deferred — ${written} for the remaining cases.\n`;
    expect(
      scanContent(fixture, { isMd: false }).hits.map((h) => h.kind),
      written,
    ).toContain("extensionless tracker pointer");
  }
});

test("a bare code marker is not a tracker pointer", () => {
  // The `TODO:` marker itself stays: it is a code marker, not a deferral to a document.
  const marker = "// TODO: Extract token parsing into a stateless utility.\n";
  expect(scanContent(marker, { isMd: false }).hits).toEqual([]);
  // And the name occurring as ordinary prose, with no construction in front of it.
  const prose = "// A PLAN value is rejected when its steps disagree with the tree.\n";
  expect(scanContent(prose, { isMd: false }).hits).toEqual([]);
});

// ---------------------------------------------------------------------------
// Corpus scoping.
// ---------------------------------------------------------------------------

// An untracked skill directory is vendored third-party prose: this repo neither wrote it nor may
// edit it, so it is out of the corpus by that durable property. The skill-symbol-citation gate
// scopes the same directories the same way, and the two must not disagree about what the corpus is.
test.skipIf(!HAS_SKILLS_CHECKOUT)("the skill corpus is scoped to the tracked skill directories", () => {
  const { mdFiles, untrackedSkillFiles } = collectFiles();
  expect(mdFiles.length).toBeGreaterThan(0);
  for (const p of mdFiles) expect(untrackedSkillFiles).not.toContain(p);
  // Every scanned skill file sits under the skill root; nothing else sneaks in through the walk.
  for (const p of mdFiles) expect(p.startsWith(`${norm(SKILLS_ROOT)}/`)).toBe(true);
});
