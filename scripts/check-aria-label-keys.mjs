// Fails when a human-facing markup attribute carries an i18n KEY instead of translated text.
//
// `aria-label` OVERRIDES a control's visible label for assistive technology, so a literal
// `aria-label="gameSettings.scene.gridSize"` beside a visible `{t("gameSettings.scene.gridSize")}`
// is strictly worse than no attribute at all: a sighted user reads "Grid cell size (pixels)" while a
// screen reader announces the raw key. The same holds for every attribute whose value is read to a
// person — `title`, `placeholder` and the `aria-*` text attributes — which is why the covered set is
// the ATTRIBUTE FAMILY and not one attribute.
//
// A key is recognised by its SHAPE rather than by a catalog lookup: a lowercase word, a dot, and no
// whitespace anywhere in the value. Human text with a dot carries a space somewhere ("Reset to
// default."), so the shape separates the two without reading the locale catalog — a key that is
// missing from the catalog is still a key, and that is the case a lookup would have waved through.
// Over-scanning is the deliberate failure direction: a bare version label in a title is a visible
// false positive, whereas a missed key is announced to a user and never seen by a reviewer.
//
// Three writers of the same defect are matched, because the second and third are what the first
// turns into once someone needs a per-row identifier in the name:
//   aria-label="a.b"                       a literal key
//   aria-label="a.b.{row.id}"              a literal key with a template interpolation
//   aria-label={"a.b." + row.id}           an expression whose FIRST token is a key literal
//   aria-label={`a.b.${row.id}`}           the template-literal spelling of the row above
// An expression that begins with a call (`{t("a.b")}`) or an identifier is not matched: the gate
// cannot see what a variable holds, so `aria-label={labelKey}` where `labelKey` is a raw key stays
// a review obligation, not a claim this gate makes.
//
// A match inside a markup comment is prose; telling code from comment is shared with the other
// gates through comment-span.mjs rather than reimplemented here.

import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";
import { norm, under } from "./lib/gate-corpus.mjs";
import { splitLine } from "./lib/comment-span.mjs";

/** Repository roots whose tracked component files the gate reads. */
export const ROOTS = ["src", "examples"];

/**
 * Attributes whose value is read to a person. `alt` is deliberately absent: an image alt text that
 * names a file (`alt="portrait.png"`) has the same no-whitespace-with-a-dot shape as a key, and the
 * gate would then fail on a defect it does not describe.
 */
export const HUMAN_ATTRS = ["aria-label", "aria-description", "aria-roledescription", "aria-placeholder", "title", "placeholder"];

/**
 * The shape of an i18n key: a lowercase word, a dot, then anything but whitespace. Matched against
 * the literal's text up to its first template substitution, so a concatenation prefix that ends in
 * a dot or a hyphen (`"gameSettings.resources.name-"`) is still a key.
 */
export const KEY_SHAPE = /^[a-z][A-Za-z0-9_-]*\.\S*$/;

// One alternation per writer: a quoted attribute value, or an expression whose first token is a
// string or template literal. `=\s*` spans a line break so a value wrapped onto the next line is
// not a way out.
const ATTR_RE = new RegExp(
  `(?:^|[\\s{(])(${HUMAN_ATTRS.join("|")})=\\s*(?:"([^"]*)"|'([^']*)'|\\{\\s*(?:"([^"]*)"|'([^']*)'|\`([^\`]*)\`))`,
  "g",
);

/** Whether `path` is a component file the gate covers. */
export function isComponentSource(path) {
  const p = norm(path);
  return ROOTS.some((r) => under(p, r)) && p.endsWith(".svelte");
}

/**
 * Every human-facing attribute in `text` whose value is written as an i18n key.
 * @param text - A component file's full source.
 * @returns One `{ line, attr, value }` per offending attribute, in source order.
 */
export function scanAriaLabelKeys(text) {
  const lines = text.split("\n").map((l) => l.replace(/\r$/, ""));
  let state = { inBlock: false, inHtml: false };
  const code = [];
  for (const line of lines) {
    const r = splitLine(line, state);
    state = r.state;
    code.push(r.code);
  }
  // Scanned as one string so `=\s*` can cross a line break; the line is recovered from the offset.
  const joined = code.join("\n");
  const out = [];
  for (const m of joined.matchAll(ATTR_RE)) {
    const attr = m[1];
    const isTemplate = m[6] !== undefined;
    const raw = m[2] ?? m[3] ?? m[4] ?? m[5] ?? m[6];
    const head = isTemplate ? raw.split("${")[0] : raw;
    if (!KEY_SHAPE.test(head)) continue;
    const line = joined.slice(0, m.index + m[0].indexOf(attr)).split("\n").length;
    out.push({ line, attr, value: raw });
  }
  return out;
}

function main() {
  const files = execFileSync("git", ["ls-files", "-z", "--", ...ROOTS], { encoding: "utf8" })
    .split("\0")
    .filter(isComponentSource);
  let errors = 0;
  for (const path of files) {
    for (const v of scanAriaLabelKeys(readFileSync(path, "utf8"))) {
      errors++;
      console.error(`I18N KEY AS ACCESSIBLE TEXT: ${norm(path)}:${v.line}: ${v.attr}="${v.value}" — an i18n key is not human text; resolve it with t(...)`);
    }
  }
  console.log(`lint:aria-labels: ${files.length} files scanned, ${errors} error(s)`);
  process.exit(errors === 0 ? 0 : 1);
}

if (isDirectEntry(import.meta.url)) main();
