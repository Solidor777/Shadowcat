// Fails when client code writes a same-origin server path as a bare string.
//
// The server can mount under a route prefix (`Config.route_prefix`, for a reverse proxy that
// serves it at a subpath). The page learns the prefix from `window.__SHADOWCAT_BASE__`, and
// `basePath`, `apiPath` and `wsUrl` in `@shadowcat/core` (`base-path.ts`) are the only code that
// reads it. A URL the client builds as a plain `/api/...`, `/modules/...` or `/ws` string skips
// the prefix, so it works at the root mount and breaks behind a subpath proxy while every
// root-mount test stays green.
//
// Detector: a string literal, a no-substitution template, or the head of a template expression
// whose text begins with `/api/`, `/modules/`, `/ws`, or is exactly `/api`, parsed with the
// TypeScript compiler so comments and doc examples are never read. The literal is allowed only as
// an argument expression inside an `apiPath(...)` call, which is the seam that applies the prefix,
// or inside a `new Error(...)` message, which names a request for a reader and never issues one.
// `.svelte` files are read through the `<script>` blocks and template expressions of
// `svelteRegions`, plus a text pass over template attributes (`src`, `href`, `action`, `poster`).
//
// BOUND (what no static pass can decide): a path assembled from pieces that individually carry no
// leading `/api/` (`"/" + "api"`, a variable holding the prefix-free path before it reaches
// `apiPath`). A caller that reaches `apiPath` through a variable passes the gate; the literal that
// seeds the variable still has to sit inside an `apiPath(...)` call to pass.
//
// Not covered: test files, `e2e` and `e2e-support` directories (they address the test server
// directly, with its own base URL), `vite.config.ts` (the dev proxy), and generated types.

import { readFileSync } from "node:fs";
import process from "node:process";
import ts from "typescript";
import { isDirectEntry } from "./lib/is-main.mjs";
import { CLIENT_ROOTS, norm, repoFiles, under } from "./lib/gate-corpus.mjs";
import { svelteRegions } from "./check-secure-context-apis.mjs";

/** The file that defines the seam; the scan must include it or the gate is inert. */
export const SEAM_FILE = "src/client/core/src/base-path.ts";

/** The function whose argument is allowed to hold a bare server path. */
const SEAM_CALL = "apiPath";

/** A literal text that names a server path: `/api`, `/api/...`, `/modules/...`, `/ws`, `/ws?...`. */
const SERVER_PATH = /^(?:\/api(?:[/?#]|$)|\/modules\/|\/ws(?:[/?#]|$))/;

/** A template attribute whose value is a server path: `src="/api/..."`. */
const ATTRIBUTE_PATH = /\b(?:src|href|action|poster|srcset)\s*=\s*["']\/(?:api|modules|ws)(?:[/?#"']|$)/;

const EXTS = [".ts", ".svelte", ".js", ".mjs"];

/** Path fragments of files the gate leaves out; see the header. */
const EXCLUDED = [".test.", ".spec.", "/e2e/", "/e2e-support/", "/__fixtures__/", "/vite.config.", "/vitest.setup."];

/** Whether `path` is a client source file the gate covers.
 * @param path A repo-relative path.
 * @returns True when the gate scans it.
 */
export function isCovered(path) {
  const p = norm(path);
  return (
    CLIENT_ROOTS.some((r) => under(p, r)) &&
    EXTS.some((e) => p.endsWith(e)) &&
    !under(p, "src/types/generated") &&
    !under(p, "src/server") &&
    !EXCLUDED.some((x) => p.includes(x)) &&
    !p.endsWith(".d.ts")
  );
}

/** Whether `node` is allowed to hold a bare server path: it sits inside the argument list of an
 * `apiPath(...)` call, or inside the message of a `new Error(...)` (text a reader sees, never a
 * URL the client requests). */
function insideSeamCall(node) {
  for (let p = node.parent; p; p = p.parent) {
    if (ts.isCallExpression(p) && ts.isIdentifier(p.expression) && p.expression.text === SEAM_CALL && p.arguments.some((a) => a.pos <= node.pos && node.end <= a.end)) return true;
    if (ts.isNewExpression(p) && ts.isIdentifier(p.expression) && p.expression.text === "Error") return true;
  }
  return false;
}

/** Spans of the bare server-path literals in TypeScript source.
 * @param code Source text.
 * @returns `{ start, end, line }` per offender; `start`/`end` bound the whole literal (the whole
 * template expression for a template with substitutions) and `line` is 1-based.
 */
export function literalSpans(code) {
  const sf = ts.createSourceFile("x.ts", code, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const out = [];
  const add = (node, whole) => {
    if (insideSeamCall(whole)) return;
    out.push({ start: whole.getStart(sf), end: whole.getEnd(), line: sf.getLineAndCharacterOfPosition(node.getStart(sf)).line + 1 });
  };
  const visit = (node) => {
    if ((ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) && SERVER_PATH.test(node.text)) {
      if (!ts.isImportDeclaration(node.parent) && !ts.isExportDeclaration(node.parent) && !ts.isLiteralTypeNode(node.parent)) add(node, node);
    } else if (ts.isTemplateExpression(node) && SERVER_PATH.test(node.head.text)) {
      add(node.head, node);
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
  return out;
}

/** Every bare server path in one file.
 * @param path Repo-relative path, to pick the TypeScript or Svelte reader.
 * @param text File contents.
 * @returns `{ line }` per offender, sorted.
 */
export function scanRoutePrefix(path, text) {
  const p = norm(path);
  const found = [];
  if (p.endsWith(".svelte")) {
    for (const r of svelteRegions(text).regions) for (const s of literalSpans(r.code)) found.push({ line: s.line + r.lineOffset });
    const template = text.replace(/<(script|style)\b[^>]*>[\s\S]*?<\/\1>/g, (m) => m.replace(/[^\n]/g, " ")).replace(/<!--[\s\S]*?-->/g, (m) => m.replace(/[^\n]/g, " "));
    template.split("\n").forEach((l, i) => {
      if (ATTRIBUTE_PATH.test(l)) found.push({ line: i + 1 });
    });
  } else {
    for (const s of literalSpans(text)) found.push({ line: s.line });
  }
  return found.sort((a, b) => a.line - b.line);
}

/** Throws when the scanned set cannot support a clean verdict: empty, missing the seam file, or
 * holding no `.svelte` file.
 * @param files Repo-relative paths about to be scanned.
 */
export function assertGateLive(files) {
  if (files.length === 0) throw new Error("lint:route-prefix scanned 0 files; the gate is inert");
  const n = files.map(norm);
  if (!n.includes(SEAM_FILE)) throw new Error(`lint:route-prefix did not scan ${SEAM_FILE}; the gate is inert`);
  if (!n.some((f) => f.endsWith(".svelte"))) throw new Error("lint:route-prefix scanned no .svelte file; the Svelte path is inert");
}

/** Runs the gate over the files `listFiles` reports.
 * @param io Injected effects: `listFiles`, `readFile`, `log`, `error`.
 * @returns 0 when clean, 1 on a violation, 2 when the tree cannot be listed.
 */
export function runGate({ listFiles, readFile, log, error }) {
  let listed;
  try {
    listed = listFiles();
  } catch (e) {
    error(`lint:route-prefix: ${e.message}`);
    return 2;
  }
  const files = listed.filter((f) => f && isCovered(f));
  assertGateLive(files);
  let errors = 0;
  for (const path of files) {
    for (const v of scanRoutePrefix(path, readFile(path))) {
      errors++;
      error(`BARE SERVER PATH: ${norm(path)}:${v.line}: wrap the path in \`apiPath(...)\` (or build a WebSocket URL with \`wsUrl()\`) from @shadowcat/core so a route prefix applies`);
    }
  }
  log(`lint:route-prefix: ${files.length} files scanned, ${errors} error(s)`);
  return errors === 0 ? 0 : 1;
}

if (isDirectEntry(import.meta.url)) {
  const listFiles = () => repoFiles({ roots: CLIENT_ROOTS, label: "lint:route-prefix" });
  process.exit(runGate({ listFiles, readFile: (p) => readFileSync(p, "utf8"), log: console.log, error: console.error }));
}
