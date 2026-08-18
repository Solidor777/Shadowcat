// The lexical definition of "comment text" shared by every gate that must tell code from prose.
//
// Two gates need this: the ephemeral-reference scanner examines comment text, and the suppression
// scanner must NOT read an attribute quoted inside a doc comment as a real attribute. Defining it
// once is the point — two scanners with their own idea of what a comment is disagree about the
// same file and neither can say which is right.
//
// The controls below run on import, so every consumer inherits them.

export /**
 * Splits one source line into its comment span and its code span, carrying block-comment state
 * across lines.
 *
 * INVARIANT: a comment is a lexical REGION, not a line shape. Classifying whole lines by a
 * leading-token pattern makes every comment whose text does not begin its line invisible —
 * trailing `//` after code, a block opener, an HTML comment in markup, a block continuation line
 * that does not lead with `*` — and each such gap silently zeroes counts while every BANNED
 * pattern still looks correct. Deriving the span instead of matching a shape removes the whole
 * class rather than one anchor at a time.
 *
 * Over-scanning is the deliberate failure direction WHERE THERE IS A CHOICE: a construct misread as
 * a comment surfaces a visible false positive, whereas a missed comment is invisible and reads as a
 * pass. Two residuals fail the other way, and naming the direction without naming them would claim
 * a property the function does not have:
 *
 *  - A single quote opens a string literal, so a line carrying an ODD number of them consumes the
 *    rest of the line into the CODE span. A Rust lifetime parameter is the live shape — one lone
 *    quote, then a trailing comment that vanishes from the comment span.
 *  - No string state crosses a line, so a line CONTINUING a multi-line template literal is lexed
 *    from a cold start, and a quote inside that literal opens a span that was never open.
 *
 * Both UNDER-scan, which is the invisible direction. Each has a specimen in the control set below,
 * asserting what the function does today rather than what it should do — the point of a control is
 * that the behaviour cannot change without a run saying so.
 */
function splitLine(line, state) {
  let code = "";
  let comment = "";
  let i = 0;
  let { inBlock, inHtml } = state;
  while (i < line.length) {
    if (inHtml) {
      const end = line.indexOf("-->", i);
      if (end === -1) return { code, comment: comment + line.slice(i), state: { inBlock, inHtml } };
      comment += line.slice(i, end);
      i = end + 3;
      inHtml = false;
      continue;
    }
    if (inBlock) {
      const end = line.indexOf("*/", i);
      if (end === -1) return { code, comment: comment + line.slice(i), state: { inBlock, inHtml } };
      comment += line.slice(i, end);
      i = end + 2;
      inBlock = false;
      continue;
    }
    const c = line[i];
    // A `//` or `/*` inside a string literal is program data the code acts on, never a comment
    // opener; consuming the literal whole keeps it in the code span for the literal rules below.
    if (c === '"' || c === "'" || c === "`") {
      let j = i + 1;
      while (j < line.length) {
        if (line[j] === "\\") {
          j += 2;
          continue;
        }
        if (line[j] === c) {
          j += 1;
          break;
        }
        j += 1;
      }
      code += line.slice(i, j);
      i = j;
      continue;
    }
    if (c === "/" && line[i + 1] === "/") {
      comment += line.slice(i + 2);
      break;
    }
    if (c === "/" && line[i + 1] === "*") {
      inBlock = true;
      i += 2;
      continue;
    }
    if (c === "<" && line.startsWith("<!--", i)) {
      inHtml = true;
      i += 4;
      continue;
    }
    code += c;
    i += 1;
  }
  return { code, comment, state: { inBlock, inHtml } };
}

const SUBJECT_SPECIMENS = [
  ["line comment", "// text", " text"],
  ["rust doc comment", "/// text", "/ text"],
  ["rust inner doc", "//! text", "! text"],
  ["block opener", "/** text", "* text"],
  ["block continuation with star", " * text", " * text"],
  ["block continuation without star", "   text", "   text"],
  ["trailing comment after code", "let x = 1; // text", " text"],
  ["html comment", "<!-- text -->", " text "],
  ["code only", "const x = 1;", null],
  ["url inside a string is not a comment", 'const u = "https://example.test/a";', null],
  ["escaped quote does not end the string", 'const s = "a\\"// b";', null],
  // A PAIRED lifetime quote closes the literal the first one opened, so the trailing comment is
  // still reached. This is the control that keeps the two residuals below readable as residuals
  // rather than as "quotes break everything".
  ["paired lifetime quotes leave the trailing comment reachable", "impl<'a> Row<'a> {} // text", " text"],
  // The two UNDER-scanning residuals, pinned to what the function does TODAY. `null` here asserts
  // a comment is NOT seen — the invisible failure direction, which until now had no control at all
  // and so could have changed in either direction without any run saying so.
  ["odd lifetime quote swallows the trailing comment", "pub struct Row<'a> { n: u32 } // text", null],
  ["a template-literal continuation line is lexed cold", "b) + `; // text", null],
];

for (const [label, line, want] of SUBJECT_SPECIMENS) {
  // A star-led continuation is comment text only while block state is open, so it is fed that
  // state; every other specimen is checked from a cold start.
  const cold = { inBlock: label.startsWith("block continuation"), inHtml: false };
  const got = splitLine(line, cold).comment;
  const ok = want === null ? got.trim() === "" : got === want;
  if (!ok) {
    console.error(
      `check-comment-refs: splitLine fails its own control "${label}".\n` +
        `  line:     ${JSON.stringify(line)}\n` +
        `  expected: ${JSON.stringify(want)}\n` +
        `  actual:   ${JSON.stringify(got)}\n` +
        `Every count this script produces is void until this is fixed.`,
    );
    process.exit(2);
  }
}
