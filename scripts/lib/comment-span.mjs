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
 *  - No ordinary string state crosses a line, so a line CONTINUING a multi-line template literal is
 *    lexed from a cold start, and a quote inside that literal opens a span that was never open.
 *    A Rust raw string is the one literal whose state does cross lines (`rawStringHashes`).
 *
 * Rust raw strings (`r".."`, `r#".."#`, `br#".."#`) are opaque: a backslash is data, not an escape, and
 * the literal closes at the first `"` followed by AT LEAST as many `#` as the opener had, consuming
 * exactly that many (a further `#` is ordinary code). A bare `r`/`br` directly before `#*"` is always
 * a raw-string opener in compiling Rust, because an identifier cannot abut a string literal.
 * `state.rawStringHashes` is the opener's `#` count while a raw string is open across lines, else
 * `null`; a caller that predates the field passes a state without it and gets `null`.
 *
 * Both UNDER-scan, which is the invisible direction. Each has a specimen in the control set below,
 * asserting what the function does today rather than what it should do — the point of a control is
 * that the behaviour cannot change without a run saying so.
 */
function splitLine(line, state) {
  let code = "";
  let comment = "";
  let i = 0;
  let { inBlock, inHtml, rawStringHashes = null } = state;
  // Index just past the closer of a raw string that needs `hashes` `#`, searching from `from`; -1
  // when this line holds no closer. A `"` followed by fewer `#` than `hashes` is raw-string content.
  const rawCloser = (from, hashes) => {
    let q = line.indexOf('"', from);
    while (q !== -1) {
      let n = 0;
      while (line[q + 1 + n] === "#") n += 1;
      if (n >= hashes) return q + 1 + hashes;
      q = line.indexOf('"', q + 1);
    }
    return -1;
  };
  while (i < line.length) {
    if (rawStringHashes !== null) {
      const end = rawCloser(i, rawStringHashes);
      if (end === -1) {
        return { code: code + line.slice(i), comment, state: { inBlock, inHtml, rawStringHashes } };
      }
      code += line.slice(i, end);
      i = end;
      rawStringHashes = null;
      continue;
    }
    if (inHtml) {
      const end = line.indexOf("-->", i);
      if (end === -1) {
        return { code, comment: comment + line.slice(i), state: { inBlock, inHtml, rawStringHashes } };
      }
      comment += line.slice(i, end);
      i = end + 3;
      inHtml = false;
      continue;
    }
    if (inBlock) {
      const end = line.indexOf("*/", i);
      if (end === -1) {
        return { code, comment: comment + line.slice(i), state: { inBlock, inHtml, rawStringHashes } };
      }
      comment += line.slice(i, end);
      i = end + 2;
      inBlock = false;
      continue;
    }
    const c = line[i];
    if (c === "r" || c === "b") {
      RAW_STRING_OPENER.lastIndex = i;
      const raw = RAW_STRING_OPENER.exec(line);
      if (raw !== null) {
        const hashes = raw[1].length;
        const openEnd = i + raw[0].length;
        const end = rawCloser(openEnd, hashes);
        if (end === -1) {
          return {
            code: code + line.slice(i),
            comment,
            state: { inBlock, inHtml, rawStringHashes: hashes },
          };
        }
        code += line.slice(i, end);
        i = end;
        continue;
      }
    }
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
  return { code, comment, state: { inBlock, inHtml, rawStringHashes } };
}

// The opener of a Rust raw string: `r` or `br`, then `#` characters, then `"`. Sticky so it tests
// exactly at `lastIndex`.
const RAW_STRING_OPENER = /(?:br|r)(#*)"/y;

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
  // Rust raw strings are opaque: no escapes, closer needs at least the opener's `#` count.
  ["a raw string hides a slash-slash from becoming a comment", 'let s = r"http://example.test"; // text', " text"],
  ["a raw string with an embedded bare quote is not closed early", 'let s = r#"say "hi" now"#; // text', " text"],
  [
    "a raw string closer followed by one extra # leaves that # as ordinary code",
    'let s = r#"x"## ; // text',
    " text",
  ],
  [
    "a raw string containing a banned-name-shaped substring stays out of the comment span",
    'let s = r"remove_file(x) // not a call"; // text',
    " text",
  ],
  ["a raw byte string hides a slash-slash", 'let s = br#"a // b"#; // text', " text"],
  ["a raw string backslash is data, not an escape", 'let s = r"a\\"; // text', " text"],
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

// The multi-line raw-string checks thread `state` across calls, which the single-line loop above
// cannot, so they run as their own blocks with the same fail-loud convention.
function failControl(label, expected, actual) {
  console.error(
    `check-comment-refs: splitLine fails its own control "${label}".\n` +
      `  expected: ${JSON.stringify(expected)}\n` +
      `  actual:   ${JSON.stringify(actual)}\n` +
      `Every count this script produces is void until this is fixed.`,
  );
  process.exit(2);
}

{
  const cold = { inBlock: false, inHtml: false };
  const first = splitLine('let s = r#"line one', cold);
  if (first.comment !== "" || first.state.rawStringHashes !== 1) {
    failControl("a raw string opens on one line", { comment: "", rawStringHashes: 1 }, first);
  }
  const second = splitLine('line two"#; // text', first.state);
  if (second.comment !== " text" || second.state.rawStringHashes !== null) {
    failControl("a raw string closes on a later line", { comment: " text", rawStringHashes: null }, second);
  }
}

{
  const cold = { inBlock: false, inHtml: false };
  const first = splitLine('let s = r#"line one', cold);
  const second = splitLine("line two still open // not a comment", first.state);
  if (second.comment !== "" || second.state.rawStringHashes !== 1) {
    failControl("an unterminated raw string stays open", { comment: "", rawStringHashes: 1 }, second);
  }
}
