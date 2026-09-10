import { test, expect } from "vitest";
import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { runGit } from "./run-git.mjs";
import { norm } from "./gate-corpus.mjs";
import { execFileSync } from "node:child_process";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

// This repo permits no permanent-deletion call (owner ruling): `rmSync`/`rmdirSync`/`unlinkSync`
// and their bare async/callback and `node:fs/promises` counterparts all destroy a path immediately
// and irrecoverably, whether the target is tracked source, a debug dump, or a directory a test
// itself created — a test fixture gets no exemption, because the rule is about the API called, not
// about who owns the path it is called on. Every deletion need is served by `trash` (see
// `removeRecoverably` in `clean-build-outputs.mjs`) or by a fixture rewritten in place at one fixed
// path instead of a fresh one per run (`check-skill-symbol-refs-cli.test.mjs`'s own precedent).
//
// Same shape as `run-git.test.mjs`'s GIT-CALL-BUDGET check: enumerated via `git ls-files` rather
// than a directory walk, so an entry point living anywhere in the tracked tree is in scope by
// construction rather than by someone remembering to widen a root.

const FS_MODULES = new Set(["fs", "node:fs", "fs/promises", "node:fs/promises"]);
// The permanent-deletion API family: matched on the MEMBER name, since `rm`/`unlink`/`rmdir` are
// exported identically (same name, no `Sync` suffix) from both "fs" (async callback form) and
// "fs/promises" (promise form) — banning the name bans both shapes at once.
const BANNED_NAMES = new Set(["rmSync", "rm", "unlinkSync", "unlink", "rmdirSync", "rmdir"]);
// Cheap pre-filter so a file naming none of these identifiers is never even parsed.
const MENTIONS_BANNED_NAME = new RegExp(`\\b(${[...BANNED_NAMES].join("|")})\\b`);

const SCANNED_EXTENSIONS = [".js", ".mjs", ".cjs", ".ts", ".mts", ".cts"];

// Per-file exceptions, each named with its reason — never a pattern. Empty: every call site found
// on this branch was converted, and the one place `assemble-docs.mjs` deletes a whole build output
// tree already routes through `trash` via `removeRecoverably`, so it never reaches this list.
const EXCLUDED_FILES = new Set([]);

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

/** The parser mode for `rel`: TypeScript for a `.ts`/`.mts`/`.cts` file, JavaScript otherwise. */
function scriptKindOf(rel) {
  return /\.[cm]?ts$/.test(rel) ? ts.ScriptKind.TS : ts.ScriptKind.JS;
}

/** Every path `git ls-files` reports under `repoRoot`, NUL-separated and unfiltered. */
function listTrackedFiles(repoRoot) {
  const execFile = (cmd, args, opts) =>
    execFileSync(cmd, args, { ...opts, cwd: repoRoot, maxBuffer: 64 * 1024 * 1024 });
  const result = runGit(["ls-files", "-z"], "the tracked file list", { execFile });
  if (!result.ok) throw new Error(result.message);
  return result.stdout.split("\0").filter(Boolean);
}

/**
 * Every git-tracked file under `repoRoot` with a scanned extension that can name a banned member at
 * all, as `{ rel, text, scriptKind }`. A file mentioning none of `BANNED_NAMES` cannot contain a
 * matching call, so it is dropped before the parse.
 */
function trackedSources(repoRoot) {
  const out = [];
  for (const entry of listTrackedFiles(repoRoot)) {
    const rel = norm(entry);
    if (!SCANNED_EXTENSIONS.some((ext) => rel.endsWith(ext))) continue;
    const text = readFileSync(resolve(repoRoot, entry), "utf8");
    if (!MENTIONS_BANNED_NAME.test(text)) continue;
    out.push({ rel, text, scriptKind: scriptKindOf(rel) });
  }
  return out;
}

/** Every tracked source file under `repoRoot` (minus `EXCLUDED_FILES`) with a real permanent-deletion call, as `path:line`. */
function findFsDeleteCallers(repoRoot) {
  const offenders = [];
  for (const { rel, text, scriptKind } of trackedSources(repoRoot)) {
    if (EXCLUDED_FILES.has(rel)) continue;
    for (const call of dangerousFsDeleteCalls(text, scriptKind)) {
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

test("no tracked source file calls a node:fs permanent-deletion function outside the documented exceptions", () => {
  const offenders = findFsDeleteCallers(REPO_ROOT);
  expect(offenders).toEqual([]);
});

test("every named exclusion still names a tracked file that carries a real permanent-deletion call", () => {
  // An exclusion whose file stops calling one of these, or stops existing, is a stale exemption
  // that reads as a documented exception while exempting nothing — surfaced here rather than kept.
  const byPath = new Map(trackedSources(REPO_ROOT).map((s) => [s.rel, s]));
  for (const rel of EXCLUDED_FILES) {
    const source = byPath.get(rel);
    expect(source, `${rel} is excluded but is not a tracked source naming a banned member`).toBeDefined();
    expect(
      dangerousFsDeleteCalls(source.text, source.scriptKind).length,
      `${rel} is excluded but carries no real permanent-deletion call`,
    ).toBeGreaterThan(0);
  }
});
