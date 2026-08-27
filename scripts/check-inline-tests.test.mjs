import { test, expect } from "vitest";
import { scanInlineTests, isRustSource } from "./check-inline-tests.mjs";

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

test("isRustSource covers tracked .rs under src only", () => {
  expect(isRustSource("src/server/src/lib.rs")).toBe(true);
  expect(isRustSource("src\\server\\build.rs")).toBe(true);
  expect(isRustSource("scripts/x.mjs")).toBe(false);
  expect(isRustSource("target/debug/build/x.rs")).toBe(false);
});
