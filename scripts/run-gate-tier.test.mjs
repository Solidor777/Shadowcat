import { test, expect } from "vitest";
import { mkdtempSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  tierCommands,
  manifestHash,
  writeReceipt,
  readReceipt,
  receiptMatches,
} from "./run-gate-tier.mjs";

const E = (job, command, tier) => ({ job, command, tier, reason: "", line: 1 });

test("the commit tier is exactly the commit entries, in workflow order", () => {
  const entries = [
    E("rust", "cargo fmt --all -- --check", "commit"),
    E("rust", "cargo test --all", "push"),
    E("web", "pnpm lint", "commit"),
    E("ui-e2e", "pnpm e2e", "ci-only"),
    E("docs", "pnpm install --frozen-lockfile", "setup"),
  ];
  expect(tierCommands(entries, "commit")).toEqual(["cargo fmt --all -- --check", "pnpm lint"]);
});

test("the push tier contains the commit tier and preserves workflow order", () => {
  const entries = [
    E("rust", "pnpm build", "push"),
    E("rust", "cargo fmt --all -- --check", "commit"),
    E("rust", "cargo test --all", "push"),
  ];
  expect(tierCommands(entries, "push")).toEqual([
    "pnpm build",
    "cargo fmt --all -- --check",
    "cargo test --all",
  ]);
});

test("a command repeated across jobs runs once, at its first position", () => {
  const entries = [
    E("rust", "pnpm build", "push"),
    E("web", "pnpm build", "push"),
    E("web", "pnpm -r test", "push"),
  ];
  expect(tierCommands(entries, "push")).toEqual(["pnpm build", "pnpm -r test"]);
});

test("setup and ci-only never run locally", () => {
  const entries = [E("e2e", "pnpm --filter @shadowcat/core test:e2e", "ci-only")];
  expect(tierCommands(entries, "push")).toEqual([]);
});

test("a receipt round-trips and matches only its own tree and manifest", () => {
  const dir = mkdtempSync(join(tmpdir(), "gate-"));
  mkdirSync(join(dir, "sub"), { recursive: true });
  const m = manifestHash('[[gate]]\njob = "x"\n');
  writeReceipt(dir, { tree: "aaa", sha: "sha1", manifest: m, finishedAt: "t" });
  const r = readReceipt(dir);
  expect(r.tree).toBe("aaa");
  expect(receiptMatches(r, { tree: "aaa", manifest: m }).ok).toBe(true);
  expect(receiptMatches(r, { tree: "bbb", manifest: m }).ok).toBe(false);
  expect(receiptMatches(r, { tree: "bbb", manifest: m }).why).toMatch(/tree/);
  expect(receiptMatches(r, { tree: "aaa", manifest: "other" }).why).toMatch(/manifest/);
});

test("a missing receipt is reported, not thrown", () => {
  const dir = mkdtempSync(join(tmpdir(), "gate-"));
  expect(readReceipt(dir)).toBe(null);
  expect(receiptMatches(null, { tree: "aaa", manifest: "m" })).toEqual({
    ok: false,
    why: "no receipt: run `pnpm gate:push`",
  });
});
