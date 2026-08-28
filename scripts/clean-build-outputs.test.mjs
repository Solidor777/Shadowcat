import { mkdtempSync, mkdirSync, writeFileSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { test, expect } from "vitest";
import { TARGETS, resolveTargets, assertAllowed, clean, removeRecoverably } from "./clean-build-outputs.mjs";

function repo() {
  const root = mkdtempSync(join(tmpdir(), "clean-"));
  for (const d of ["dist", "dist-docs", "docs/site/.vitepress/dist", "examples/a/dist", "examples/b/dist", "target/package", "src/client/core/src"]) {
    mkdirSync(join(root, d), { recursive: true });
    writeFileSync(join(root, d, "x.txt"), "x");
  }
  return root;
}

test("TARGETS is the enumerated list and nothing else", () => {
  expect(TARGETS).toEqual(["dist", "dist-docs", "docs/site/.vitepress/dist", "examples/*/dist", "target/package"]);
});

test("resolveTargets expands examples/*/dist and skips absent dirs", () => {
  const root = repo();
  const got = resolveTargets(root, TARGETS).map((p) => p.slice(root.length + 1).split("\\").join("/")).sort();
  expect(got).toEqual(["dist", "dist-docs", "docs/site/.vitepress/dist", "examples/a/dist", "examples/b/dist", "target/package"]);
  expect(resolveTargets(root, ["dist-docs"]).length).toBe(1);
  expect(resolveTargets(mkdtempSync(join(tmpdir(), "empty-")), TARGETS)).toEqual([]);
});

test("assertAllowed refuses anything outside the enumerated output dirs", () => {
  const root = repo();
  expect(() => assertAllowed(root, join(root, "dist"))).not.toThrow();
  expect(() => assertAllowed(root, join(root, "examples", "a", "dist"))).not.toThrow();
  expect(() => assertAllowed(root, join(root, "src", "client", "core"))).toThrow(/refus/i);
  expect(() => assertAllowed(root, join(root, "docs", "site"))).toThrow(/refus/i);
  expect(() => assertAllowed(root, resolve(root, ".."))).toThrow(/refus/i);
  expect(() => assertAllowed(root, join(root, "dist", "..", "src"))).toThrow(/refus/i);
});

test("clean removes exactly the resolved targets through the injected remover", async () => {
  const root = repo();
  const removed = [];
  const out = await clean({ root, patterns: TARGETS, remove: async (p) => { removed.push(p); } });
  expect(out.length).toBe(6);
  expect(removed).toEqual(out);
  expect(removed.some((p) => p.includes("src"))).toBe(false);
  expect(existsSync(join(root, "src/client/core/src/x.txt"))).toBe(true);
});

test("clean with an unlisted pattern refuses before removing anything", async () => {
  const root = repo();
  const removed = [];
  await expect(clean({ root, patterns: ["src"], remove: async (p) => { removed.push(p); } })).rejects.toThrow(/refus/i);
  expect(removed).toEqual([]);
});

test("removeRecoverably sends a real path through trash and it no longer exists", async () => {
  const dir = mkdtempSync(join(tmpdir(), "clean-recoverable-"));
  const file = join(dir, "x.txt");
  writeFileSync(file, "x");
  expect(existsSync(file)).toBe(true);
  await removeRecoverably(file);
  expect(existsSync(file)).toBe(false);
});
