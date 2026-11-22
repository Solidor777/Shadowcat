import { test, expect } from "vitest";
import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { splitLine } from "./comment-span.mjs";
import { norm, repoFiles } from "./gate-corpus.mjs";
import { makeRepo } from "./temp-repo.mjs";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

// This repo permits no permanent-deletion call (owner ruling): `rmSync`/`rmdirSync`/`unlinkSync`
// and their bare async/callback and `node:fs/promises` counterparts all destroy a path immediately
// and irrecoverably, whether the target is tracked source, a debug dump, or a directory a test
// itself created — a test fixture gets no exemption, because the rule is about the API called, not
// about who owns the path it is called on. Every deletion need is served by `trash` (see
// `removeRecoverably` in `clean-build-outputs.mjs`) or by a fixture rewritten in place at one fixed
// path instead of a fresh one per run (`check-skill-symbol-refs-cli.test.mjs`'s own precedent).
//
// The Rust server follows the same rule: a permanent-deletion call is legal only inside the trash
// module (`src/server/src/trash.rs`, whose `remove_path` backs `Trash::discard`, and the module's own
// test file), which every other site reaches through `Trash::discard` / `Trash::discard_sync`.
//
// Same shape as `run-git.test.mjs`'s GIT-CALL-BUDGET check: enumerated via `git ls-files` rather
// than a directory walk, so an entry point living anywhere in the repository corpus is in scope by
// construction rather than by someone remembering to widen a root.

const FS_MODULES = new Set(["fs", "node:fs", "fs/promises", "node:fs/promises"]);
// The permanent-deletion API family: matched on the MEMBER name, since `rm`/`unlink`/`rmdir` are
// exported identically (same name, no `Sync` suffix) from both "fs" (async callback form) and
// "fs/promises" (promise form) — banning the name bans both shapes at once.
const BANNED_NAMES = new Set(["rmSync", "rm", "unlinkSync", "unlink", "rmdirSync", "rmdir"]);
// Cheap pre-filter so a file naming none of these identifiers is never even parsed.
const MENTIONS_BANNED_NAME = new RegExp(`\\b(${[...BANNED_NAMES].join("|")})\\b`);

const SCANNED_EXTENSIONS = [".js", ".mjs", ".cjs", ".ts", ".mts", ".cts"];

// The Rust permanent-deletion family, matched by NAME (not by `std::fs::` / `tokio::fs::` path) so a
// bare call after `use std::fs::remove_file;` and the `fs::remove_file(..)` shorthand are caught
// with no path resolution.
const RUST_BANNED_NAME = /\b(remove_file|remove_dir_all|remove_dir)\b/;
// A banned identifier followed by `(`, excluding a function DEFINITION (`fn remove_file(`). `_` is a
// word character, so `remove_dir` can never match inside `remove_dir_all`.
const RUST_BANNED_CALL = /(?<!\bfn\s+)\b(remove_file|remove_dir_all|remove_dir)\s*\(/g;
// Cheap pre-filter: a Rust file naming none of the identifiers is never scanned.
const MENTIONS_RUST_BANNED_NAME = /remove_file|remove_dir/;

// Per-file exceptions, each named with its reason — never a pattern. The JS side has none: the one
// place `assemble-docs.mjs` deletes a whole build output tree routes through `trash` via
// `removeRecoverably`. The Rust side names the trash module: `trash.rs` holds the one real removal
// (`remove_path`, the only code that destroys a trashed item), and `trash/tests.rs` calls
// `std::fs::remove_file` to prove a discarded file is gone from its original location.
const EXCLUDED_FILES = new Set(["src/server/src/trash.rs", "src/server/src/trash/tests.rs"]);

/** True when `node` is a string literal (or template literal with no substitutions) naming a fs module. */
function isFsModuleLiteral(node) {
  return node !== undefined && ts.isStringLiteralLike(node) && FS_MODULES.has(node.text);
}

/**
 * Every direct call to a `node:fs`/`node:fs/promises` permanent-deletion function in `text`, as
 * `{ line, name }` — `name` is the canonical member name (`rmSync`, `rm`, `unlink`, ...) regardless
 * of any local alias. Resolved through real import/require bindings rather than by matching the
 * banned identifiers as bare text, so an unrelated local function named `rm` is not flagged and a
 * renamed import (`import { rmSync as nuke }`) still is. Covers: a named import/`require`
 * destructure of the banned member; a call through a namespace/default binding of the module
 * (`fs.rmSync(...)`); and the same reached through `.promises` off an `fs`-namespace binding
 * (`fs.promises.rm(...)`) or an inline `require("fs").rmSync(...)` / `require("fs/promises").rm(...)`.
 */
export function dangerousFsDeleteCalls(text, scriptKind = ts.ScriptKind.JS) {
  const source = ts.createSourceFile("scan", text, ts.ScriptTarget.Latest, true, scriptKind);

  // local identifier -> canonical banned member name.
  const namedBindings = new Map();
  // local identifiers bound to an entire fs module namespace.
  const namespaceBindings = new Set();

  const collectBindings = (node) => {
    if (ts.isImportDeclaration(node) && isFsModuleLiteral(node.moduleSpecifier)) {
      const clause = node.importClause;
      if (clause) {
        if (clause.name) namespaceBindings.add(clause.name.text);
        const nb = clause.namedBindings;
        if (nb && ts.isNamespaceImport(nb)) namespaceBindings.add(nb.name.text);
        if (nb && ts.isNamedImports(nb)) {
          for (const el of nb.elements) {
            const exported = (el.propertyName ?? el.name).text;
            if (BANNED_NAMES.has(exported)) namedBindings.set(el.name.text, exported);
          }
        }
      }
    }
    if (
      ts.isVariableDeclaration(node) &&
      node.initializer &&
      ts.isCallExpression(node.initializer) &&
      ts.isIdentifier(node.initializer.expression) &&
      node.initializer.expression.text === "require" &&
      isFsModuleLiteral(node.initializer.arguments[0])
    ) {
      const name = node.name;
      if (ts.isIdentifier(name)) {
        namespaceBindings.add(name.text);
      } else if (ts.isObjectBindingPattern(name)) {
        for (const el of name.elements) {
          if (!ts.isIdentifier(el.name)) continue;
          const exported = (el.propertyName ?? el.name).text;
          if (BANNED_NAMES.has(exported)) namedBindings.set(el.name.text, exported);
        }
      }
    }
    ts.forEachChild(node, collectBindings);
  };
  collectBindings(source);

  /** Whether `expr` resolves to a bound fs-module namespace, directly, through `.promises`, or through an inline `require(...)`. */
  const isFsNamespaceExpr = (expr) => {
    if (ts.isIdentifier(expr)) return namespaceBindings.has(expr.text);
    if (ts.isPropertyAccessExpression(expr) && expr.name.text === "promises") {
      return isFsNamespaceExpr(expr.expression);
    }
    if (
      ts.isCallExpression(expr) &&
      ts.isIdentifier(expr.expression) &&
      expr.expression.text === "require"
    ) {
      return isFsModuleLiteral(expr.arguments[0]);
    }
    return false;
  };

  const calls = [];
  const visit = (node) => {
    if (ts.isCallExpression(node)) {
      const callee = node.expression;
      if (ts.isIdentifier(callee) && namedBindings.has(callee.text)) {
        calls.push({
          line: source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1,
          name: namedBindings.get(callee.text),
        });
      } else if (
        ts.isPropertyAccessExpression(callee) &&
        BANNED_NAMES.has(callee.name.text) &&
        isFsNamespaceExpr(callee.expression)
      ) {
        calls.push({
          line: source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1,
          name: callee.name.text,
        });
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return calls;
}

/**
 * Every Rust permanent-deletion reference in `text`, as `{ line, name, kind }`: `kind` is `"call"`
 * for a banned identifier followed by `(` (a `fn` definition excluded) and `"use"` for a banned
 * identifier anywhere inside a `use` declaration, called or not. The `use` rule makes an aliased
 * import (`use std::fs::remove_file as zap;`, `use tokio::fs::{remove_dir_all as rmrf};`) a violation
 * at the import, where the original name is still spelled, so no alias tracking is needed. Only the
 * code span of each line is read (`splitLine`, which also understands Rust raw strings); text inside
 * a string literal stays in the code span, so a literal spelling a banned call is a false positive a
 * reviewer polices. A macro that expands to a call without spelling the name is out of reach of any
 * lexical scan.
 */
export function dangerousRustFsDeleteCalls(text) {
  let state = { inBlock: false, inHtml: false };
  const codeLines = [];
  for (const raw of text.split("\n")) {
    const split = splitLine(raw, state);
    state = split.state;
    codeLines.push(split.code);
  }
  const code = codeLines.join("\n");
  const lineOf = (offset) => code.slice(0, offset).split("\n").length;
  const found = [];

  for (const m of code.matchAll(RUST_BANNED_CALL)) {
    found.push({ line: lineOf(m.index), name: m[1], kind: "call", offset: m.index });
  }

  for (const m of code.matchAll(/\buse\b/g)) {
    // The declaration ends at the first `;` outside any `{ }` group, so a multi-line grouped
    // import is one span.
    let depth = 0;
    let end = m.index + m[0].length;
    while (end < code.length) {
      const ch = code[end];
      if (ch === "{") depth += 1;
      else if (ch === "}") depth -= 1;
      else if (ch === ";" && depth <= 0) break;
      end += 1;
    }
    const span = code.slice(m.index, end);
    for (const hit of span.matchAll(new RegExp(RUST_BANNED_NAME.source, "g"))) {
      const offset = m.index + hit.index;
      found.push({ line: lineOf(offset), name: hit[1], kind: "use", offset });
    }
  }

  return found.sort((a, b) => a.offset - b.offset).map(({ line, name, kind }) => ({ line, name, kind }));
}

/** The parser mode for `rel`: TypeScript for a `.ts`/`.mts`/`.cts` file, JavaScript otherwise. */
function scriptKindOf(rel) {
  return /\.[cm]?ts$/.test(rel) ? ts.ScriptKind.TS : ts.ScriptKind.JS;
}

/**
 * Every path in the shared repository corpus (`repoFiles`, minus files deleted in the working
 * tree), unfiltered. `include: "all"` adds untracked, non-ignored files, which a ban scan reads so a
 * banned call is caught before `git add`; `include: "tracked"` reads the index only, which an
 * existence check reads so a file CI will not have cannot satisfy it.
 */
function listCorpusFiles(repoRoot, include = "all") {
  return repoFiles({ cwd: repoRoot, include, log: () => {} });
}

/**
 * Every file of the corpus under `repoRoot` (see `listCorpusFiles` for `include`) with a scanned
 * extension (JavaScript/TypeScript, or `.rs`) that can name a banned member at all, as
 * `{ rel, text, rust, scriptKind }`. A file mentioning none of the banned names cannot contain a
 * matching call, so it is dropped before the parse.
 */
function corpusSources(repoRoot, include = "all") {
  const out = [];
  for (const entry of listCorpusFiles(repoRoot, include)) {
    const rel = norm(entry);
    const rust = rel.endsWith(".rs");
    if (!rust && !SCANNED_EXTENSIONS.some((ext) => rel.endsWith(ext))) continue;
    const text = readFileSync(resolve(repoRoot, entry), "utf8");
    if (!(rust ? MENTIONS_RUST_BANNED_NAME : MENTIONS_BANNED_NAME).test(text)) continue;
    out.push({ rel, text, rust, scriptKind: scriptKindOf(rel) });
  }
  return out;
}

/** The permanent-deletion references in one corpus source, by its language. */
function deleteCallsIn({ text, rust, scriptKind }) {
  return rust ? dangerousRustFsDeleteCalls(text) : dangerousFsDeleteCalls(text, scriptKind);
}

/** Every corpus source file under `repoRoot` (minus `EXCLUDED_FILES`) with a real permanent-deletion call, as `path:line`. */
function findFsDeleteCallers(repoRoot) {
  const offenders = [];
  for (const source of corpusSources(repoRoot)) {
    const { rel } = source;
    if (EXCLUDED_FILES.has(rel)) continue;
    for (const call of deleteCallsIn(source)) {
      offenders.push(`${rel}:${call.line}`);
    }
  }
  return offenders;
}

test("dangerousFsDeleteCalls matches every banned member, bare-imported, aliased, or through a namespace/promises binding", () => {
  const text = [
    'import { rmSync } from "node:fs";',
    'import { unlinkSync as del } from "fs";',
    'import * as fs from "node:fs";',
    'import fsPromises from "node:fs/promises";',
    "rmSync(a);",
    "del(b);",
    "fs.rmdirSync(c);",
    "fs.promises.rm(d);",
    "fsPromises.unlink(e);",
  ].join("\n");
  expect(dangerousFsDeleteCalls(text)).toEqual([
    { line: 5, name: "rmSync" },
    { line: 6, name: "unlinkSync" },
    { line: 7, name: "rmdirSync" },
    { line: 8, name: "rm" },
    { line: 9, name: "unlink" },
  ]);
});

test("dangerousFsDeleteCalls matches CommonJS require, bare and destructured, including an inline require chain", () => {
  const text = [
    'const fs = require("fs");',
    'const { rmSync: nuke } = require("node:fs");',
    "fs.unlinkSync(a);",
    "nuke(b);",
    'require("fs/promises").rm(c);',
  ].join("\n");
  expect(dangerousFsDeleteCalls(text)).toEqual([
    { line: 3, name: "unlinkSync" },
    { line: 4, name: "rmSync" },
    { line: 5, name: "rm" },
  ]);
});

test("dangerousFsDeleteCalls does not flag an unrelated local function or module sharing a banned name", () => {
  const text = [
    'import { readFileSync } from "node:fs";',
    "function rm(x) { return x; }",
    "rm(readFileSync);",
    'const rmdir = "/tmp/scratch";',
    "console.log(rmdir);",
  ].join("\n");
  expect(dangerousFsDeleteCalls(text)).toEqual([]);
});

test("a banned call spelled inside a string literal or a comment is not a call", () => {
  const text = [
    'const specimen = \'rmSync("/tmp/x", { recursive: true })\';',
    '// fs.rmSync("/tmp/x")',
    "/* unlinkSync(path) */",
  ].join("\n");
  expect(dangerousFsDeleteCalls(text)).toEqual([]);
});

test("no source file in the repository corpus calls a node:fs permanent-deletion function outside the documented exceptions", () => {
  const offenders = findFsDeleteCallers(REPO_ROOT);
  expect(offenders).toEqual([]);
});

test("a banned call in an untracked file is seen, and one in an ignored file is not", () => {
  const banned = 'import { rmSync } from "node:fs";\nrmSync("x");\n';
  const dir = makeRepo(
    { ".gitignore": "ignored.mjs\n", "tracked.mjs": "export const a = 1;\n", "stray.mjs": banned, "ignored.mjs": banned },
    [".gitignore", "tracked.mjs"],
  );
  expect(findFsDeleteCallers(dir)).toEqual(["stray.mjs:2"]);
});

/**
 * The members of `excluded` that no longer justify an exemption: a path that is not a TRACKED
 * source naming a banned member, or one whose file carries no real permanent-deletion call. It
 * reads the index only, so an untracked file cannot satisfy an exemption that CI would not see.
 */
function staleExclusions(repoRoot, excluded) {
  const byPath = new Map(corpusSources(repoRoot, "tracked").map((s) => [s.rel, s]));
  const stale = [];
  for (const rel of excluded) {
    const source = byPath.get(rel);
    if (source === undefined) stale.push(`${rel} is excluded but is not a tracked source naming a banned member`);
    else if (deleteCallsIn(source).length === 0) {
      stale.push(`${rel} is excluded but carries no real permanent-deletion call`);
    }
  }
  return stale;
}

test("every named exclusion still names a tracked file that carries a real permanent-deletion call", () => {
  // An exclusion whose file stops calling one of these, or stops existing, is a stale exemption
  // that reads as a documented exception while exempting nothing — surfaced here rather than kept.
  expect(staleExclusions(REPO_ROOT, EXCLUDED_FILES)).toEqual([]);
});

test("an untracked file cannot satisfy a named exclusion", () => {
  const banned = 'import { rmSync } from "node:fs";\nrmSync("x");\n';
  const dir = makeRepo({ "kept.mjs": banned, "stray.mjs": banned }, ["kept.mjs"]);
  expect(staleExclusions(dir, ["kept.mjs"])).toEqual([]);
  expect(staleExclusions(dir, ["stray.mjs"])).toEqual([
    "stray.mjs is excluded but is not a tracked source naming a banned member",
  ]);
});

test("dangerousRustFsDeleteCalls flags a fully qualified call", () => {
  const text = 'fn f() { std::fs::remove_file("/tmp/x").unwrap(); }';
  expect(dangerousRustFsDeleteCalls(text)).toEqual([{ line: 1, name: "remove_file", kind: "call" }]);
});

test("dangerousRustFsDeleteCalls flags a bare call after a use import, on both the import and the call", () => {
  const text = ["use std::fs::remove_file;", "fn f() {", '    remove_file("/tmp/x").unwrap();', "}"].join("\n");
  expect(dangerousRustFsDeleteCalls(text)).toEqual([
    { line: 1, name: "remove_file", kind: "use" },
    { line: 3, name: "remove_file", kind: "call" },
  ]);
});

test("dangerousRustFsDeleteCalls flags an aliased import at the import, though the call site spells another name", () => {
  const text = ["use std::fs::remove_file as zap;", "fn f() {", '    zap("/tmp/x").unwrap();', "}"].join("\n");
  expect(dangerousRustFsDeleteCalls(text)).toEqual([{ line: 1, name: "remove_file", kind: "use" }]);
});

test("dangerousRustFsDeleteCalls flags a grouped alias import with no call anywhere, single line or multi-line", () => {
  expect(dangerousRustFsDeleteCalls("use tokio::fs::{remove_dir_all as rmrf};")).toEqual([
    { line: 1, name: "remove_dir_all", kind: "use" },
  ]);
  const multi = ["use tokio::fs::{", "    metadata,", "    remove_dir_all as rmrf,", "};"].join("\n");
  expect(dangerousRustFsDeleteCalls(multi)).toEqual([{ line: 3, name: "remove_dir_all", kind: "use" }]);
});

test("dangerousRustFsDeleteCalls names each identifier once: remove_dir never matches inside remove_dir_all", () => {
  const text = ["remove_dir_all(a);", "remove_dir(b);", "remove_file(c);"].join("\n");
  expect(dangerousRustFsDeleteCalls(text)).toEqual([
    { line: 1, name: "remove_dir_all", kind: "call" },
    { line: 2, name: "remove_dir", kind: "call" },
    { line: 3, name: "remove_file", kind: "call" },
  ]);
});

test("dangerousRustFsDeleteCalls ignores a function definition but not a call to it", () => {
  const text = ["fn remove_file(p: &Path) {}", "fn g() { remove_file(p); }"].join("\n");
  expect(dangerousRustFsDeleteCalls(text)).toEqual([{ line: 2, name: "remove_file", kind: "call" }]);
});

test("dangerousRustFsDeleteCalls does not read a call spelled in a Rust comment", () => {
  const text = [
    '// std::fs::remove_file("/tmp/x")',
    "/// `remove_dir_all(p)` is banned",
    "/* remove_dir(x) */",
    "fn f() {} // remove_file(y)",
    "/*",
    " remove_file(z)",
    "*/",
  ].join("\n");
  expect(dangerousRustFsDeleteCalls(text)).toEqual([]);
});

test("dangerousRustFsDeleteCalls sees a call after a raw string whose backslash is data, not an escape", () => {
  // An ordinary-string lexer reads the `\"` as an escape and swallows the rest of the line, hiding
  // the call; the raw-string rule closes the literal at its quote.
  const text = 'let s = r"a\\"; std::fs::remove_file(p);';
  expect(dangerousRustFsDeleteCalls(text)).toEqual([{ line: 1, name: "remove_file", kind: "call" }]);
});

test("the trash module's own removal is found by the detector but exempt at the corpus layer", () => {
  const body = 'fn remove_path(p: &Path) -> io::Result<()> { std::fs::remove_file(p) }\n';
  expect(dangerousRustFsDeleteCalls(body)).toEqual([{ line: 1, name: "remove_file", kind: "call" }]);
  const dir = makeRepo({
    "src/server/src/trash.rs": body,
    "src/server/src/other.rs": body,
  });
  expect(findFsDeleteCallers(dir)).toEqual(["src/server/src/other.rs:1"]);
});
