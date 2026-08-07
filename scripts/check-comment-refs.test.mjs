import { readFileSync } from "node:fs";
import { test, expect } from "vitest";
import {
  scanContent,
  scanCandidates,
  sources,
  MD_ROOTS,
  MD_EXTS,
} from "./check-comment-refs.mjs";

// Every fixture string below is a SPECIMEN whose exact wording is the thing under test — an id, a
// date, a filename this suite must reproduce verbatim to prove the pattern matches it. Each such
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

test("skill mode flags a sweep/round/review marker", () => {
  const fixture = "Docs-ratchet is live here (docs sweep 2b).\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["sweep / round / review marker"]);
});

test("skill mode flags a dated plan filename", () => {
  const fixture =
    "Plan: `docs/superpowers/plans/2026-07-15-m13a-formula-library.md`.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["dated plan/spec file"]);
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

// Skill-only local-marker categories (never reused by BANNED — src carries live identifiers and
// test names of the identical shape, so a shared entry would fail the code-file-count-0 gate).
test("skill mode flags a bare local letter+digit marker", () => {
  const fixture = "documented as the C1 no-pool-query-on-the-hot-path property.\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["local letter+digit marker"]);
});

test("code mode does NOT gain the local letter+digit marker category", () => {
  const { hits } = scanContent('const label = "C1"; // a real identifier, not prose\n', {
    isMd: false,
  });
  expect(hits).toEqual([]);
});

test("skill mode flags a numbered constraint reference", () => {
  const fixture = "requires exactly one runtime instance (Global Constraint 1).\n"; // EXAMPLE:
  const { hits } = scanContent(fixture, { isMd: true });
  expect(hits.map((h) => h.kind)).toEqual(["numbered constraint"]);
});

test("code mode does NOT gain the numbered constraint category", () => {
  const { hits } = scanContent('it("Constraint 1: single runtime instance", () => {});\n', {
    isMd: false,
  });
  expect(hits).toEqual([]);
});

// Coverage control (`scanCandidates`): a genuine miss, an acknowledged legitimate token, and a
// BANNED-shadowed candidate must resolve to exactly the right bucket.
test("scanCandidates reports a genuinely novel shape as residue", () => {
  const { residue, acknowledged } = scanCandidates(
    "Fixed in Sprint 4 without a regression.\n",
  );
  expect(residue.map((r) => r.token)).toEqual(["Sprint 4"]);
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

test("scanCandidates respects the EXAMPLE: exemption", () => {
  const { residue, acknowledged, exempted } = scanCandidates(
    "- EXAMPLE: `Sprint 4` is not a real reference.\n",
  );
  expect(residue).toEqual([]);
  expect(acknowledged).toEqual([]);
  expect(exempted).toBe(1);
});

// The coverage control itself, wired into `pnpm test:scripts` so a new unrecognised form in the
// governed skill corpus fails CI instead of passing silently the way the spelled-out task-id form
// once did. Every acknowledged match must be real (present in ACKNOWLEDGED with a reason) and
// every remaining candidate must be empty — a red run here means a real corpus token nobody has
// looked at, not a flaky test.
test("coverage control: the governed skill corpus has no unrecognised candidate tokens", () => {
  const files = MD_ROOTS.flatMap((d) => sources(d, MD_EXTS));
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
