// Fails when a Rust `mod` item carries an outer doc comment (`///`, `/** */` or `#[doc ...]`).
//
// A module is documented only by the `//!` block inside its own file or braces. The outer text on a
// `mod x;` declaration is merged with that `//!` block by rustdoc and the merged text resolves
// relative intra-doc links in the PARENT's scope, so a link that is correct as written inside the
// module breaks as soon as the module also carries an outer doc, and the same words end up written
// in two places that can disagree. The rule therefore lives on the declaration form: no `mod` item,
// file-backed or inline, has an outer doc.
//
// Telling code from comment is by line shape: a doc line sets a pending state that survives blank
// lines, plain `//` comments and further attributes (all of which leave the doc attached to the
// next item), and any other line clears it.

import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";
import { norm, under } from "./lib/gate-corpus.mjs";
import { runGit } from "./lib/run-git.mjs";

const OUTER_LINE_DOC = /^\s*\/\/\/(?!\/)/;
const OUTER_BLOCK_DOC_OPEN = /^\s*\/\*\*(?![*/])/;
const ATTR_START = /^\s*#\[/;
const DOC_ATTR = /^\s*#\[\s*doc\b/;
const PLAIN_COMMENT = /^\s*\/\/(?![/!])/;
const MOD_ITEM = /^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*[;{]/;

/** Whether `path` is a Rust source file the gate covers. */
export function isRustSource(path) {
  const p = norm(path);
  return under(p, "src") && p.endsWith(".rs");
}

/** Net `[` minus `]` on a line, so an attribute spanning lines is read to its closing bracket. */
function bracketDelta(line) {
  let d = 0;
  for (const ch of line) {
    if (ch === "[") d++;
    else if (ch === "]") d--;
  }
  return d;
}

/**
 * Every `mod` item whose declaration carries an outer doc.
 * @param text The Rust source text.
 * @returns One `{ line, module, inline }` per violation; `line` is the `mod` line, 1-based.
 */
export function scanModuleDocs(text) {
  const lines = text.split("\n").map((l) => l.replace(/\r$/, ""));
  const out = [];
  let pending = false;
  let inBlockDoc = false;
  let attrDepth = 0;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (inBlockDoc) {
      if (line.includes("*/")) {
        inBlockDoc = false;
        pending = true;
      }
      continue;
    }
    if (attrDepth > 0) {
      attrDepth += bracketDelta(line);
      continue;
    }
    if (OUTER_LINE_DOC.test(line)) {
      pending = true;
      continue;
    }
    if (OUTER_BLOCK_DOC_OPEN.test(line)) {
      if (line.includes("*/", line.indexOf("/**") + 3)) pending = true;
      else inBlockDoc = true;
      continue;
    }
    if (ATTR_START.test(line)) {
      if (DOC_ATTR.test(line)) pending = true;
      attrDepth = Math.max(0, bracketDelta(line));
      continue;
    }
    if (line.trim() === "" || PLAIN_COMMENT.test(line)) continue;
    const m = line.match(MOD_ITEM);
    if (m && pending) out.push({ line: i + 1, module: m[1], inline: line.includes("{") });
    pending = false;
  }
  return out;
}

/** Lists the tracked Rust sources through `runGit`, with a buffer sized for a repo-wide list. */
function listTrackedFiles() {
  const execFile = (cmd, args, opts) => execFileSync(cmd, args, { ...opts, maxBuffer: 64 * 1024 * 1024 });
  return runGit(["ls-files", "-z", "--", "src"], "the tracked source file list", { execFile });
}

/**
 * Runs the gate over the files `listFiles` reports and returns the process exit code.
 * @param io Injected effects: `listFiles` returns a `runGit`-shaped result, `readFile` reads a
 * path, `log`/`error` receive output lines.
 * @returns 0 when clean, 1 on a violation or an unlistable tree.
 */
export function runGate({ listFiles, readFile, log, error }) {
  const listed = listFiles();
  if (!listed.ok) {
    error(`lint:module-docs: ${listed.message}`);
    return 1;
  }
  const files = listed.stdout.split("\0").filter((f) => f && isRustSource(f));
  let errors = 0;
  for (const path of files) {
    for (const v of scanModuleDocs(readFile(path))) {
      errors++;
      error(
        `MODULE DOC ON A mod ITEM: ${norm(path)}:${v.line}: \`mod ${v.module}\` carries an outer doc — move the text into the ${v.inline ? "`//!` block inside the braces" : "`//!` block at the top of the module's own file"}. Rustdoc merges the outer doc with the module's inner docs and resolves the merged text in the parent's scope, so a relative intra-doc link in the module breaks, and the module is documented in two places.`,
      );
    }
  }
  log(`lint:module-docs: ${files.length} files scanned, ${errors} error(s)`);
  return errors === 0 ? 0 : 1;
}

if (isDirectEntry(import.meta.url)) {
  process.exit(runGate({ listFiles: listTrackedFiles, readFile: (p) => readFileSync(p, "utf8"), log: console.log, error: console.error }));
}
