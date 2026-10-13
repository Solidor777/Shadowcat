// Fails when a Rust `mod` item carries an outer doc comment (`///`, `/** */` or `#[doc ...]`).
//
// A module is documented only by the `//!` block inside its own file or braces. The outer text on a
// `mod x;` declaration is merged with that `//!` block by rustdoc and the merged text resolves
// relative intra-doc links in the PARENT's scope, so a link that is correct as written inside the
// module breaks as soon as the module also carries an outer doc, and the same words end up written
// in two places that can disagree. The rule therefore lives on the declaration form: no `mod` item,
// file-backed or inline, has an outer doc.
//
// Telling code from comment is shared with the other gates through comment-span.mjs: an outer doc
// is a comment SPAN that opens a line with `///` or `/**`; inside a block comment the span is the
// whole line, so a triple-slash there is prose, and a plain block comment between a doc and its
// item leaves the doc attached. A doc sets a pending state that survives blank lines, comments and
// further attributes; leading `#[...]` groups are stripped from a code line (a `#[doc = "..."]`
// group is itself a doc) and the remainder is tested for a `mod` item, which consumes the pending
// state whether or not it reports. Bound: no string state crosses a line, so a triple-slash line
// inside a multi-line raw string literal is read as a doc. Bound: `splitLine` does not model nested
// block comments, so the first `*/` closes the outermost `/*` even where rustc nests; text that
// rustc reads inside a nested comment is read as code after that first `*/`.

import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";
import { isRustSource, norm } from "./lib/gate-corpus.mjs";
import { splitLine } from "./lib/comment-span.mjs";
import { runGit } from "./lib/run-git.mjs";

const MOD_ITEM = /^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*[;{]/;
const DOC_ATTR_BODY = /^\s*doc\b/;

/**
 * Whether a line that opens with a comment opens an OUTER doc, by the rustc rule on the comment
 * text: `///` is an outer doc when the span after `//` begins with one more `/`, not two; `/**` is
 * an outer doc when the character after the opener is neither `*` nor `/`, and the end of the line
 * counts as a doc (so a multi-line `/**` opener alone on its line is one). An opener followed by a
 * star or a slash is a plain comment. A line that begins inside a plain block comment opens
 * nothing: the opener there is comment text.
 * @param line The source line.
 * @param comment The comment span `splitLine` returned for the line.
 * @param startsInBlock Whether the line begins inside a block comment.
 * @returns Whether the line opens an outer doc.
 */
function opensOuterDoc(line, comment, startsInBlock) {
  if (startsInBlock) return false;
  const lead = line.trimStart();
  if (lead.startsWith("//")) return comment.startsWith("/") && !comment.startsWith("//");
  if (lead.startsWith("/**")) return !/^[*/]/.test(lead.slice(3));
  return false;
}

/**
 * Strips leading `#[...]` groups from a code span, reading a group across lines through `depth`.
 * @param code The code span of one line (comments already removed).
 * @param depth Bracket depth carried in from an attribute still open at the previous line's end.
 * @returns `{ rest, depth, doc }`: the code after the groups, the depth open at the line's end, and
 * whether a `#[doc ...]` group started on this line.
 */
function stripAttributes(code, depth) {
  let i = 0;
  let doc = false;
  for (;;) {
    if (depth === 0) {
      while (i < code.length && /\s/.test(code[i])) i++;
      if (!code.startsWith("#[", i)) break;
      if (DOC_ATTR_BODY.test(code.slice(i + 2))) doc = true;
      depth = 1;
      i += 2;
    }
    while (i < code.length && depth > 0) {
      const ch = code[i];
      if (ch === '"') {
        i++;
        while (i < code.length && code[i] !== '"') i += code[i] === "\\" ? 2 : 1;
      } else if (ch === "[") depth++;
      else if (ch === "]") depth--;
      i++;
    }
    if (depth > 0) return { rest: "", depth, doc };
  }
  return { rest: code.slice(i), depth, doc };
}

/**
 * Every `mod` item whose declaration carries an outer doc.
 * @param text The Rust source text.
 * @returns One `{ line, module, inline }` per violation; `line` is the `mod` line, 1-based.
 */
export function scanModuleDocs(text) {
  const lines = text.split("\n").map((l) => l.replace(/\r$/, ""));
  const out = [];
  let state = { inBlock: false, inHtml: false };
  let pending = false;
  let docBlock = false;
  let attrDepth = 0;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const startsInBlock = state.inBlock;
    const split = splitLine(line, state);
    state = split.state;
    if (attrDepth === 0 && opensOuterDoc(line, split.comment, startsInBlock)) {
      if (line.trimStart().startsWith("//")) pending = true;
      else docBlock = true;
    }
    if (docBlock && !state.inBlock) {
      docBlock = false;
      pending = true;
    }
    const attrs = stripAttributes(split.code, attrDepth);
    attrDepth = attrs.depth;
    if (attrs.doc) pending = true;
    if (attrs.rest.trim() === "") continue;
    const m = attrs.rest.match(MOD_ITEM);
    if (m && pending) out.push({ line: i + 1, module: m[1], inline: attrs.rest.includes("{") });
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
