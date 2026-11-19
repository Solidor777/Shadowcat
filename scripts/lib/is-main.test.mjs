import { describe, it, expect, afterEach } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import process from "node:process";
import { splitLine } from "./comment-span.mjs";
import { repoFiles } from "./gate-corpus.mjs";
import { isDirectEntry } from "./is-main.mjs";

const SCRIPTS_DIR = resolve(fileURLToPath(import.meta.url), "..", "..");

// `argv[1]` is this function's whole input, so every case drives it directly. The value vitest was
// launched with is restored after each case because the runner reads it too.
const RUNNER_ARGV1 = process.argv[1];

/**
 * Every `.mjs` under `scripts/` that node can be invoked on directly.
 *
 * Test modules are excluded: a test file is never the process entry point — the runner is — and
 * these cases assign `argv[1]` themselves, which would make the pin below match its own source.
 *
 * The set is the repository corpus (`repoFiles`: tracked plus untracked, non-ignored files), so a
 * new script that reads `argv[1]` is caught before it is added to git. This is a BAN pin.
 *
 * @param {string} repoRoot - The repository root whose `scripts/` directory is listed.
 * @returns {string[]} Absolute paths, sorted so the pin's expectation is order-independent.
 * @example
 * const files = executableScripts(dirname(SCRIPTS_DIR));
 */
function executableScripts(repoRoot) {
  return repoFiles({ cwd: repoRoot, roots: ["scripts"], exts: [".mjs"], label: "argv[1] pin" })
    .filter((p) => !p.endsWith(".test.mjs"))
    .map((p) => join(repoRoot, ...p.split("/")))
    .sort();
}

/**
 * The code span of a source file, with comment text removed.
 *
 * A comment ABOUT the entry decision is prose, not a second copy of it — two gate files describe
 * the comparison in their header prose. The shared lexer draws the line so this pin and the
 * comment gate cannot disagree about the same source line.
 *
 * @param {string} source - Whole file contents.
 * @returns {string} The concatenated code spans, one line each.
 * @example
 * codeOf('const a = 1; // argv[1]'); // -> "const a = 1; \n"
 */
function codeOf(source) {
  let state = { inBlock: false, inHtml: false };
  let code = "";
  for (const line of source.split("\n")) {
    const split = splitLine(line, state);
    state = split.state;
    code += `${split.code}\n`;
  }
  return code;
}

describe("isDirectEntry", () => {
  afterEach(() => {
    process.argv[1] = RUNNER_ARGV1;
  });

  it("matches an argv[1] spelled as an absolute path", () => {
    const entry = resolve(process.cwd(), "gate-cli.mjs");
    process.argv[1] = entry;
    expect(isDirectEntry(pathToFileURL(entry).href)).toBe(true);
  });

  it("matches an argv[1] spelled relative to the working directory", () => {
    const entry = resolve(process.cwd(), "scripts", "gate-cli.mjs");
    process.argv[1] = join("scripts", "gate-cli.mjs");
    expect(isDirectEntry(pathToFileURL(entry).href)).toBe(true);
  });

  it("matches an argv[1] carrying an unnormalized parent segment", () => {
    const entry = resolve(process.cwd(), "scripts", "gate-cli.mjs");
    // Assembled with `Array.prototype.join` and the platform separator, never `path.join`:
    // `path.join` collapses `..` before it returns, so a path built with it arrives at `argv[1]`
    // already normalized and asks the same question as the relative case above. The segment
    // assertion below pins that premise, because an input that quietly loses its `..` makes this
    // case a duplicate of its sibling while both keep passing.
    process.argv[1] = ["scripts", "lib", "..", "gate-cli.mjs"].join(sep);
    expect(process.argv[1].split(sep)).toContain("..");
    expect(isDirectEntry(pathToFileURL(entry).href)).toBe(true);
  });

  it("rejects an argv[1] naming a sibling of the calling module", () => {
    const entry = resolve(process.cwd(), "scripts", "gate-cli.mjs");
    process.argv[1] = resolve(process.cwd(), "scripts", "other-cli.mjs");
    expect(isDirectEntry(pathToFileURL(entry).href)).toBe(false);
  });

  it("returns false rather than throwing when the process has no argv[1]", () => {
    const entry = resolve(process.cwd(), "scripts", "gate-cli.mjs");
    process.argv[1] = undefined;
    expect(isDirectEntry(pathToFileURL(entry).href)).toBe(false);
  });
});

describe("the entry-point decision", () => {
  it("is read from argv[1] in exactly one module across the scripts tree", () => {
    const readers = executableScripts(dirname(SCRIPTS_DIR))
      .filter((file) => codeOf(readFileSync(file, "utf8")).includes("argv[1]"))
      .map((file) => relative(SCRIPTS_DIR, file).split(sep).join("/"));
    expect(readers).toEqual(["lib/is-main.mjs"]);
  });
});
