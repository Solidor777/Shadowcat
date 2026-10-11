import { test, expect } from "vitest";
import { scanModuleDocs, isRustSource, runGate } from "./check-module-docs.mjs";

test("a documented file-backed mod is a violation", () => {
  expect(scanModuleDocs("/// Summary.\npub mod audio;\n")).toEqual([{ line: 2, module: "audio", inline: false }]);
});

test("a documented inline mod is a violation and is reported as inline", () => {
  expect(scanModuleDocs("/// Summary.\nmod cap {\n}\n")).toEqual([{ line: 2, module: "cap", inline: true }]);
});

test("an inner-only mod passes", () => {
  expect(scanModuleDocs("pub mod audio;\n")).toEqual([]);
  expect(scanModuleDocs("mod cap {\n    //! Summary.\n}\n")).toEqual([]);
});

test("attributes, blank lines and plain comments between the doc and the mod do not hide it", () => {
  expect(scanModuleDocs("/// Summary.\n#[cfg(test)]\n\n// note\npub(crate) mod t;\n")).toEqual([{ line: 5, module: "t", inline: false }]);
  expect(scanModuleDocs("/// Summary.\n#[cfg_attr(\n    test,\n    allow(x)\n)]\nmod t;\n")).toEqual([{ line: 6, module: "t", inline: false }]);
});

test("block and attribute doc forms are violations", () => {
  expect(scanModuleDocs("/** Summary. */\nmod a;\n")).toEqual([{ line: 2, module: "a", inline: false }]);
  expect(scanModuleDocs("/**\n * Summary.\n */\nmod a;\n")).toEqual([{ line: 4, module: "a", inline: false }]);
  expect(scanModuleDocs('#[doc = "Summary."]\nmod a;\n')).toEqual([{ line: 2, module: "a", inline: false }]);
});

test("a doc on a non-mod item does not carry over to a later mod", () => {
  expect(scanModuleDocs("/// A function.\nfn f() {}\nmod a;\n")).toEqual([]);
  expect(scanModuleDocs("/// A struct.\npub struct S;\n\nmod a;\n")).toEqual([]);
});

test("a four-slash line and a plain comment are not docs", () => {
  expect(scanModuleDocs("//// not a doc\nmod a;\n")).toEqual([]);
  expect(scanModuleDocs("// plain\nmod a;\n")).toEqual([]);
});

test("a mod keyword inside an identifier is not a mod item", () => {
  expect(scanModuleDocs("/// Doc.\nfn module() {}\n")).toEqual([]);
});

test("only Rust files under src are covered", () => {
  expect(isRustSource("src/server/src/lib.rs")).toBe(true);
  expect(isRustSource(String.raw`src\server\src\lib.rs`)).toBe(true);
  expect(isRustSource("src/client/core/src/a.ts")).toBe(false);
  expect(isRustSource("scripts/x.rs")).toBe(false);
});

function gate(files, listOk = true) {
  const logs = [];
  const errors = [];
  const code = runGate({
    listFiles: () => (listOk ? { ok: true, stdout: Object.keys(files).join("\0") } : { ok: false, message: "boom" }),
    readFile: (p) => files[p],
    log: (m) => logs.push(m),
    error: (m) => errors.push(m),
  });
  return { code, logs, errors };
}

test("the gate fails a documented mod fixture, names the rule, and passes inner-only", () => {
  const bad = gate({ "src/a/lib.rs": "/// Summary.\npub mod x;\n" });
  expect(bad.code).toBe(1);
  expect(bad.errors[0]).toContain("src/a/lib.rs:2");
  expect(bad.errors[0]).toContain("parent's scope");
  const inline = gate({ "src/a/lib.rs": "/// Summary.\nmod x {\n}\n" });
  expect(inline.code).toBe(1);
  expect(inline.errors[0]).toContain("inside the braces");
  const good = gate({ "src/a/lib.rs": "pub mod x;\n", "src/a/x.rs": "//! Summary.\n" });
  expect(good.code).toBe(0);
  expect(good.logs.at(-1)).toContain("2 files scanned, 0 error(s)");
});

test("an unlistable tree fails the gate", () => {
  const r = gate({}, false);
  expect(r.code).toBe(1);
  expect(r.errors[0]).toContain("boom");
});
