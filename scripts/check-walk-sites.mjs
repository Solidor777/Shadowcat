// Fails when a directory-walking call exists in the repository that the inventory does not
// classify, or when the inventory records a site that no longer exists.
//
// Every gate that reads files must mean the same thing by "the repository", and the way to keep
// that true is one file list (`repoFiles`). A private directory walk is a second opinion about what
// the repository holds, so each remaining walk call is recorded in `walk-sites.json` with the reason
// it is not a repository-corpus read. A converted walk is gone from the tree and therefore gone from
// the inventory: the inventory records only what exists, so a stale entry fails as loudly as a new,
// unrecorded walk.
//
// Enumeration: the TypeScript parser over every `.mjs .js .cjs .ts` file in the corpus
// (`REPO_ROOTS` plus the repository-root files). A site is a call whose callee is one of
// `WALK_CALLEES`, spelled as a bare identifier, as an import alias or destructuring alias of one,
// or as a property of any object (`fs.readdirSync`, `fsp.readdir`, `fs.promises.readdir`). Site id:
// `file :: nearest named function :: callee :: ordinal`.
//
// Stated bound: the scan does NOT see
//   - a call through a computed property (`fs["readdirSync"]`);
//   - a callee re-exported under another name from a different module;
//   - a callee aliased through a local variable (`const rd = fs.readdirSync; rd(dir)`): only import
//     and destructuring aliases are followed, and the text recount matches the callee's own name
//     followed by `(`, so it misses the alias call too;
//   - a call into a third-party glob or walk library (`fast-glob`, `globby`, `tinyglobby`,
//     `walkdir`) whose function is not named in `WALK_CALLEES`;
//   - a `child_process` `find`;
//   - any file whose extension is outside `SCAN_EXTS`, for example `.mts`, `.cts`, `.tsx` and
//     `.svelte` (a walk in a `.svelte` script block is unread);
//   - any directory outside `REPO_ROOTS` and the repository-root files, for example `vendor/`,
//     `docs/` and `.claude/`.
// The test compares the parser's count with an independent text recount, so a form the parser
// misses and the text sees, inside a scanned file, fails loudly; the forms above are invisible to
// both.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { REPO_ROOTS, repoFiles } from "./lib/gate-corpus.mjs";
import { isDirectEntry } from "./lib/is-main.mjs";

/** Names that enumerate a directory or a glob; a call to any of them is a walk site. */
export const WALK_CALLEES = new Set([
  "readdir",
  "readdirSync",
  "opendir",
  "opendirSync",
  "glob",
  "globSync",
  "scandir",
]);

/** File name suffixes the enumeration parses. */
export const SCAN_EXTS = [".mjs", ".js", ".cjs", ".ts"];

/** Path of the inventory file, beside this script. */
export const INVENTORY_PATH = join(dirname(fileURLToPath(import.meta.url)), "walk-sites.json");

/** The only class an inventory entry may carry. */
const NOT_CORPUS = "not-corpus";

/**
 * Local names that stand for a walk callee through an import alias or a destructuring alias.
 * @param {ts.SourceFile} source - parsed file.
 * @returns {Map<string, string>} local name to the walk callee it stands for.
 * @example
 * collectAliases(ts.createSourceFile("a.mjs", 'import { readdirSync as rd } from "node:fs";', ts.ScriptTarget.Latest, true));
 */
export function collectAliases(source) {
  const aliases = new Map();
  const visit = (node) => {
    if (ts.isImportSpecifier(node) && node.propertyName && WALK_CALLEES.has(node.propertyName.text)) {
      aliases.set(node.name.text, node.propertyName.text);
    }
    if (
      ts.isBindingElement(node) &&
      node.propertyName &&
      ts.isIdentifier(node.propertyName) &&
      ts.isIdentifier(node.name) &&
      WALK_CALLEES.has(node.propertyName.text)
    ) {
      aliases.set(node.name.text, node.propertyName.text);
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return aliases;
}

/**
 * The walk callee a call expression invokes, or null when it is not a walk call.
 * @param {ts.CallExpression} call - the call to classify.
 * @param {Map<string, string>} aliases - local names standing for a walk callee.
 * @returns {string|null} the canonical callee name.
 * @example
 * calleeOf(call, new Map());
 */
export function calleeOf(call, aliases) {
  const e = call.expression;
  if (ts.isIdentifier(e)) return aliases.get(e.text) ?? (WALK_CALLEES.has(e.text) ? e.text : null);
  if (ts.isPropertyAccessExpression(e) && WALK_CALLEES.has(e.name.text)) return e.name.text;
  return null;
}

/**
 * The nearest enclosing function that has a name; a `const f = () => ...` arrow counts as `f`.
 * @param {ts.Node} node - the node whose enclosing function is wanted.
 * @returns {string} the function name, or `<module>` at module scope.
 * @example
 * enclosingName(call);
 */
export function enclosingName(node) {
  for (let n = node.parent; n; n = n.parent) {
    if ((ts.isFunctionDeclaration(n) || ts.isMethodDeclaration(n)) && n.name) return n.name.getText();
    if (ts.isArrowFunction(n) || ts.isFunctionExpression(n)) {
      if (n.name) return n.name.text;
      const p = n.parent;
      if (ts.isVariableDeclaration(p) && ts.isIdentifier(p.name)) return p.name.text;
      if (ts.isPropertyAssignment(p) && p.name) return p.name.getText();
    }
  }
  return "<module>";
}

/**
 * Every walk call site in one file's text, in source order.
 * @param {string} text - the file contents.
 * @param {string} file - the repo-relative path, used in the id and to pick the parser mode.
 * @returns {string[]} site ids, `file :: function :: callee :: ordinal`.
 * @example
 * findWalkSites('import { readdirSync } from "node:fs"; export function f() { return readdirSync("."); }', "a.mjs");
 */
export function findWalkSites(text, file) {
  const kind = file.endsWith(".ts") ? ts.ScriptKind.TS : ts.ScriptKind.JS;
  const source = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, kind);
  const aliases = collectAliases(source);
  const hits = [];
  const visit = (node) => {
    if (ts.isCallExpression(node)) {
      const callee = calleeOf(node, aliases);
      if (callee !== null) hits.push({ pos: node.getStart(source), fn: enclosingName(node), callee });
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  hits.sort((a, b) => a.pos - b.pos);
  const seen = new Map();
  return hits.map(({ fn, callee }) => {
    const key = `${fn} :: ${callee}`;
    const ordinal = (seen.get(key) ?? 0) + 1;
    seen.set(key, ordinal);
    return `${file} :: ${key} :: ${ordinal}`;
  });
}

/**
 * Every walk site across `files`.
 * @param {string[]} files - repo-relative paths.
 * @param {(path: string) => string} [read] - file reader, injectable for tests.
 * @returns {string[]} site ids in file order.
 * @example
 * collectSites(["scripts/clean-build-outputs.mjs"]);
 */
export function collectSites(files, read = (p) => readFileSync(p, "utf8")) {
  return files.flatMap((f) => findWalkSites(read(f), f));
}

/**
 * A text recount of walk-call spellings that is independent of the parser: lines that are not an
 * import, an `export ... from`, or a comment, matched by name followed by an open parenthesis.
 * @param {string} text - the file contents.
 * @returns {number} the number of matches.
 * @example
 * recountWalkCalls('const x = readdirSync(".");');
 */
export function recountWalkCalls(text) {
  const call = new RegExp(`\\b(${[...WALK_CALLEES].join("|")})\\s*\\(`, "g");
  let total = 0;
  for (const line of text.split("\n")) {
    const t = line.trim();
    if (/^(import\b|export\b[^=(]*\bfrom\b|\/\/|\/\*|\*)/.test(t)) continue;
    total += (line.match(call) ?? []).length;
  }
  return total;
}

/**
 * Compares the sites found in the tree with the inventory.
 * @param {string[]} found - site ids found by the enumeration.
 * @param {{ id: string, class: string, reason: string }[]} recorded - inventory entries.
 * @returns {string[]} one message per violation; empty when the inventory is exact.
 * @example
 * reconcile(["a.mjs :: f :: readdirSync :: 1"], []);
 */
export function reconcile(found, recorded) {
  const errors = [];
  const recordedIds = new Set(recorded.map((r) => r.id));
  const foundIds = new Set(found);
  for (const id of found) {
    if (!recordedIds.has(id)) errors.push(`unclassified walk site: ${id}`);
  }
  for (const id of recordedIds) {
    if (!foundIds.has(id)) errors.push(`recorded walk site no longer exists: ${id}`);
  }
  for (const r of recorded) {
    if (r.class !== NOT_CORPUS) errors.push(`${r.id}: class must be "${NOT_CORPUS}", got "${r.class}"`);
    if (typeof r.reason !== "string" || r.reason.trim() === "") errors.push(`${r.id}: missing reason`);
  }
  if (found.length !== recorded.length) {
    errors.push(`count mismatch: found ${found.length}, recorded ${recorded.length}`);
  }
  return errors;
}

if (isDirectEntry(import.meta.url)) {
  let files;
  try {
    files = repoFiles({
      roots: REPO_ROOTS,
      rootFiles: true,
      exts: SCAN_EXTS,
      label: "lint:walk-sites",
    });
  } catch (e) {
    console.error(`lint:walk-sites: ${e.message}`);
    process.exit(2);
  }
  const found = collectSites(files);
  const recorded = JSON.parse(readFileSync(INVENTORY_PATH, "utf8")).sites;
  const errors = reconcile(found, recorded);
  console.log(`walk sites: found ${found.length}, recorded ${recorded.length}`);
  if (errors.length > 0) {
    for (const message of errors) console.error(`lint:walk-sites: ${message}`);
    console.error(
      "lint:walk-sites: read files through repoFiles (lib/gate-corpus.mjs); record a walk in " +
        "scripts/walk-sites.json only when it is not a repository-corpus read, with its reason.",
    );
    process.exit(1);
  }
}
