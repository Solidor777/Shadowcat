import { renameSync } from "node:fs";
import { join } from "node:path";
import { test, expect } from "vitest";
import { makeRepo, runScriptIn } from "./lib/temp-repo.mjs";
import { scanInlineTests } from "./check-inline-tests.mjs";

test("a braced test module body is a violation", () => {
  const src = "fn a() {}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n}\n";
  expect(scanInlineTests(src)).toEqual([{ line: 4, module: "tests" }]);
});

test("a pub(crate) braced test module is a violation", () => {
  const src = "#[cfg(test)]\npub(crate) mod tests {\n}\n";
  expect(scanInlineTests(src)).toEqual([{ line: 2, module: "tests" }]);
});

test("a declaration-only test module passes", () => {
  expect(scanInlineTests("#[cfg(test)]\nmod tests;\n")).toEqual([]);
  expect(scanInlineTests("#[cfg(test)]\npub(crate) mod tests;\n")).toEqual([]);
});

test("cfg(test) on a non-module item passes", () => {
  expect(scanInlineTests("#[cfg(test)]\npub(crate) fn with_capacity_for_test(n: usize) {}\n")).toEqual([]);
  expect(scanInlineTests("    #[cfg(test)]\n    visible_cells_recompute_count: AtomicU64,\n")).toEqual([]);
  expect(scanInlineTests("#[cfg(test)]\nimpl SceneEcs {\n}\n")).toEqual([]);
});

test("the attribute inside a doc comment is prose", () => {
  expect(scanInlineTests("/// Write `#[cfg(test)]`\n/// mod tests {\nfn a() {}\n")).toEqual([]);
  expect(scanInlineTests("/* #[cfg(test)]\nmod tests { */\nfn a() {}\n")).toEqual([]);
});

test("blank lines, comments and further attributes between the attribute and the mod line do not hide it", () => {
  const src = "#[cfg(test)]\n\n// helpers\n#[rustfmt::skip]\nmod smoke {\n}\n";
  expect(scanInlineTests(src)).toEqual([{ line: 5, module: "smoke" }]);
});

test("two modules in one file are both reported", () => {
  const src = "#[cfg(test)]\nmod a {\n}\n\n#[cfg(test)]\nmod b {\n}\n";
  expect(scanInlineTests(src).map((v) => v.module)).toEqual(["a", "b"]);
});

// The corpus is the repository file list: an untracked, non-ignored Rust file is scanned before it
// is added to git, an ignored one never, and a tracked file deleted in the working tree is not read.
const GATE = "scripts/check-inline-tests.mjs";
const INLINE = "#[cfg(test)]\nmod tests {\n}\n";

test("an untracked Rust file with an inline test module fails the gate before git add", () => {
  const dir = makeRepo({ "src/a/lib.rs": "fn a() {}\n", "src/a/new.rs": INLINE }, ["src/a/lib.rs"]);
  const r = runScriptIn(dir, GATE);
  expect(r.status).toBe(1);
  expect(r.stderr).toContain("src/a/new.rs");
});

test("the same file under an ignored path passes", () => {
  const dir = makeRepo(
    { ".gitignore": "ignored/\n", "src/a/lib.rs": "fn a() {}\n", "src/a/ignored/new.rs": INLINE },
    [".gitignore", "src/a/lib.rs"],
  );
  expect(runScriptIn(dir, GATE).status).toBe(0);
});

test("a tracked file deleted in the working tree does not throw ENOENT", () => {
  const dir = makeRepo({ "src/a/lib.rs": "fn a() {}\n", "src/a/gone.rs": "fn g() {}\n" });
  // Moved aside, not deleted: the index still lists it while the working tree no longer has it.
  renameSync(join(dir, "src", "a", "gone.rs"), join(dir, "gone-aside.rs"));
  const r = runScriptIn(dir, GATE);
  expect(r.status).toBe(0);
  expect(r.stderr).not.toContain("ENOENT");
});
