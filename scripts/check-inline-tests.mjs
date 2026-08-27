// Fails when a Rust test module body is written inline in a production source file.
//
// A `#[cfg(test)] mod x { … }` body is the single largest contributor to oversize files in this
// crate, and it is the one part of a file that can move without changing what the file exports:
// `mod x;` with the body in a sibling file resolves `use super::*` to the same parent, so nothing
// widens and nothing is lost. The declaration form is therefore the only allowed one.
//
// `#[cfg(test)]` on a non-module item (a helper fn, a field, an impl block) is a test-only
// declaration that the extracted tests reach through `super::`; it stays in the production file
// and is not a violation.
//
// Telling code from comment is shared with the other gates through comment-span.mjs so an
// attribute quoted in a doc comment is prose, not a match.

import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";
import { norm, under } from "./lib/gate-corpus.mjs";
import { splitLine } from "./lib/comment-span.mjs";

const ATTR = /^\s*#\[cfg\(test\)\]\s*$/;
const ANY_ATTR = /^\s*#\[/;
const MOD_BODY = /^\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{/;

/** Whether `path` is a Rust source file the gate covers. */
export function isRustSource(path) {
  const p = norm(path);
  return under(p, "src") && p.endsWith(".rs");
}

/** Every `#[cfg(test)]` attribute followed (past blanks, comments, other attributes) by a braced `mod`. */
export function scanInlineTests(text) {
  const lines = text.split("\n").map((l) => l.replace(/\r$/, ""));
  const code = [];
  let state = { inBlock: false, inHtml: false };
  for (const line of lines) {
    const r = splitLine(line, state);
    state = r.state;
    code.push(r.code);
  }
  const out = [];
  for (let i = 0; i < code.length; i++) {
    if (!ATTR.test(code[i])) continue;
    let j = i + 1;
    while (j < code.length && (code[j].trim() === "" || ANY_ATTR.test(code[j]))) j++;
    if (j >= code.length) continue;
    const m = code[j].match(MOD_BODY);
    if (m) out.push({ line: j + 1, module: m[1] });
  }
  return out;
}

function main() {
  const files = execFileSync("git", ["ls-files", "-z", "--", "src"], { encoding: "utf8" })
    .split("\0")
    .filter(isRustSource);
  let errors = 0;
  for (const path of files) {
    for (const v of scanInlineTests(readFileSync(path, "utf8"))) {
      errors++;
      console.error(`INLINE TEST MODULE: ${norm(path)}:${v.line}: mod ${v.module} { … } — move the body to a sibling file and declare it as \`#[cfg(test)] mod ${v.module};\``);
    }
  }
  console.log(`lint:inline-tests: ${files.length} files scanned, ${errors} error(s)`);
  process.exit(errors === 0 ? 0 : 1);
}

if (isDirectEntry(import.meta.url)) main();
