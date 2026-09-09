import { test, expect, vi } from "vitest";
import { mkdtempSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  tierCommands,
  manifestHash,
  receiptPath,
  writeReceipt,
  readReceipt,
  receiptMatches,
  pushDirtyTreeRefusal,
  pushDirtyTreeRefusalAfterRun,
  headMovedRefusal,
} from "./run-gate-tier.mjs";
import { parseGateManifest, MANIFEST } from "./check-gate-manifest.mjs";

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

test("a setup-tier entry never runs in the push tier, distinctly from ci-only", () => {
  const entries = [E("rust", "pnpm install --frozen-lockfile", "setup")];
  expect(tierCommands(entries, "push")).toEqual([]);
});

test("in the real manifest, `pnpm build` precedes every cargo-prefixed push command", () => {
  const entries = parseGateManifest(readFileSync(MANIFEST, "utf8"), MANIFEST);
  const commands = tierCommands(entries, "push");
  const buildIndex = commands.indexOf("pnpm build");
  expect(buildIndex).toBeGreaterThanOrEqual(0);
  const cargoIndexes = commands
    .map((c, i) => (c.startsWith("cargo") ? i : -1))
    .filter((i) => i >= 0);
  expect(cargoIndexes.length).toBeGreaterThan(0);
  for (const i of cargoIndexes) expect(i).toBeGreaterThan(buildIndex);
});

test("a receipt round-trips and matches only its own tree and manifest", () => {
  const dir = mkdtempSync(join(tmpdir(), "gate-"));
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

test("a corrupt receipt is reported distinctly from a missing one, not thrown", () => {
  const dir = mkdtempSync(join(tmpdir(), "gate-"));
  writeFileSync(receiptPath(dir), "{ this is not json");
  const spy = vi.spyOn(console, "error").mockImplementation(() => {});
  expect(readReceipt(dir)).toBe(null);
  expect(spy).toHaveBeenCalledTimes(1);
  expect(spy.mock.calls[0][0]).toMatch(/corrupt/);
  spy.mockRestore();
});

test("push mode refuses on a dirty tree, before any gate runs", () => {
  expect(pushDirtyTreeRefusal(" M scripts/run-gate-tier.mjs\n")).toEqual({
    ok: false,
    why: expect.stringMatching(/refusing.*dirty/),
  });
});

test("push mode proceeds on a clean tree", () => {
  expect(pushDirtyTreeRefusal("")).toEqual({ ok: true, why: "" });
  expect(pushDirtyTreeRefusal("   \n")).toEqual({ ok: true, why: "" });
});

test("the receipt write is refused if the tree went dirty during the run", () => {
  expect(pushDirtyTreeRefusalAfterRun(" M scripts/run-gate-tier.mjs\n")).toEqual({
    ok: false,
    why: expect.stringMatching(/refusing to write the receipt.*dirty/),
  });
  expect(pushDirtyTreeRefusalAfterRun("")).toEqual({ ok: true, why: "" });
});

test("the receipt write is refused if HEAD moved during the run", () => {
  expect(headMovedRefusal("aaa111", "bbb222")).toEqual({
    ok: false,
    why: expect.stringMatching(/refusing to write the receipt.*HEAD moved/),
  });
  expect(headMovedRefusal("aaa111", "aaa111")).toEqual({ ok: true, why: "" });
});
