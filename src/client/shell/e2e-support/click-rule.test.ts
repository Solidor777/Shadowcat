// @vitest-environment node
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { runGit } from "../../../../scripts/lib/run-git.mjs";

// The e2e click rule: a spec may run without a real window only if it drives the UI with clicks the
// browser hit-tests at coordinates (`locator.click()`: on a covered element it times out and no
// handler fires; on a free element it sends a trusted event at the element's centre), so the UI is
// proven navigable. A bypass delivers the event without that hit test, so it proves nothing about
// navigability. The bypasses are `dispatchEvent`, `force: true`, and an `evaluate` family call that
// runs `.click()` or `dispatchEvent` inside the page.

/** One bypass found in a file. */
interface Bypass {
  /** What was found: `"dispatchEvent"`, `"force: true"` or `"evaluate .click()"`. */
  kind: string;
  /** The 1-based line of the match in the original source. */
  line: number;
}

/** Replaces every comment in `source` with spaces (newlines kept), so a comment that names a bypass
 * is not a finding and every offset still addresses the original text. Quotes and template
 * literals are tracked so a `//` inside a string is not read as a comment.
 * @param source The TypeScript source text.
 * @returns The same-length text with comments blanked.
 * @example
 * ```ts
 * blankComments("a(); // dispatchEvent"); // => "a();               "
 * ```
 */
function blankComments(source: string): string {
  const out = source.split("");
  let i = 0;
  while (i < source.length) {
    const c = source[i];
    const next = source[i + 1];
    if (c === "/" && next === "/") {
      while (i < source.length && source[i] !== "\n") out[i++] = " ";
    } else if (c === "/" && next === "*") {
      while (i < source.length && !(source[i] === "*" && source[i + 1] === "/")) {
        if (source[i] !== "\n") out[i] = " ";
        i += 1;
      }
      if (i < source.length) {
        out[i] = " ";
        out[i + 1] = " ";
        i += 2;
      }
    } else if (c === '"' || c === "'" || c === "`") {
      i += 1;
      while (i < source.length && source[i] !== c) i += source[i] === "\\" ? 2 : 1;
      i += 1;
    } else {
      i += 1;
    }
  }
  return out.join("");
}

/** The text of the call whose opening parenthesis is at `open`, up to its matching closing one.
 * @param code Comment-blanked source.
 * @param open Index of the `(`.
 * @returns The arguments' text and the index where it starts.
 * @example
 * ```ts
 * callArguments("f(a(b), 'c)')", 1); // => { start: 2, text: "a(b), 'c)'" }
 * ```
 */
function callArguments(code: string, open: number): { start: number; text: string } {
  let depth = 0;
  let i = open;
  while (i < code.length) {
    const c = code[i];
    if (c === '"' || c === "'" || c === "`") {
      i += 1;
      while (i < code.length && code[i] !== c) i += code[i] === "\\" ? 2 : 1;
    } else if (c === "(") {
      depth += 1;
    } else if (c === ")") {
      depth -= 1;
      if (depth === 0) break;
    }
    i += 1;
  }
  return { start: open + 1, text: code.slice(open + 1, i) };
}

/** The 1-based line of offset `index` in `source`.
 * @param source The text.
 * @param index A character offset.
 * @returns The line number.
 * @example
 * ```ts
 * lineOf("a\nb", 2); // => 2
 * ```
 */
function lineOf(source: string, index: number): number {
  let line = 1;
  for (let i = 0; i < index; i += 1) if (source[i] === "\n") line += 1;
  return line;
}

/** Finds every click-rule bypass in a spec or e2e-support source.
 * @param source The TypeScript source text.
 * @returns The bypasses, in source order.
 * @example
 * ```ts
 * findBypasses("await el.dispatchEvent('click');"); // => [{ kind: "dispatchEvent", line: 1 }]
 * ```
 */
function findBypasses(source: string): Bypass[] {
  const code = blankComments(source);
  const found: Bypass[] = [];
  for (const m of code.matchAll(/\bdispatchEvent\b/g)) found.push({ kind: "dispatchEvent", line: lineOf(code, m.index) });
  for (const m of code.matchAll(/\bforce\s*:\s*true\b/g)) found.push({ kind: "force: true", line: lineOf(code, m.index) });
  for (const m of code.matchAll(/(?:\bevaluate\w*|\$\$?eval)\s*\(/g)) {
    const { start, text } = callArguments(code, m.index + m[0].length - 1);
    for (const click of text.matchAll(/\.\s*click\s*\(/g)) {
      found.push({ kind: "evaluate .click()", line: lineOf(code, start + click.index) });
    }
  }
  return found.sort((a, b) => a.line - b.line);
}

/** The tracked e2e specs and e2e-support sources the rule covers, as repository-relative paths:
 * every `.ts` file tracked under the shell package's `e2e/` and `e2e-support/` except `*.test.ts`
 * (unit tests of the support code, which hold bypass specimens as data).
 * @returns The paths.
 * @example
 * ```ts
 * trackedE2eSources(); // => ["src/client/shell/e2e-support/browser-ui.ts", ...]
 * ```
 */
function trackedE2eSources(): string[] {
  const listed = runGit(
    ["-C", REPO_ROOT, "ls-files", "--", "src/client/shell/e2e", "src/client/shell/e2e-support"],
    "the tracked e2e files",
  );
  if (!listed.ok) throw new Error(listed.message);
  return listed.stdout
    .split(/\r?\n/)
    .filter((p) => p.endsWith(".ts") && !p.endsWith(".test.ts"))
    .sort();
}

/** The repository root, two package levels above this file's package. */
const REPO_ROOT = fileURLToPath(new URL("../../../../", import.meta.url));

describe("e2e click rule", () => {
  it("fires on each bypass shape: dispatchEvent, force: true, and a page-side click inside evaluate", () => {
    expect(findBypasses("await page.locator('a').dispatchEvent('click');")).toEqual([{ kind: "dispatchEvent", line: 1 }]);
    expect(findBypasses("\nawait page.locator('a').click({ force: true });")).toEqual([{ kind: "force: true", line: 2 }]);
    expect(findBypasses("x();\ny();\nawait page.evaluate(() => document.querySelector('a')?.click());")).toEqual([
      { kind: "evaluate .click()", line: 3 },
    ]);
    expect(findBypasses("await page.$$eval('a', (els) => els.forEach((e) => e.click()));").map((b) => b.kind)).toEqual([
      "evaluate .click()",
    ]);
    expect(findBypasses("await page.evaluate(\"document.body.dispatchEvent(new Event('x'))\");").map((b) => b.kind)).toEqual([
      "dispatchEvent",
    ]);
  });

  it("passes a hit-tested click, a click outside evaluate, an evaluate that reads, and comments that name a bypass", () => {
    expect(findBypasses("await page.locator('a').click();\nawait handle.click();")).toEqual([]);
    expect(findBypasses("await page.evaluate(() => document.title);\nawait page.locator('a').click();")).toEqual([]);
    expect(findBypasses("// never dispatchEvent or force: true\n/* page.evaluate(() => e.click()) */\nawait a.click();")).toEqual([]);
    expect(findBypasses("const url = 'http://x'; await a.click({ force: false });")).toEqual([]);
  });

  it("enumerates the tracked specs and support files from git", () => {
    const files = trackedE2eSources();
    expect(files.length).toBeGreaterThan(30);
    expect(files).toContain("src/client/shell/e2e/https-upgrade-chromium.spec.ts");
    expect(files).toContain("src/client/shell/e2e-support/browser-ui.ts");
    expect(files.some((p) => p.endsWith(".test.ts"))).toBe(false);
  });

  it("finds no bypass in any tracked e2e spec or support file", () => {
    const report: string[] = [];
    for (const file of trackedE2eSources()) {
      for (const b of findBypasses(readFileSync(join(REPO_ROOT, file), "utf8"))) report.push(`${file}:${b.line} ${b.kind}`);
    }
    expect(report, "bypasses of the browser's hit test").toEqual([]);
  });
});
