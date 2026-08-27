import { test, expect } from "vitest";
import {
  countLines,
  isCovered,
  parseAllowlist,
  evaluate,
  SOFT_LIMIT,
  HARD_LIMIT,
} from "./check-file-lines.mjs";

const lines = (n) => Array.from({ length: n }, (_, i) => `line ${i}`).join("\n") + "\n";

test("countLines matches wc -l and counts an unterminated final line", () => {
  expect(countLines("")).toBe(0);
  expect(countLines("a\n")).toBe(1);
  expect(countLines("a\nb")).toBe(2);
  expect(countLines(lines(5001))).toBe(5001);
});

test("isCovered admits source under the roots and rejects generated types and other extensions", () => {
  expect(isCovered("src/server/src/data/sqlite.rs")).toBe(true);
  expect(isCovered("src/client/shell/src/App.svelte")).toBe(true);
  expect(isCovered("scripts/check-file-lines.mjs")).toBe(true);
  expect(isCovered("examples/system-minimal/src/index.ts")).toBe(true);
  expect(isCovered("src/types/generated/ServerMsg.ts")).toBe(false);
  expect(isCovered("docs/superpowers/plans/x.md")).toBe(false);
  expect(isCovered("pnpm-lock.yaml")).toBe(false);
  expect(isCovered("src\\server\\src\\lib.rs")).toBe(true);
});

test("parseAllowlist accepts [[file]] tables of quoted strings and errors on anything else", () => {
  const text = `# header\n[[file]]\npath = "src/a.rs"\nlines_at_approval = "5321"\nreason = "why"\n`;
  expect(parseAllowlist(text, "x.toml")).toEqual([
    { path: "src/a.rs", lines_at_approval: "5321", reason: "why" },
  ]);
  expect(() => parseAllowlist("[[allow]]\n", "x.toml")).toThrow(/x\.toml:1/);
  expect(() => parseAllowlist("path = 5\n", "x.toml")).toThrow(/x\.toml:1/);
});

test("evaluate: soft fail above 5000 without an entry, pass with one", () => {
  const files = [{ path: "src/a.rs", lines: SOFT_LIMIT + 1 }];
  expect(evaluate({ files, allow: [] }).map((e) => e.kind)).toEqual(["SOFT LIMIT"]);
  const allow = [{ path: "src/a.rs", lines_at_approval: "5001", reason: "r" }];
  expect(evaluate({ files, allow })).toEqual([]);
});

test("evaluate: exactly 5000 passes; hard fail above 10000 even when allowlisted", () => {
  expect(evaluate({ files: [{ path: "src/a.rs", lines: SOFT_LIMIT }], allow: [] })).toEqual([]);
  const files = [{ path: "src/a.rs", lines: HARD_LIMIT + 1 }];
  const allow = [{ path: "src/a.rs", lines_at_approval: "10001", reason: "r" }];
  expect(evaluate({ files, allow }).map((e) => e.kind)).toEqual(["HARD LIMIT"]);
});

test("evaluate: an allowlist entry for a file at or under 5000, or not in the file set, is stale", () => {
  const allow = [
    { path: "src/small.rs", lines_at_approval: "5001", reason: "r" },
    { path: "src/gone.rs", lines_at_approval: "5001", reason: "r" },
  ];
  const files = [{ path: "src/small.rs", lines: 4999 }];
  expect(evaluate({ files, allow }).map((e) => [e.kind, e.path])).toEqual([
    ["STALE ALLOWLIST ENTRY", "src/small.rs"],
    ["STALE ALLOWLIST ENTRY", "src/gone.rs"],
  ]);
});

test("evaluate: every error names the path and the measured count", () => {
  const [e] = evaluate({ files: [{ path: "src/big.rs", lines: 12000 }], allow: [] });
  expect(e.message).toContain("src/big.rs");
  expect(e.message).toContain("12000");
});
