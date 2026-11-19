import { renameSync } from "node:fs";
import { join } from "node:path";
import { test, expect } from "vitest";
import { scanModuleDocs, runGate } from "./check-module-docs.mjs";
import { makeRepo, runScriptIn } from "./lib/temp-repo.mjs";

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

test("a doc followed by a one-line attribute-plus-mod is a violation and is consumed there", () => {
  expect(scanModuleDocs("/// Summary.\n#[cfg(test)] mod t;\n")).toEqual([{ line: 2, module: "t", inline: false }]);
  expect(scanModuleDocs("/// Summary.\n#[cfg(test)] mod t {\n}\n")).toEqual([{ line: 2, module: "t", inline: true }]);
});

test("a pending doc is not carried past a one-line attribute-plus-mod to a later undocumented mod", () => {
  expect(scanModuleDocs("/// Summary.\n#[cfg(test)] mod t;\nmod later;\n")).toEqual([{ line: 2, module: "t", inline: false }]);
  expect(scanModuleDocs("/// A struct.\n#[derive(Debug)] struct S;\nmod later;\n")).toEqual([]);
});

test("a one-line doc attribute on the mod line is a violation", () => {
  expect(scanModuleDocs('#[doc = "Summary."] mod a;\n')).toEqual([{ line: 1, module: "a", inline: false }]);
  expect(scanModuleDocs('#[doc = "has ] in it"]\nmod a;\n')).toEqual([{ line: 2, module: "a", inline: false }]);
  expect(scanModuleDocs("#[cfg(test)] mod a;\n")).toEqual([]);
});

test("a plain block comment between a doc and its mod does not clear the doc", () => {
  expect(scanModuleDocs("/// Summary.\n/* plain */\nmod a;\n")).toEqual([{ line: 3, module: "a", inline: false }]);
  expect(scanModuleDocs("/// Summary.\n/* plain\n   more */\nmod a;\n")).toEqual([{ line: 4, module: "a", inline: false }]);
});

test("a triple-slash line inside a plain block comment is not a doc", () => {
  expect(scanModuleDocs("/*\n/// not a doc\n*/\nmod a;\n")).toEqual([]);
  expect(scanModuleDocs("/* /// not a doc */\nmod a;\n")).toEqual([]);
});

test("a closed block-doc opener on a line inside a plain block comment is not a doc", () => {
  expect(scanModuleDocs("/*\n/** x */\nmod a;\n*/\n")).toEqual([]);
});

test("a closed block-doc opener inside a plain block comment does not doc the mod after the comment", () => {
  expect(scanModuleDocs("/*\n/** x */\n*/\nmod a;\n")).toEqual([]);
});

test("an unclosed block-doc opener inside a plain block comment does not doc the mod after the comment", () => {
  expect(scanModuleDocs("/*\n/** x\n*/\nmod a;\n")).toEqual([]);
});

test("an outer block doc followed by code on its closing line is a violation", () => {
  expect(scanModuleDocs("/** Summary. */ mod a;\n")).toEqual([{ line: 1, module: "a", inline: false }]);
  expect(scanModuleDocs("/**/\nmod a;\n")).toEqual([]);
});

test("a block comment that opens with more than two stars is not a doc", () => {
  expect(scanModuleDocs("/*** banner ***/\nmod a;\n")).toEqual([]);
  expect(scanModuleDocs("/***/\nmod a;\n")).toEqual([]);
  expect(scanModuleDocs("/****/\nmod a;\n")).toEqual([]);
});

test("stated bound: a triple-slash line inside a multi-line raw string is read as a doc", () => {
  expect(scanModuleDocs('let s = r#"\n/// text\nmod a;\n"#;\n')).toEqual([{ line: 3, module: "a", inline: false }]);
});

function gate(files, listOk = true) {
  const logs = [];
  const errors = [];
  const code = runGate({
    listFiles: () => {
      if (!listOk) throw new Error("boom");
      return Object.keys(files);
    },
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

const DOCUMENTED_MOD = "/// Summary.\npub mod x;\n";

test("an untracked Rust file with a documented mod fails the gate before git add", () => {
  const dir = makeRepo({ "src/a/lib.rs": "pub mod x;\n", "src/a/new.rs": DOCUMENTED_MOD }, ["src/a/lib.rs"]);
  const r = runScriptIn(dir, "scripts/check-module-docs.mjs");
  expect(r.status).toBe(1);
  expect(r.stderr).toContain("src/a/new.rs:2");
});

test("the same file under an ignored path passes", () => {
  const dir = makeRepo(
    { ".gitignore": "ignored/\n", "src/a/lib.rs": "pub mod x;\n", "src/a/ignored/new.rs": DOCUMENTED_MOD },
    [".gitignore", "src/a/lib.rs"],
  );
  expect(runScriptIn(dir, "scripts/check-module-docs.mjs").status).toBe(0);
});

test("a tracked file deleted in the working tree does not throw ENOENT", () => {
  const dir = makeRepo({ "src/a/lib.rs": "pub mod x;\n", "src/a/gone.rs": "pub mod y;\n" });
  // Moved aside, not deleted: the index still lists it while the working tree no longer has it.
  renameSync(join(dir, "src", "a", "gone.rs"), join(dir, "gone-aside.rs"));
  const r = runScriptIn(dir, "scripts/check-module-docs.mjs");
  expect(r.status).toBe(0);
  expect(r.stderr).not.toContain("ENOENT");
});
