// Verifies every code-symbol citation in a tracked codebase skill resolves to a symbol that
// actually exists in the tree. RULE 15 requires skill prose to cite symbols rather than file names
// or line numbers, on the argument that "a symbol breaks only on rename, which a grep finds" —
// nobody ran that grep, so two dead citations (a renamed test function, a shorthand that was never
// a real symbol) sat undetected. `check-skill-api-refs.mjs` verifies a DIFFERENT corpus —
// `/api/...` doc-site pointers against `dist-docs/` — and does not see this class at all. This
// script is the missing check for the class RULE 15 actually argues about: a backticked identifier
// or `Owner::member`/`Owner.member` path cited as if it names live code.
//
// A token's SHAPE never decides whether it is checked; only resolution does. A shape-based
// exclusion is invisible in both directions at once — it hides the citations it skips AND it hides
// every gap in the symbol index, because a name the index cannot see is indistinguishable from a
// name the shape rule declined to look at. Every backtick span is therefore classified into
// exactly one printed bucket: verified, acknowledged non-symbol, broken, or not citation-shaped.
//
// Pure library: no top-level side effects. `check-skill-symbol-refs-cli.mjs` is the executable
// entry point.
//
// Cross-platform: node:path/node:fs only, plus one `git ls-files` invocation for corpus scoping.
import { readFileSync, readdirSync } from "node:fs";
import { join, basename } from "node:path";
import { execFileSync } from "node:child_process";
import ts from "typescript";
import {
  sources,
  norm,
  under,
  GENERATED_ROOT,
  EXAMPLE_EXEMPT,
  MD_ROOTS,
  MD_EXTS,
  SKIP_DIRS,
} from "./check-comment-refs.mjs";

// A brace-depth counter that reads every character, including inside a string or char literal,
// misreads a lone unbalanced brace-shaped character (a Rust char literal `'{'`, found in this
// tree's own `no_debug_artifacts` helper) as a real nesting change — the two DO NOT cancel out on
// that line, so depth silently drifts for the rest of the file and a later container's closing
// brace no longer matches the depth recorded when it opened. Stripping string/char-literal
// CONTENTS (not the file's declaration syntax, which brace-counting never needs to see inside a
// literal) before counting removes the false signal at its source, rather than tolerating a drift
// the rest of this module was built to be robust to.
const RUST_STRING_OR_CHAR_LITERAL = /"(?:[^"\\]|\\.)*"|'(?:\\.|[^'\\])'/g;

/** Blanks out string/char literal contents on one Rust source line, for brace-depth counting
 * only — the returned text is never used for symbol-name extraction. */
function stripLiteralsForBraceCounting(line) {
  return line.replace(RUST_STRING_OR_CHAR_LITERAL, (m) => "_".repeat(m.length));
}

/**
 * Splits source text into lines with any carriage return removed. Line-ending normalization
 * happens ONCE, here, rather than in each pattern: git checks this tree out with CRLF endings on
 * Windows, and a `$`-anchored pattern applied to a line still carrying its `\r` fails to match
 * with no error and no output difference — the file is simply, silently, indexed short. A
 * per-pattern `\r?$` fixes the patterns someone remembered; normalizing the input fixes the class.
 * @param {string} text - source file contents.
 * @returns {string[]} lines, free of carriage returns.
 */
export function splitSourceLines(text) {
  return text.split("\n").map((l) => (l.endsWith("\r") ? l.slice(0, -1) : l));
}

// ---------------------------------------------------------------------------------------------
// Rust symbol index: fn / struct / enum / trait / type items, plus impl-block methods (both bare
// and `Type::method` qualified, so a citation in either style resolves).
// ---------------------------------------------------------------------------------------------

const RUST_ITEM = /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:default\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]*"\s+)?(fn|struct|enum|trait|const|static|type)\s+([A-Za-z_][A-Za-z0-9_]*)/;
const RUST_IMPL = /^\s*(?:unsafe\s+)?impl(?:<[^{]*?>)?\s+(?:[A-Za-z_][A-Za-z0-9_:]*(?:<[^{]*?>)?\s+for\s+)?([A-Za-z_][A-Za-z0-9_]*)/;
const RUST_TRAIT = /^\s*(?:pub(?:\([^)]*\))?\s+)?trait\s+([A-Za-z_][A-Za-z0-9_]*)/;
const RUST_MOD = /^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*[;{]/;
// Field and variant patterns are GLOBAL and match at any `{`/`,`/`(` boundary, not only at the
// start of a line: several fields or variants on one source line is legal Rust (just not
// `cargo fmt`'s default), and a one-match-per-line reader indexes the first and drops the rest
// with no signal that it did.
const RUST_STRUCT_FIELD = /(?:^|[{,(])\s*(?:pub(?:\([^)]*\))?\s+)?([a-zA-Z_][a-zA-Z0-9_]*)\s*:(?!:)/g;
const RUST_ENUM_VARIANT = /(?:^|[{,])\s*([A-Za-z_][A-Za-z0-9_]*)\s*(?=[,({}]|$)/g;

// A `let` binding and a function parameter are declarations the tree really carries, and skill
// prose cites them by name whenever it explains an algorithm step by step (`t_max_i` in a
// traversal, `check_mask` in a gate, `db_path` in a restore). Indexing them is what lets a
// citation of one be VERIFIED — and lets a rename of one be caught — instead of being written off
// as an unresolvable shape. Tuple/struct destructuring patterns are matched too, since the names
// they bind are exactly as citable.
const RUST_LET_BINDING = /\blet\s+(?:mut\s+)?([a-z_][A-Za-z0-9_]*)/g;
const RUST_LET_TUPLE = /\blet\s+\(([^)]*)\)\s*=/g;
const RUST_FN_SIGNATURE_OPEN = /\bfn\s+[A-Za-z_][A-Za-z0-9_]*\s*(?:<[^>(]*>)?\s*\(/;
// A parameter inside a signature: `name: Type`, at a `(` or `,` boundary. `self` binds no name.
const RUST_FN_PARAM = /(?:^|[(,])\s*(?:mut\s+)?([a-z_][A-Za-z0-9_]*)\s*:(?!:)/g;

// `#[serde(rename_all = "...")]` on a container, and `#[serde(rename = "...")]` on one field or
// variant, are DECLARATIONS of a wire name: the JSON a document actually carries says `blocksMove`
// and nothing in the tree spells that name any other way. Skipping the attribute makes every
// renamed wire field look like a citation of something that does not exist.
const SERDE_RENAME_ALL = /\brename_all\s*=\s*"([^"]+)"/;
const SERDE_RENAME = /\brename\s*=\s*"([^"]+)"/;

/** Splits an identifier into lowercase words, on `_` boundaries or camel/Pascal humps. */
function identifierWords(name) {
  if (name.includes("_")) return name.split("_").filter(Boolean).map((w) => w.toLowerCase());
  return name
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .split(" ")
    .filter(Boolean)
    .map((w) => w.toLowerCase());
}

/**
 * Applies one serde `rename_all` style to a declared Rust identifier, yielding the wire name the
 * serialized document actually carries.
 * @param {string} name - the Rust field or variant identifier.
 * @param {string} style - a serde `rename_all` style string.
 * @returns {string|null} the renamed form, or null for a style this recognizer does not model.
 */
export function applyRenameAll(name, style) {
  const words = identifierWords(name);
  const cap = (w) => w.charAt(0).toUpperCase() + w.slice(1);
  switch (style) {
    case "lowercase":
      return words.join("");
    case "UPPERCASE":
      return words.join("").toUpperCase();
    case "PascalCase":
      return words.map(cap).join("");
    case "camelCase":
      return words.map((w, i) => (i === 0 ? w : cap(w))).join("");
    case "snake_case":
      return words.join("_");
    case "SCREAMING_SNAKE_CASE":
      return words.join("_").toUpperCase();
    case "kebab-case":
      return words.join("-");
    case "SCREAMING-KEBAB-CASE":
      return words.join("-").toUpperCase();
    default:
      return null;
  }
}

/**
 * Derives the full module-path segments a `.rs` file's OWN declarations sit under, from
 * `src/server/src` (the crate root) down: each intervening directory name is one segment, and
 * the file's own basename is the final segment — except a `mod.rs` file, which contributes no
 * segment of its own (it names its parent directory's module). `main.rs`/`lib.rs` at the crate
 * root are not real Rust path segments (the crate root has no name from outside the crate), but
 * this repo's own skill prose still writes `main::run_backup` as a doc-convention shorthand for
 * "declared in `main.rs`" — the same non-literal-Rust-syntax latitude already extended to a
 * struct field's `::`/`.` separator choice — so `main` (not `lib`, which no skill cites this way)
 * is kept as a one-segment path rather than `[]`.
 * @param {string} rustRoot - absolute path to `src/server/src`.
 * @param {string} filePath - absolute path to a `.rs` file under `rustRoot`.
 * @returns {string[]} module path segments, root-to-leaf.
 */
export function rustModulePath(rustRoot, filePath) {
  const rel = norm(filePath).slice(norm(rustRoot).length + 1);
  const segs = rel.split("/");
  const file = segs.pop();
  const stem = file.replace(/\.rs$/, "");
  if (stem === "lib") return [];
  if (stem === "main") return ["main"];
  if (stem !== "mod") segs.push(stem);
  return segs;
}

/**
 * Extracts every Rust symbol this checker resolves citations against, from one file's text:
 * `fn`/`struct`/`enum`/`trait`/`const`/`static`/`type` item names (bare, and — when the module
 * path is known — as `stem::NAME`, since every item in `pathfinding.rs` is reachable from outside
 * as `pathfinding::NAME`); every `fn` SIGNATURE found inside an `impl <Type>` (`impl Trait for
 * <Type>`) block OR a `trait <Name> { … }` body — a trait method has no body of its own but is
 * exactly as real a citable symbol as an implementation's — indexed both bare and as
 * `Owner::method`; every `mod NAME` declaration, indexed bare and as `stem::NAME`; every struct
 * FIELD and enum VARIANT declared in a brace-bodied item, indexed both bare and as `Type.field` /
 * `Type::Variant` — these are real symbols the tree declares exactly as much as a function name
 * is, and a skill legitimately cites `RegionField.terrain_multiplier`-style field access and
 * `MoveReject::SceneUnknown`-style variant names; and each field's or variant's serde WIRE name,
 * from `#[serde(rename = "…")]` or the container's `#[serde(rename_all = "…")]`.
 *
 * Brace-depth tracking is line-based and does not parse strings/comments; a `{`/`}` inside a
 * string or comment can shift the count by one. Rust source overwhelmingly keeps braces off
 * string/comment lines, and a rare miscount only ever WIDENS which lines are treated as "inside a
 * container" — it cannot cause a real item to be missed from the bare-name index, only
 * (harmlessly) cause an extra qualified alias, or an extra field/variant candidate, to be
 * recorded.
 *
 * A bare item is additionally registered under EVERY SUFFIX of its enclosing module path (from
 * `rustModulePath`) — `data::sqlite::apply_intent`, `sqlite::apply_intent`, and bare
 * `apply_intent` all resolve — since this repo's own citations mix full-path, one-hop, and bare
 * forms and none of them is wrong.
 *
 * @param {string} text - one `.rs` file's contents.
 * @param {string[]} [modulePath] - this file's module path segments, from `rustModulePath`;
 *   defaults to `[]` (no qualification) in tests that don't care about it.
 * @returns {Set<string>} every bare and qualified symbol name declared in this file.
 */
export function extractRustSymbols(text, modulePath = []) {
  const names = new Set();
  // An INLINE `mod name { … }` body (as opposed to a file-referencing `mod name;`) nests further
  // declarations under its own name too — `pub mod cap { pub const WRITE_FIELDS: … }` makes
  // `cap::WRITE_FIELDS` a real path a citation can name. Stacked (not a single slot) since inline
  // modules can themselves nest.
  const modStack = []; // { name, depth }[]
  /** Registers `name` bare, plus once per suffix of `[...modulePath, ...modStack names]`
   * (`a::b::name`, `b::name`, …, and every inline-module-qualified form). */
  const addQualified = (name) => {
    names.add(name);
    const chain = [...modulePath, ...modStack.map((m) => m.name)];
    for (let i = 0; i < chain.length; i += 1) {
      names.add(`${chain.slice(i).join("::")}::${name}`);
    }
  };
  const lines = splitSourceLines(text);
  // Method-owner container (`impl <Type>` or `trait <Name>`): qualifies a `fn` signature found
  // inside its body as `Owner::method`, whether that signature has a body (`impl`) or is a bare
  // declaration (`trait`).
  let methodOwner = null; // { name, depth }
  // Struct/enum field-and-variant container: at most one active at a time, since a struct or
  // enum body cannot itself open a SECOND struct/enum body at the same nesting level the
  // container tracks — a nested brace (a struct-variant's own fields) simply sits one level
  // deeper than `depth === container.depth + 1` and is correctly excluded without a stack.
  let container = null; // { kind: "struct" | "enum", name, depth, renameAll }
  let depth = 0;
  // Attribute lines accumulate until the declaration they annotate consumes them. A doc comment
  // or a blank line between the two does not clear them, because both routinely sit there.
  let pendingAttrs = [];
  let inSignature = false;
  let signatureParens = 0;
  for (const line of lines) {
    const trimmed = line.trim();
    const isAttr = trimmed.startsWith("#[") || trimmed.startsWith("#!");
    const isComment = trimmed.startsWith("//");
    let attrs = "";
    if (isAttr) {
      pendingAttrs.push(trimmed);
    } else if (!isComment && trimmed !== "") {
      attrs = pendingAttrs.join(" ");
      pendingAttrs = [];
    }
    if (!isAttr && !isComment) {
      if (methodOwner === null) {
        const implMatch = RUST_IMPL.exec(line);
        const traitMatch = implMatch ? null : RUST_TRAIT.exec(line);
        if (implMatch) methodOwner = { name: implMatch[1], depth };
        else if (traitMatch) methodOwner = { name: traitMatch[1], depth };
      }
      const item = RUST_ITEM.exec(line);
      if (item) {
        const [, kind, name] = item;
        addQualified(name);
        if (kind === "fn" && methodOwner !== null) {
          // Qualified BOTH ways: `Type::method` alone (the common citation form) and, chained
          // through `addQualified`, every `module::path::Type::method` prefix too — a citation may
          // combine the module path with the owning type, as `data::sqlite::SqliteRepository::connect`
          // does.
          addQualified(`${methodOwner.name}::${name}`);
        }
        if ((kind === "struct" || kind === "enum") && line.includes("{") && container === null) {
          container = {
            kind,
            name,
            depth,
            renameAll: SERDE_RENAME_ALL.exec(attrs)?.[1] ?? null,
          };
        }
      } else if (container === null) {
        const modMatch = RUST_MOD.exec(line);
        if (modMatch) {
          addQualified(modMatch[1]);
          if (line.trimEnd().endsWith("{")) modStack.push({ name: modMatch[1], depth });
        }
      }
      if (container !== null && depth === container.depth + 1) {
        const pattern = container.kind === "struct" ? RUST_STRUCT_FIELD : RUST_ENUM_VARIANT;
        pattern.lastIndex = 0;
        const explicitRename = SERDE_RENAME.exec(attrs)?.[1] ?? null;
        for (const match of line.matchAll(pattern)) {
          const member = match[1];
          // Both separators, regardless of container kind: Markdown prose is not held to strict
          // Rust syntax, and a skill citing a struct field as `RollSpec::direction` (real syntax:
          // `.direction`) means the same thing to a reader as `RollSpec.direction` — the
          // conceptual owner-qualification, not the literal access-operator choice. Routed through
          // `addQualified` so a module-path-prefixed form (`recalc::RecalcOp::ReplaceDie`)
          // resolves too, exactly as a plain item's qualification does.
          for (const form of [
            member,
            explicitRename,
            container.renameAll === null ? null : applyRenameAll(member, container.renameAll),
          ]) {
            if (form === null) continue;
            addQualified(form);
            addQualified(`${container.name}.${form}`);
            addQualified(`${container.name}::${form}`);
          }
        }
      }
    }
    if (!isComment) {
      for (const m of line.matchAll(RUST_LET_BINDING)) names.add(m[1]);
      for (const m of line.matchAll(RUST_LET_TUPLE))
        for (const part of m[1].split(","))
          for (const id of part.trim().matchAll(/\b([a-z_][A-Za-z0-9_]*)\b/g)) names.add(id[1]);
      // A signature can wrap across lines, so parameter scanning stays open until the parameter
      // list's own paren closes rather than assuming the signature fits on the `fn` line.
      const code = stripLiteralsForBraceCounting(line);
      let scanFrom = 0;
      if (!inSignature) {
        const open = RUST_FN_SIGNATURE_OPEN.exec(code);
        if (open !== null) {
          inSignature = true;
          signatureParens = 0;
          scanFrom = open.index;
        }
      }
      if (inSignature) {
        const rest = code.slice(scanFrom);
        for (const m of rest.matchAll(RUST_FN_PARAM)) names.add(m[1]);
        for (const ch of rest) {
          if (ch === "(") signatureParens += 1;
          else if (ch === ")") {
            signatureParens -= 1;
            if (signatureParens <= 0) {
              inSignature = false;
              break;
            }
          }
        }
      }
    }
    for (const ch of stripLiteralsForBraceCounting(line)) {
      if (ch === "{") depth += 1;
      else if (ch === "}") {
        depth -= 1;
        if (methodOwner !== null && depth <= methodOwner.depth) methodOwner = null;
        if (container !== null && depth <= container.depth) container = null;
        while (modStack.length > 0 && depth <= modStack[modStack.length - 1].depth)
          modStack.pop();
      }
    }
  }
  for (const name of extractRustReexports(text)) addQualified(name);
  for (const name of extractRustLiteralAlternatives(text)) names.add(name);
  return names;
}

// A run of string-literal alternatives in a Rust pattern (`matches!(t, "token" | "scene" | …)`,
// a `match` arm) DECLARES a closed set of wire values, exactly as a TS string-literal union type
// does on the client side. Requiring at least two alternatives is what separates a declaration of
// a value set from an ordinary string argument.
const RUST_LITERAL_ALTERNATIVES = /"[A-Za-z0-9_-]+"(?:\s*\|\s*"[A-Za-z0-9_-]+")+/g;

/**
 * Extracts every string literal appearing in an alternation pattern in one Rust file's text.
 * @param {string} text - one `.rs` file's contents.
 * @returns {Set<string>} every alternative's literal value.
 */
export function extractRustLiteralAlternatives(text) {
  const names = new Set();
  for (const run of text.matchAll(RUST_LITERAL_ALTERNATIVES))
    for (const lit of run[0].matchAll(/"([A-Za-z0-9_-]+)"/g)) names.add(lit[1]);
  return names;
}

// `pub use path::{a, b as c};` / `pub use path::name;` re-exports the imported item's name INTO
// this module's own public surface — RULE 15's own citation convention names a re-exporting
// module rather than the literal declaring file (`chat/mod.rs`'s `broadcast` → `chat::broadcast`
// is the rule's own example), so a re-exported item must resolve under the RE-EXPORTING file's
// module path, not only its original declaring file's. Matched across the whole file text rather
// than line-by-line because a `use` list commonly wraps its braces across several lines.
const RUST_USE = /\bpub(?:\([^)]*\))?\s+use\s+([^;]+);/g;

/**
 * Extracts every name a `pub use` statement re-exports from one Rust file's text: the alias
 * target of `path::{a, b as c}` (registers `c`, not `b`) or a bare `path::name`.
 * @param {string} text - one `.rs` file's contents.
 * @returns {string[]} every re-exported name.
 */
export function extractRustReexports(text) {
  const names = [];
  for (const m of text.matchAll(RUST_USE)) {
    const useBody = m[1].replace(/\/\/[^\n]*/g, "").trim();
    const listMatch = /\{([^}]*)\}\s*$/.exec(useBody);
    const items = listMatch
      ? listMatch[1].split(",").map((s) => s.trim()).filter(Boolean)
      : [useBody.split("::").pop().trim()];
    for (const item of items) {
      if (item === "*" || item === "self") continue;
      const asMatch = /\bas\s+([A-Za-z_][A-Za-z0-9_]*)\s*$/.exec(item);
      const name = asMatch ? asMatch[1] : item.split("::").pop().trim();
      if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) names.push(name);
    }
  }
  return names;
}

// ---------------------------------------------------------------------------------------------
// SQL schema index: the baseline migration is the sole declaration of every table and column
// name, and skill prose cites those names directly (a table a broadcast reads, a column a query
// filters on). No other extractor can see them — a `.sql` file has no Rust or TS declaration to
// mirror — so without this source every schema citation is a name the tree "does not declare".
// ---------------------------------------------------------------------------------------------

const SQL_TABLE = /\bCREATE\s+(?:VIRTUAL\s+)?TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?([A-Za-z_][A-Za-z0-9_]*)/gi;
const SQL_INDEX = /\bCREATE\s+(?:UNIQUE\s+)?INDEX\s+(?:IF\s+NOT\s+EXISTS\s+)?([A-Za-z_][A-Za-z0-9_]*)/gi;
// A column declaration is the first identifier on a line inside a `CREATE TABLE (...)` body. The
// leading-keyword exclusions are the table-level constraint clauses, which occupy the same
// position but declare no column.
const SQL_COLUMN = /^\s*([A-Za-z_][A-Za-z0-9_]*)\s+(?!KEY\b)[A-Za-z(]/;
const SQL_CONSTRAINT_KEYWORD =
  /^(?:PRIMARY|FOREIGN|UNIQUE|CHECK|CONSTRAINT|CREATE|INSERT|PRAGMA|BEGIN|COMMIT|END|WITHOUT|SELECT|VALUES|ON|REFERENCES|DEFAULT)$/i;

/**
 * Extracts every table, index and column name declared by one `.sql` schema file.
 * @param {string} text - one `.sql` file's contents.
 * @returns {Set<string>} table/index names, column names, and `table.column` qualified forms.
 */
export function extractSqlSymbols(text) {
  const names = new Set();
  for (const m of text.matchAll(SQL_TABLE)) names.add(m[1]);
  for (const m of text.matchAll(SQL_INDEX)) names.add(m[1]);
  let table = null;
  let depth = 0;
  for (const line of splitSourceLines(text)) {
    const trimmed = line.trim();
    if (trimmed.startsWith("--")) continue;
    if (table !== null && depth > 0) {
      const col = SQL_COLUMN.exec(trimmed);
      if (col && !SQL_CONSTRAINT_KEYWORD.test(col[1])) {
        names.add(col[1]);
        names.add(`${table}.${col[1]}`);
      }
    }
    SQL_TABLE.lastIndex = 0;
    const open = SQL_TABLE.exec(line);
    if (open) table = open[1];
    for (const ch of trimmed) {
      if (ch === "(") depth += 1;
      else if (ch === ")") {
        depth -= 1;
        if (depth <= 0) {
          depth = 0;
          table = null;
        }
      }
    }
  }
  return names;
}

// ---------------------------------------------------------------------------------------------
// TS/JS/Svelte symbol index, read from the TypeScript parser's syntax tree rather than from
// line patterns. A pattern reader cannot see a member nested inside an inline object type, and
// mis-reads several real declaration forms outright (`export const enum X` registers the word
// `enum`; a type-only re-export registers nothing) — silently, since a name absent from the index
// looks exactly like a name absent from the tree. The parser answers the question the patterns
// were approximating: what does this file declare, at any depth?
// ---------------------------------------------------------------------------------------------

/** The declared name of a member node, with a `#private` marker stripped; "" when the member is
 * computed (`[key]: T`), which declares no citable name. */
function declaredMemberName(name) {
  if (name === undefined) return "";
  if (ts.isIdentifier(name)) return name.text;
  if (ts.isPrivateIdentifier(name)) return name.text.replace(/^#/, "");
  if (ts.isStringLiteral(name) || ts.isNumericLiteral(name)) return name.text;
  return "";
}

// A declared name, or a dotted chain of them: an i18n catalog and a config map both declare a
// dotted key as a single quoted property, and that key is what prose cites.
const IDENTIFIER_SHAPED = /^[A-Za-z_$][A-Za-z0-9_$]*(?:\.[A-Za-z_$][A-Za-z0-9_$]*)*$/;

/**
 * Extracts every symbol name one TS/JS module declares: top-level `function`/`class`/`interface`/
 * `type`/`enum`/`namespace` declarations and module-level `const`/`let`/`var` bindings (exported
 * or not — a skill may legitimately cite a module-private symbol); every class, interface,
 * type-literal and enum MEMBER at any nesting depth, indexed bare and under every suffix of its
 * owner chain (`UiState.global.lastWorld`, `global.lastWorld`, `lastWorld`) so both the
 * declaration-citation shape and this repo's wire-path notation resolve at whatever depth prose
 * cites them; every name a named re-export publishes (`export { a as b }`,
 * `export type { Foo }`); every name the module IMPORTS, which is this tree's own declaration
 * that the name exists in a package it depends on; and every identifier-shaped string LITERAL
 * TYPE, which is how a wire-protocol discriminant value is declared (`type SyncState = "none" |
 * "up_to_date"`).
 *
 * @param {string} text - one `.ts`/`.mjs`/`.js` file's contents, or a `.svelte` file's extracted
 *   script text.
 * @param {string} [fileName] - a name for the parser's diagnostics; never read as a path.
 * @returns {Set<string>} every symbol name this module declares.
 */
export function extractTsSymbols(text, fileName = "module.ts") {
  const names = new Set();
  const source = ts.createSourceFile(fileName, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  // Registered under EVERY SUFFIX of its owner chain, mirroring the Rust side's module-path
  // qualification: `AppContext.chat.send`, `chat.send` and bare `send` all name the same
  // declaration, and this repo's own prose cites all three depths.
  const add = (name, chain) => {
    if (name === "" || !IDENTIFIER_SHAPED.test(name)) return;
    names.add(name);
    for (let i = 0; i < chain.length; i += 1) names.add(`${chain.slice(i).join(".")}.${name}`);
  };

  const walk = (node, chain) => {
    let nextChain = chain;
    if (
      ts.isFunctionDeclaration(node) ||
      ts.isClassDeclaration(node) ||
      ts.isInterfaceDeclaration(node) ||
      ts.isTypeAliasDeclaration(node) ||
      ts.isEnumDeclaration(node) ||
      ts.isModuleDeclaration(node)
    ) {
      const named = node.name !== undefined && ts.isIdentifier(node.name) ? node.name.text : "";
      if (named !== "") {
        add(named, chain);
        if (!ts.isFunctionDeclaration(node)) nextChain = [named];
      }
    } else if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name)) {
      add(node.name.text, chain);
      nextChain = [node.name.text];
    } else if (ts.isBindingElement(node) && ts.isIdentifier(node.name)) {
      add(node.name.text, []);
    } else if (ts.isPropertyAssignment(node) || ts.isShorthandPropertyAssignment(node)) {
      const named = declaredMemberName(node.name);
      if (named !== "") {
        add(named, chain);
        nextChain = [...chain, named];
      }
    } else if (
      ts.isPropertySignature(node) ||
      ts.isMethodSignature(node) ||
      ts.isPropertyDeclaration(node) ||
      ts.isMethodDeclaration(node) ||
      ts.isGetAccessorDeclaration(node) ||
      ts.isSetAccessorDeclaration(node) ||
      ts.isEnumMember(node)
    ) {
      const named = declaredMemberName(node.name);
      if (named !== "") {
        add(named, chain);
        nextChain = [...chain, named];
      }
    } else if (ts.isParameter(node)) {
      // A parameter is a declaration prose legitimately cites by name; a constructor parameter
      // property additionally declares a real class member, which is why the owner chain applies.
      add(declaredMemberName(node.name), node.modifiers === undefined ? [] : chain);
    } else if (ts.isExportSpecifier(node) || ts.isImportSpecifier(node)) {
      add(node.name.text, []);
    } else if (ts.isNamespaceImport(node) || ts.isNamespaceExport(node)) {
      add(node.name.text, []);
    } else if (ts.isImportClause(node) && node.name !== undefined) {
      add(node.name.text, []);
    } else if (ts.isLiteralTypeNode(node) && ts.isStringLiteral(node.literal)) {
      add(node.literal.text, []);
    }
    ts.forEachChild(node, (child) => walk(child, nextChain));
  };
  ts.forEachChild(source, (child) => walk(child, []));
  return names;
}

/**
 * Extracts a `.svelte` file's `<script>` and `<script module>` block contents (each may appear
 * once), for feeding to `extractTsSymbols`. Returns "" when neither block is present.
 * @param {string} text - one `.svelte` file's raw contents.
 * @returns {string} the concatenated script block contents.
 */
export function extractSvelteScript(text) {
  const out = [];
  for (const m of text.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g)) out.push(m[1]);
  return out.join("\n");
}

/**
 * Extracts every object KEY a JSON document declares, at any depth. A config file's keys are the
 * only place a build/tooling option this repo actually sets is written down, and skill prose cites
 * them by name; RULE 15's carve-out says a config file has no code symbols to CITE, not that a
 * citation OF one may point at a key nobody set. Values are ignored — a key is a declaration, a
 * value is data.
 * @param {string} text - one `.json` file's contents.
 * @returns {Set<string>} every key name, and every `owner.key` qualified form.
 */
export function extractJsonKeys(text) {
  const names = new Set();
  let parsed;
  try {
    parsed = JSON.parse(text);
  } catch {
    return names;
  }
  const walk = (node, owner) => {
    if (Array.isArray(node)) {
      for (const item of node) walk(item, owner);
      return;
    }
    if (node === null || typeof node !== "object") return;
    for (const [key, value] of Object.entries(node)) {
      names.add(key);
      if (owner !== null) names.add(`${owner}.${key}`);
      walk(value, key);
    }
  };
  walk(parsed, null);
  return names;
}

/**
 * Extracts every key and table header a TOML manifest declares — for Cargo, that is every
 * dependency, feature and target name. A hyphenated crate name is additionally indexed in its
 * Rust-path spelling, since prose cites `axum_test` for a manifest entry reading `axum-test`.
 * @param {string} text - one `.toml` file's contents.
 * @returns {Set<string>} every declared key, plus underscore spellings of hyphenated ones.
 */
export function extractTomlKeys(text) {
  const names = new Set();
  const record = (raw) => {
    const name = raw.replace(/^"|"$/g, "");
    if (!/^[A-Za-z_][A-Za-z0-9_-]*$/.test(name)) return;
    names.add(name);
    if (name.includes("-")) names.add(name.split("-").join("_"));
  };
  for (const line of splitSourceLines(text)) {
    const trimmed = line.trim();
    if (trimmed.startsWith("#")) continue;
    const header = /^\[+([^\]]+)\]+/.exec(trimmed);
    if (header !== null) {
      for (const seg of header[1].split(".")) record(seg.trim());
      continue;
    }
    const key = /^("?[A-Za-z_][A-Za-z0-9_-]*"?)\s*=/.exec(trimmed);
    if (key !== null) record(key[1]);
  }
  return names;
}

/**
 * The module NAME a source path declares, with every extension stripped
 * (`sheetsController.svelte.ts` → `sheetsController`). A skill cites a module as a whole at least
 * as often as it cites one item inside it, and that citation names something the tree really
 * declares — the module — so resolving it against the file's own name is not a shape guess.
 * @param {string} filePath - absolute path to a source file.
 * @returns {string} the module name, or "" when the file name is not identifier-shaped.
 */
export function moduleNameOf(filePath) {
  const stem = basename(filePath).split(".")[0];
  return IDENTIFIER_SHAPED.test(stem) ? stem : "";
}

/**
 * Builds the full code-symbol index this checker resolves citations against: every Rust item,
 * impl/trait method, field, variant and serde wire name under `src/server/src`; every table,
 * index and column the SQL baseline declares; every TS/JS declaration, member, re-export, import
 * and literal-type value under `src/client`, `src/modules`, `src/types` (excluding
 * `GENERATED_ROOT`) and `scripts`; and every `.svelte` file's component name (its basename) plus
 * its script-block declarations.
 *
 * @param {string} repoRoot - absolute path to the repository root.
 * @returns {{ symbols: Set<string>, filesIndexed: number }}
 */
export function buildSymbolIndex(repoRoot) {
  const symbols = new Set();
  let filesIndexed = 0;

  const rustRoot = join(repoRoot, "src", "server", "src");
  for (const file of sources(rustRoot, [".rs"])) {
    filesIndexed += 1;
    const text = readFileSync(file, "utf8");
    const modulePath = rustModulePath(rustRoot, file);
    for (const name of extractRustSymbols(text, modulePath)) symbols.add(name);
    // The module a file IS, under every suffix of its path: a binary target under `bin/` is never
    // reached by a `mod` declaration, so `bin::test_server` and `test_server` are otherwise names
    // the tree carries and the index cannot see.
    for (let i = 0; i < modulePath.length; i += 1) symbols.add(modulePath.slice(i).join("::"));
  }

  // Cargo's manifests declare every crate this repo depends on, and skill prose names those crates
  // directly when it explains which library owns a behaviour. Rust paths spell a hyphenated crate
  // name with underscores, so both forms are indexed.
  for (const file of sources(join(repoRoot, "src", "server"), [".toml"])) {
    filesIndexed += 1;
    for (const name of extractTomlKeys(readFileSync(file, "utf8"))) symbols.add(name);
  }

  const sqlRoot = join(repoRoot, "src", "server", "migrations");
  for (const file of sources(sqlRoot, [".sql"])) {
    filesIndexed += 1;
    for (const name of extractSqlSymbols(readFileSync(file, "utf8"))) symbols.add(name);
  }

  const tsRoots = ["src/client", "src/modules", "src/types", "scripts"];
  for (const root of tsRoots) {
    const abs = join(repoRoot, ...root.split("/"));
    // Every package/module directory is a declaration too: `src/modules/topbar` is what makes
    // "the `topbar` module" a citation of something real rather than of a word.
    for (const entry of readdirSync(abs, { withFileTypes: true })) {
      if (entry.isDirectory() && !SKIP_DIRS.has(entry.name)) symbols.add(entry.name);
    }
    for (const file of sources(abs, [".ts", ".mjs", ".js"])) {
      const rel = norm(join(root, file.slice(abs.length + 1)));
      if (under(rel, GENERATED_ROOT)) continue;
      filesIndexed += 1;
      symbols.add(moduleNameOf(file));
      for (const name of extractTsSymbols(readFileSync(file, "utf8"), basename(file)))
        symbols.add(name);
    }
    for (const file of sources(abs, [".svelte"])) {
      filesIndexed += 1;
      const componentName = basename(file, ".svelte");
      symbols.add(componentName);
      const script = extractSvelteScript(readFileSync(file, "utf8"));
      for (const name of extractTsSymbols(script, `${componentName}.ts`)) {
        symbols.add(name);
        // A skill legitimately cites a script-level function as the COMPONENT's own behaviour
        // (`SystemTreeEditor.removeField`) — the component name is this file's de facto exported
        // owner, exactly as a TS class or interface name owns its members.
        symbols.add(`${componentName}.${name}`);
      }
    }
    for (const file of sources(abs, [".json"])) {
      const rel = norm(join(root, file.slice(abs.length + 1)));
      if (under(rel, GENERATED_ROOT)) continue;
      filesIndexed += 1;
      for (const name of extractJsonKeys(readFileSync(file, "utf8"))) symbols.add(name);
    }
  }
  // Repo-root config files: an eslint or TypeDoc option a skill cites is set here and nowhere
  // else, and the lint configs are real modules whose own bindings prose names.
  for (const name of readdirSync(repoRoot)) {
    const path = join(repoRoot, name);
    if (name.endsWith(".json")) {
      filesIndexed += 1;
      for (const key of extractJsonKeys(readFileSync(path, "utf8"))) symbols.add(key);
    } else if (/\.(?:ts|js|mjs|cjs)$/.test(name)) {
      filesIndexed += 1;
      symbols.add(moduleNameOf(path));
      for (const key of extractTsSymbols(readFileSync(path, "utf8"), name)) symbols.add(key);
    }
  }

  // The skill family names its own members (`nightfox`, `client-shell`) when it points a reader at
  // a sibling skill. Those directories are as real a declaration as a source module.
  for (const root of MD_ROOTS) {
    for (const entry of readdirSync(join(repoRoot, ...root.split("/")), { withFileTypes: true })) {
      if (!entry.isDirectory()) continue;
      symbols.add(entry.name);
      symbols.add(entry.name.replace(/^shadowcat-codebase-/, ""));
    }
  }
  symbols.delete("");

  return { symbols, filesIndexed };
}

// ---------------------------------------------------------------------------------------------
// Corpus scoping and citation extraction, over the tracked skill directories.
// ---------------------------------------------------------------------------------------------

/**
 * The skill directories git TRACKS — the corpus this gate governs. Tracked-ness is the property
 * that actually matters: a directory holding no committed file is vendored third-party content
 * this repo neither wrote nor maintains, and its prose documents an external tool's own API
 * rather than Shadowcat's. Scoping by a directory-NAME pattern instead would encode the wrong
 * reason and silently drop a future first-party skill named anything else.
 *
 * @param {string} repoRoot - absolute path to the repository root.
 * @returns {{ tracked: Set<string>, untracked: string[] }|null} tracked and untracked directory
 *   names under the skill roots, or null when git cannot answer (no checkout, or no git).
 */
export function listSkillDirs(repoRoot) {
  const tracked = new Set();
  for (const root of MD_ROOTS) {
    let listed;
    try {
      listed = execFileSync("git", ["ls-files", "-z", "--", root], {
        cwd: repoRoot,
        encoding: "utf8",
        maxBuffer: 64 * 1024 * 1024,
      });
    } catch {
      return null;
    }
    for (const entry of listed.split("\0")) {
      if (entry === "") continue;
      const rel = norm(entry).slice(norm(root).length + 1);
      if (rel.includes("/")) tracked.add(rel.split("/")[0]);
    }
  }
  const untracked = [];
  for (const root of MD_ROOTS) {
    const abs = join(repoRoot, ...root.split("/"));
    for (const entry of readdirSync(abs, { withFileTypes: true })) {
      if (entry.isDirectory() && !tracked.has(entry.name)) untracked.push(entry.name);
    }
  }
  return { tracked, untracked };
}

// `shadowcat-codebase-nightfox` documents a subsystem split across TWO repositories: this engine
// repo owns `@shadowcat/formula`, but the Nightfox rules-engine/sheets code the rest of the skill
// documents lives in a SEPARATE repository, nested into a dev checkout only (not present in this
// one, verified: `src/modules/nightfox` does not exist here). A citation of a Nightfox-repo symbol
// is therefore structurally unresolvable from this tree regardless of whether it is accurate —
// the same "genuinely cannot be gated" shape RULE 16 already accepted for the lowercase-hyphenated
// marker, and handled the same way: excluded from the FATAL scan, but its count is never silently
// dropped (see `checkSkillSymbolRefs`'s `nightfoxExcluded`), and it is recorded beside RULE 15 in
// this repo's doc-sweep truthfulness rules as a stated review obligation awaiting a ruling, not a
// self-authorized exemption.
const NIGHTFOX_SKILL_DIR = "shadowcat-codebase-nightfox";

/**
 * Finds every markdown file under the tracked skill directories, excluding
 * `NIGHTFOX_SKILL_DIR` (see its comment) unless `includeNightfox` is set.
 * @param {string} repoRoot - absolute path to the repository root.
 * @param {Set<string>} trackedDirs - tracked directory names, from `listSkillDirs`.
 * @param {{includeNightfox?: boolean}} [opts] - set `includeNightfox` to keep the cross-repo skill.
 * @returns {string[]} absolute paths, sorted.
 */
export function findMarkdownFiles(repoRoot, trackedDirs, { includeNightfox = false } = {}) {
  const out = [];
  for (const root of MD_ROOTS) {
    const abs = join(repoRoot, ...root.split("/"));
    for (const entry of readdirSync(abs, { withFileTypes: true })) {
      if (!entry.isDirectory() || !trackedDirs.has(entry.name)) continue;
      if (!includeNightfox && entry.name === NIGHTFOX_SKILL_DIR) continue;
      out.push(...sources(join(abs, entry.name), MD_EXTS));
    }
  }
  return out.sort();
}

// A citation candidate is the ENTIRE content of an inline code span, once trimmed, matching this
// shape: an identifier, optionally chained by `::` or `.` segments (`Owner::member`,
// `Owner.member`), optionally followed by `()` (a call-site citation, `find()`). Anchored full-
// match, not a substring search — a span containing anything else (whitespace, `<>`, `&`, `:`
// single-colon type annotations, `/` paths) is a TYPE SNIPPET, CONFIG VALUE, PATH or PROSE
// PHRASE, not a symbol citation, and is counted as a non-candidate rather than guessed at.
const CITATION_SHAPE =
  /^[A-Za-z_][A-Za-z0-9_]*(?:(?:::|\.)[A-Za-z_][A-Za-z0-9_]*)*(?:\(\))?$/;

// A token ending in a recognized non-code file extension (`CLAUDE.md`, `_semantic.scss`,
// `typedoc.json`) is a FILENAME cited as a value or a durable-document pointer — RULE 15/16
// already govern that citation shape on their own terms — not a code-symbol citation, and is
// counted as a non-candidate rather than checked against the symbol index.
const FILE_EXTENSION_TOKEN =
  /\.(?:md|scss|json|toml|ya?ml|py|txt|html?|svg|png|db|sql|rs|m?[jt]s|cjs|lock)$/i;

/**
 * Extracts every Markdown code span in a document, by CommonMark's own rule: a run of N backticks
 * opens a span that closes at the next run of exactly N, and a span may cross a line break. A
 * fixed single-backtick, line-scoped pattern gets this wrong twice over. It misses the longer-run
 * form — the construction that exists precisely to quote a span CONTAINING backticks — and
 * swallows the real citations inside it, which land in the gap between two matches, a bucket
 * nothing counts. And a span WRAPPED across a line break (this corpus wraps at a fixed column, so
 * they are common) leaves an odd backtick on each of two lines, which re-pairs every later span on
 * those lines against the wrong delimiter and yields tokens the prose never wrote.
 *
 * @param {string} text - Markdown prose, fences already stripped.
 * @returns {{content: string, index: number}[]} each span's raw content and start offset.
 */
export function extractCodeSpans(text) {
  const runs = [];
  for (const m of text.matchAll(/`+/g)) runs.push({ index: m.index, len: m[0].length });
  const spans = [];
  let i = 0;
  while (i < runs.length) {
    const open = runs[i];
    let j = i + 1;
    while (j < runs.length && runs[j].len !== open.len) j += 1;
    if (j >= runs.length) {
      i += 1;
      continue;
    }
    spans.push({ content: text.slice(open.index + open.len, runs[j].index), index: open.index });
    i = j + 1;
  }
  return spans;
}

/**
 * Every token a document offers for classification, each with the 1-based line its span opens on.
 * A span whose content itself contains backticks is a longer-run span quoting other spans: it
 * yields BOTH itself (its full content is prose, and counting it is what keeps every span
 * accounted for) and each span nested inside it, which are the real citations.
 * @param {string} text - Markdown prose, fences already stripped.
 * @returns {{token: string, line: number}[]} trimmed span contents, nested spans included.
 */
export function citationTokens(text) {
  const lineStarts = [0];
  for (let i = 0; i < text.length; i += 1) if (text[i] === "\n") lineStarts.push(i + 1);
  const lineOf = (index) => {
    let lo = 0;
    let hi = lineStarts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (lineStarts[mid] <= index) lo = mid;
      else hi = mid - 1;
    }
    return lo + 1;
  };
  const out = [];
  for (const span of extractCodeSpans(text)) {
    const line = lineOf(span.index);
    out.push({ token: span.content.trim(), line });
    if (span.content.includes("`"))
      for (const inner of extractCodeSpans(span.content))
        out.push({ token: inner.content.trim(), line });
  }
  return out.filter((t) => t.token !== "");
}

// Inline code spans only — fenced ``` blocks are illustrative snippets (BAD/GOOD examples, shell
// commands) rather than point citations, and are stripped before span extraction runs.

/** Strips fenced ``` code blocks (any info string) from Markdown text, replacing each with blank
 * lines so line numbers in the caller's reporting stay aligned with the original file. */
export function stripFences(text) {
  return text.replace(/```[\s\S]*?```/g, (m) => m.replace(/[^\n]/g, ""));
}

/**
 * Splits every code span in one skill file's prose (fenced blocks excluded) into two buckets:
 * `candidates` (resolved against the symbol index, and the gate fails on an unresolved one) and
 * `nonCandidates` (a span whose shape rules it out entirely — a path, a type snippet, a prose
 * phrase). Shape decides only whether a span is a citation AT ALL; it never decides whether a
 * citation-shaped token gets checked. Nothing is silently dropped: both buckets are numbers the
 * caller must print.
 *
 * @param {string} text - the skill file's raw contents.
 * @returns {{ candidates: {line: number, token: string}[], nonCandidates: number }}
 */
export function extractCitationCandidates(text) {
  const body = stripFences(text);
  const lines = body.split("\n");
  const candidates = [];
  let nonCandidates = 0;
  for (const { token, line } of citationTokens(body)) {
    // The EXAMPLE marker exempts the line a span OPENS on: that is the line whose author wrote the
    // specimen, and a span is only ever a specimen because of the sentence introducing it.
    if (EXAMPLE_EXEMPT.test(lines[line - 1] ?? "")) continue;
    if (!CITATION_SHAPE.test(token) || FILE_EXTENSION_TOKEN.test(token)) {
      nonCandidates += 1;
      continue;
    }
    candidates.push({ line, token });
  }
  return { candidates, nonCandidates };
}

// A citation-shaped token that is legitimately not a symbol this tree declares: an external
// crate/std/Web API name, a wire/serde keyword, or an ordinary English word set in code font.
// Named and reasoned, exactly like `check-comment-refs.mjs`'s ACKNOWLEDGED list, and for the same
// reason: an unresolved candidate must be counted and either verified, acknowledged, or reported
// as broken — never silently dropped. Every entry is HIT-COUNTED on each run and a zero-hit entry
// fails the gate, so the list cannot quietly grow a slot that absorbs a future defect.
export const ACKNOWLEDGED_NON_SYMBOLS = new Set([
  // Rust primitive/std types this repo's own code never declares — cited as VALUES.
  "f64", "u32", "i128", "Vec", "Option", "Some", "Result", "Err", "HashSet", "BTreeMap",
  "Uuid", "Arc", "Mutex", "Rc", "RefCell", "Sync", "Display", "PartialEq", "Path", "PathBuf",
  "SqlitePool", "SyntaxError", "Sink", "ConnectInfo", "JoinSet", "E",
  // Rust std method/associated-fn names cited standalone (`checked_add`, `as_ref`) — this repo
  // declares no method of these names on its own types at the point cited.
  "checked_add", "checked_mul", "saturating_mul", "is_ascii_graphic",
  "max_by_key", "min_by_key", "binary_search_by_key", "remove_dir_all", "as_ref", "is_some",
  "filter_map",
  // TS/JS standard and Web globals cited without meaning "this repo declares it".
  "undefined", "Array", "Map", "Window", "IntersectionObserver", "structuredClone",
  // Literal/keyword values, not declarations.
  "true", "false", "NaN", "Infinity",
  // Non-Rust/TS/Svelte source this checker's symbol index does not cover (Python hook internals,
  // SQL/OS keywords, HTTP method/header names).
  "SUBSYSTEMS", "file_path", "NOCASE", "Referer", "GET", "POST", "SO_SNDBUF", "SO_RCVBUF",
  "EXEMPT", "TODO",
  // `#[serde(...)]` / rustdoc attribute keywords — serde's and rustdoc's own vocabulary, not a
  // symbol this repo declares.
  "deny_unknown_fields", "skip_serializing_if", "no_run",
  // Third-party UI libraries this repo depends on but does not declare (dockview panel library,
  // TypeDoc's own internals) — their own type/class names, cited as VALUES.
  "DockviewComponent", "PopoutWindowService", "AsapEvent", "DockviewApi", "PanelApiImpl",
  "Overlay", "Options", "Vec2",
  // `serde_json::Number`'s own PRIVATE variant names (`PosInt`/`NegInt`/`Float`), cited in a
  // comment describing serde_json's internal representation split — not a type this repo declares.
  "PosInt", "NegInt", "Float",
  // Further external crate/Web-API types this repo cites as values (`hyper`'s `Host` header
  // enum, `url`'s `Url`, PixiJS's `Link` filter, the `ammonia` HTML-sanitizer's `PassThrough`
  // element-handling mode, `polyanya`'s `Mesh`/`Layer`).
  "Host", "Link", "PassThrough", "SqlSafeStr", "Date", "DefaultBodyLimit", "Url", "Mesh", "Layer",
  // Durable-tracker filenames RULE 15/16 already govern on their own terms (a "Pointers"-section
  // citation of a durable doc by name), not a code-symbol citation.
  "OPEN_BUGS", "CLOSED_BUGS",
  // The `EXAMPLE:` marker word itself, cited by `shadowcat-codebase-core` while explaining the
  // marker convention — not a code symbol.
  "EXAMPLE",
  // Web-platform API names: a DOM property, event handler or global this repo calls but does not
  // declare. `randomUUID` is the tail of `crypto.randomUUID()`.
  "innerHTML", "onscroll", "onclick", "appendChild", "queueMicrotask", "encodeURIComponent",
  "getBoundingClientRect", "fetch", "randomUUID",
  // dockview-core's own API surface (the vendored panel library): methods and events this repo
  // calls or subscribes to, all declared inside the dependency.
  "addStyles", "addPopoutGroup", "onDidRemovePopoutGroup", "onDidRemovePanel", "onDidLayoutChange",
  "onDidDimensionsChange", "onDidChangeEnd", "mutation", "getNextGroupId",
  // Test-framework API: a Vitest matcher and a Playwright locator method, named while describing
  // what a test can and cannot observe.
  "toBe", "boundingBox",
  // Language keywords cited as SYNTAX — the shape of a control-flow construct or declaration
  // form, not a name anything declares.
  "if", "continue", "catch", "finally", "await", "enum",
  // Grammar vocabulary from a standard this repo consumes: URL syntax terms and JSON-Schema
  // combinator keywords, cited while describing what a validator does and does not accept.
  "https", "userinfo", "anyOf", "oneOf",
  // A command or filesystem syscall named in prose while explaining what the code does NOT
  // shell out to, or what a single atomic operation is.
  "cargo", "xcopy", "robocopy", "rename",
  // A transitive dependency this repo never names in its own manifest, or a cargo FEATURE of one
  // (`polyanya`'s `async`/`recast`) — declared in the dependency graph, not in this tree.
  "spade", "i_overlay", "recast", "async",
  // An external crate's method, called on that crate's own builder type (`ammonia`'s URL-scheme
  // allowlist setter).
  "url_schemes",
  // A tooling option this repo cites while explaining another tool's behaviour but never sets
  // itself (TypeDoc's per-package option map), the plugin-cache state keys of the agent harness
  // (a file outside this repo), and the TypeScript config concept named as a whole.
  "packageOptions", "lastUpdated", "installedAt", "tsconfig",
  // Dice-notation operator tokens: notation the lexer ACCEPTS, spelled the way an author types
  // it, not an identifier the tree declares.
  "kh", "e3",
  // A derived quantity the implementation names only in its own explanatory comment — no binding
  // carries the name, so prose and code agree on a term the index cannot hold.
  "minConsecutiveDelta", "tMax",
  // Deliberate NEGATIVE citations, each in a sentence asserting the named thing does NOT exist:
  // no back-compat `*System` aliases (verified against `src/server/src/data/engine/`, which
  // declares none of them), no sidebar `tab` field on a `Contribution`, no `TabbedSurface`
  // component, no `sys_f64` pointer-walk helper, no bespoke `get_pointer`, no polyanya
  // `blocked_layers` mechanism, and no `rand` dependency (determinism by construction). The
  // citation's own sentence is what makes the claim true, not a pointer to a live declaration.
  "ActorSystem", "TokenSystem", "FactionRegistrySystem", "ConditionRegistrySystem", "RegionSystem",
  "tab", "TabbedSurface", "sys_f64", "get_pointer", "blocked_layers", "rand",
]);

// Regex forms, for a qualified path whose PREFIX names a known external crate/package rather
// than this repo's own code — checking a full qualified token against a flat Set would need one
// entry per method the crate exposes, which is unbounded; the crate/package NAME is the bounded,
// reasoned thing to acknowledge.
export const ACKNOWLEDGED_EXTERNAL_PREFIX = new Set([
  "tokio", "clap", "reqwest", "serde_json", "ammonia", "polyanya",
]);

/**
 * Resolves one citation token against the symbol index.
 *
 * A single identifier must resolve exactly. A `::`/`.` chain resolves at its LONGEST indexed
 * prefix, after which every remaining segment must resolve as a declared name on its own. The
 * prefix is what carries the owner relation the index actually knows; the remainder is a hop the
 * index cannot follow, because a member's declared TYPE lives in another file and no syntax-level
 * index resolves a type reference (`AppContext.chat` is declared `chat: ChatApi`, so `send` is
 * declared under `ChatApi`, never under `chat`).
 *
 * When no multi-segment prefix is indexed the head is skipped — a lowercase head is a VALUE (a
 * local, a parameter, a wire-document root) that nothing declares, and requiring it would report
 * this repo's established `doc.engine`/`ctx.documents` path notation as broken. A CAPITALIZED head
 * names a type, so an unindexed one is itself a dead citation and fails, which is what keeps a
 * fabricated `MadeUpType::Member` from passing on its member name alone.
 *
 * @param {string} token - the citation, without any trailing `()`.
 * @param {Set<string>} symbols - the built symbol index.
 * @returns {boolean} whether the tree declares what this citation names.
 */
export function resolvesAgainstIndex(token, symbols) {
  if (symbols.has(token)) return true;
  const segments = token.split(/::|\./);
  if (segments.length === 1) return false;
  let prefix = 1;
  for (let i = segments.length - 1; i >= 2; i -= 1) {
    const head = segments.slice(0, i);
    if (symbols.has(head.join(".")) || symbols.has(head.join("::"))) {
      prefix = i;
      break;
    }
  }
  if (prefix === 1 && /^[A-Z]/.test(segments[0]) && !symbols.has(segments[0])) return false;
  return segments.slice(prefix).every((s) => symbols.has(s));
}

/**
 * Resolves every citation candidate in one skill file's text against `symbols`, classifying each
 * as verified, acknowledged (a named non-symbol token), or broken. Every acknowledgement records
 * WHICH list entry absorbed it in `hits`, so the caller can fail a list entry that absorbs
 * nothing.
 *
 * @param {string} text - the skill file's raw contents.
 * @param {Set<string>} symbols - the built symbol index.
 * @param {Map<string, number>} [hits] - acknowledgement-entry hit counter, accumulated across files.
 * @returns {{ verified: number, acknowledged: number, broken: {line: number, token: string}[], nonCandidates: number }}
 */
export function checkFileCitations(text, symbols, hits = new Map()) {
  const { candidates, nonCandidates } = extractCitationCandidates(text);
  let verified = 0;
  let acknowledged = 0;
  const broken = [];
  const bump = (entry) => hits.set(entry, (hits.get(entry) ?? 0) + 1);
  for (const { line, token } of candidates) {
    const bare = token.endsWith("()") ? token.slice(0, -2) : token;
    if (resolvesAgainstIndex(bare, symbols)) {
      verified += 1;
      continue;
    }
    const segments = bare.split(/::|\./);
    const lastSegment = segments[segments.length - 1];
    const entry = ACKNOWLEDGED_NON_SYMBOLS.has(bare)
      ? bare
      : segments.length > 1
        ? [
            ACKNOWLEDGED_EXTERNAL_PREFIX.has(segments[0]) ? segments[0] : null,
            ACKNOWLEDGED_NON_SYMBOLS.has(segments[0]) ? segments[0] : null,
            ACKNOWLEDGED_NON_SYMBOLS.has(lastSegment) ? lastSegment : null,
          ].find((e) => e !== null) ?? null
        : null;
    if (entry !== null) {
      acknowledged += 1;
      bump(entry);
      continue;
    }
    broken.push({ line, token });
  }
  return { verified, acknowledged, broken, nonCandidates };
}

/**
 * Runs the full gate: builds the symbol index, scopes the corpus to the tracked skill
 * directories, then checks every markdown file in it.
 *
 * @param {string} repoRoot - absolute path to the repository root.
 * @param {{trackedDirs?: Set<string>, untrackedDirs?: string[]}} [opts] - corpus scoping override,
 *   for a fixture tree that is not itself a git checkout; production passes nothing and asks git.
 * @returns {{ filesScanned: number, filesIndexed: number, symbolCount: number,
 *   candidatesChecked: number, verified: number, acknowledged: number,
 *   broken: {file: string, line: number, token: string}[], nonCandidates: number,
 *   acknowledgedHits: Map<string, number>, unusedAcknowledgements: string[],
 *   untrackedDirs: string[], nightfoxExcludedFiles: number, nightfoxExcludedBroken: number }}
 */
export function checkSkillSymbolRefs(repoRoot, opts = {}) {
  let { trackedDirs, untrackedDirs } = opts;
  if (trackedDirs === undefined) {
    const dirs = listSkillDirs(repoRoot);
    if (dirs === null)
      throw new Error(
        "git could not list the skill corpus: this gate scopes by tracked-ness and cannot " +
          "guess it. Run it inside a git checkout with git on PATH.",
      );
    trackedDirs = dirs.tracked;
    untrackedDirs = dirs.untracked;
  }
  const { symbols, filesIndexed } = buildSymbolIndex(repoRoot);
  const files = findMarkdownFiles(repoRoot, trackedDirs);
  const acknowledgedHits = new Map();
  let verified = 0;
  let acknowledged = 0;
  let nonCandidates = 0;
  const broken = [];
  for (const file of files) {
    const result = checkFileCitations(readFileSync(file, "utf8"), symbols, acknowledgedHits);
    verified += result.verified;
    acknowledged += result.acknowledged;
    nonCandidates += result.nonCandidates;
    for (const b of result.broken) broken.push({ file, ...b });
  }

  // The excluded Nightfox skill still gets counted — never silently dropped — as its own
  // structurally-unresolvable review obligation, per `NIGHTFOX_SKILL_DIR`'s comment. Its
  // acknowledgements are deliberately NOT counted into `acknowledgedHits`: an entry that only
  // ever fires on the excluded file is dead as far as the gated corpus is concerned.
  const nightfoxFiles = findMarkdownFiles(repoRoot, trackedDirs, { includeNightfox: true }).filter(
    (f) => !files.includes(f),
  );
  let nightfoxExcludedBroken = 0;
  for (const file of nightfoxFiles)
    nightfoxExcludedBroken += checkFileCitations(readFileSync(file, "utf8"), symbols).broken.length;

  const unusedAcknowledgements = [
    ...ACKNOWLEDGED_NON_SYMBOLS,
    ...ACKNOWLEDGED_EXTERNAL_PREFIX,
  ].filter((entry) => !acknowledgedHits.has(entry));

  return {
    filesScanned: files.length,
    filesIndexed,
    symbolCount: symbols.size,
    candidatesChecked: verified + acknowledged + broken.length,
    verified,
    acknowledged,
    broken,
    nonCandidates,
    acknowledgedHits,
    unusedAcknowledgements,
    untrackedDirs: untrackedDirs ?? [],
    nightfoxExcludedFiles: nightfoxFiles.length,
    nightfoxExcludedBroken,
  };
}
