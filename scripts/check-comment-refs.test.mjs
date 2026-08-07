import { test, expect } from "vitest";
import { scanContent } from "./check-comment-refs.mjs";

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
