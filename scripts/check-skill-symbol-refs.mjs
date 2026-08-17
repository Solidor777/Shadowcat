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
// exactly one printed bucket: verified, acknowledged non-symbol, cross-repo, broken,
// EXAMPLE-exempt, or not citation-shaped. `EXAMPLE-exempt` is a printed count for the same reason
// as the rest: an exclusion nobody counts is a backdoor, and a line carrying both a specimen and a
// real citation would otherwise take the citation out of the gate with nothing in the output
// changing.
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
//
// They are registered ONLY under the `fn` that declares them (`execute_move::chord_ok`), never
// bare. A function-local name is invisible outside its function, so a bare entry carries no owner
// relation while the rest of the index does: it makes the index answer "yes, the tree declares
// that" to any citation spelling any local anywhere, which is how a renamed field stays green
// because some unrelated function happens to bind the old name. Owner-qualification is also what
// RULE 15 asks the citation itself to do.
const RUST_LET_BINDING = /\blet\s+(?:mut\s+)?([a-z_][A-Za-z0-9_]*)/g;
const RUST_LET_TUPLE = /\blet\s+\(([^)]*)\)\s*=/g;
// A `for` pattern binds names exactly as a `let` does — `for (next, sc, parity) in …` is where
// `astar_leg`'s per-step cost gets its name — and prose cites them the same way.
const RUST_FOR_BINDING = /\bfor\s+(?:\(([^)]*)\)|(?:mut\s+)?([a-z_][A-Za-z0-9_]*))\s+in\b/g;
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
  // The `fn` a local name belongs to. Stacked because a `fn` may nest inside another; an entry is
  // popped when a sibling declaration opens at the same depth, which is what ends a body-less
  // trait method's scope.
  const fnStack = []; // { name, depth }[]
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
        if (kind === "fn") {
          while (fnStack.length > 0 && depth <= fnStack[fnStack.length - 1].depth) fnStack.pop();
          fnStack.push({ name, depth });
        }
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
      // A STRUCT-VARIANT's own fields sit one level deeper than its variant name, and are real
      // declarations the wire carries (`TokenVisual::Faces`'s `#[serde(rename = "faceMap")]` is the
      // tree's only spelling of `faceMap`). Only the field's EXPLICIT rename applies here: a
      // container's `rename_all` renames its VARIANTS, not a variant's fields.
      const inVariantBody =
        container !== null && container.kind === "enum" && depth === container.depth + 2;
      if (container !== null && (depth === container.depth + 1 || inVariantBody)) {
        const pattern =
          container.kind === "struct" || inVariantBody ? RUST_STRUCT_FIELD : RUST_ENUM_VARIANT;
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
            container.renameAll === null || inVariantBody
              ? null
              : applyRenameAll(member, container.renameAll),
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
      // Owned by the innermost enclosing `fn`; a local outside every `fn` has no owner to cite it
      // by and is not indexed at all, rather than being indexed bare.
      const owner = fnStack.length === 0 ? null : fnStack[fnStack.length - 1].name;
      const addLocal = (name) => {
        if (owner !== null) addQualified(`${owner}::${name}`);
      };
      for (const m of line.matchAll(RUST_LET_BINDING)) addLocal(m[1]);
      for (const m of line.matchAll(RUST_LET_TUPLE))
        for (const part of m[1].split(","))
          for (const id of part.trim().matchAll(/\b([a-z_][A-Za-z0-9_]*)\b/g)) addLocal(id[1]);
      for (const m of line.matchAll(RUST_FOR_BINDING)) {
        if (m[2] !== undefined) addLocal(m[2]);
        else
          for (const part of m[1].split(","))
            for (const id of part.trim().matchAll(/\b([a-z_][A-Za-z0-9_]*)\b/g)) addLocal(id[1]);
      }
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
        for (const m of rest.matchAll(RUST_FN_PARAM)) addLocal(m[1]);
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
        while (fnStack.length > 0 && depth <= fnStack[fnStack.length - 1].depth) fnStack.pop();
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
 * The array literal a module-level declaration is initialized to, seen through the wrappers a
 * value set is normally written behind (`as const`, a type assertion, parentheses); null for
 * anything else.
 *
 * A collection CONSTRUCTOR (`new Set([...])`) is deliberately NOT unwrapped. This gate's own
 * acknowledgement lists are declared that way and `scripts/` is an indexed root, so unwrapping
 * makes every acknowledged non-symbol index itself as a declared name — the index would then
 * resolve exactly the tokens the list exists to flag, and the list would go dead while the gate
 * reported everything verified.
 * @param {ts.Node|undefined} node - a variable declaration's initializer.
 * @returns {ts.ArrayLiteralExpression|null} the array literal, or null.
 */
function arrayLiteralInitializer(node) {
  let cur = node;
  while (
    cur !== undefined &&
    (ts.isAsExpression(cur) || ts.isTypeAssertionExpression(cur) || ts.isParenthesizedExpression(cur))
  )
    cur = cur.expression;
  return cur !== undefined && ts.isArrayLiteralExpression(cur) ? cur : null;
}

/**
 * Extracts every symbol name one TS/JS module declares: top-level `function`/`class`/`interface`/
 * `type`/`enum`/`namespace` declarations and module-level `const`/`let`/`var` bindings (exported
 * or not — a skill may legitimately cite a module-private symbol); every class, interface,
 * type-literal and enum MEMBER at any nesting depth, indexed bare and under every suffix of its
 * owner chain (`UiState.global.lastWorld`, `global.lastWorld`, `lastWorld`) so both the
 * declaration-citation shape and this repo's wire-path notation resolve at whatever depth prose
 * cites them; every name a named re-export publishes (`export { a as b }`,
 * `export type { Foo }`); every name the module IMPORTS, which is this tree's own declaration
 * that the name exists in a package it depends on; every identifier-shaped string LITERAL
 * TYPE, which is how a wire-protocol discriminant value is declared (`type SyncState = "none" |
 * "up_to_date"`); and every identifier-shaped string member of a MODULE-LEVEL array literal, which
 * is the other way this tree declares a closed value set (`NOTATION_KEYWORDS`).
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
  // The owner-qualified forms ONLY. A name with an empty chain reaches nothing: an anonymous
  // callback's parameter has no owner a citation could name, so it is indexed nowhere rather than
  // bare.
  const addOwned = (name, chain) => {
    if (name === "" || !IDENTIFIER_SHAPED.test(name)) return;
    for (let i = 0; i < chain.length; i += 1) names.add(`${chain.slice(i).join(".")}.${name}`);
  };

  const walk = (node, chain, inFunction) => {
    let nextChain = chain;
    // Everything declared below a function-like node is function-local, at any depth.
    const nextInFunction =
      inFunction ||
      ts.isFunctionDeclaration(node) ||
      ts.isFunctionExpression(node) ||
      ts.isArrowFunction(node) ||
      ts.isMethodDeclaration(node) ||
      ts.isConstructorDeclaration(node) ||
      ts.isGetAccessorDeclaration(node) ||
      ts.isSetAccessorDeclaration(node);
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
        nextChain = [named];
      }
    } else if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name)) {
      if (inFunction) {
        addOwned(node.name.text, chain);
        nextChain = [...chain, node.name.text];
      } else {
        add(node.name.text, chain);
        nextChain = [node.name.text];
      }
      // A module-level array literal of strings DECLARES a closed set of accepted values —
      // `NOTATION_KEYWORDS`'s dice-notation prefixes are the tree's only spelling of `kh`/`cs`/`cf`
      // — exactly as a Rust string-literal alternation does on the server side, and prose cites a
      // member of that set by its literal spelling. Registered bare, like a string-literal union
      // member, since the set's own name is not part of how the value is written. Restricted to a
      // MODULE-LEVEL declaration: an array built inside a function is a local value, not a
      // declaration the tree publishes.
      if (
        node.parent?.parent?.parent !== undefined &&
        ts.isSourceFile(node.parent.parent.parent)
      ) {
        const literal = arrayLiteralInitializer(node.initializer);
        if (literal !== null) {
          for (const element of literal.elements)
            if (ts.isStringLiteral(element)) add(element.text, []);
        } else if (node.initializer !== undefined && ts.isStringLiteral(node.initializer)) {
          // The same rationale at cardinality one: `ITEM_DOC_TYPE = "item"` is the tree's only
          // spelling of that wire value, and prose cites the VALUE, not the constant holding it.
          add(node.initializer.text, []);
        }
      }
    } else if (ts.isBindingElement(node) && ts.isIdentifier(node.name)) {
      if (inFunction) addOwned(node.name.text, chain);
      else add(node.name.text, []);
    } else if (ts.isPropertyAssignment(node) || ts.isShorthandPropertyAssignment(node)) {
      const named = declaredMemberName(node.name);
      if (named !== "") {
        if (inFunction) addOwned(named, chain);
        else add(named, chain);
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
      // A constructor parameter property declares a real class member and is indexed as one; a
      // plain parameter is function-local and is owned by the function it belongs to.
      if (node.modifiers === undefined) addOwned(declaredMemberName(node.name), chain);
      else add(declaredMemberName(node.name), chain);
    } else if (ts.isExportSpecifier(node) || ts.isImportSpecifier(node)) {
      add(node.name.text, []);
    } else if (ts.isNamespaceImport(node) || ts.isNamespaceExport(node)) {
      add(node.name.text, []);
    } else if (ts.isImportClause(node) && node.name !== undefined) {
      add(node.name.text, []);
    } else if (ts.isLiteralTypeNode(node) && ts.isStringLiteral(node.literal)) {
      add(node.literal.text, []);
    }
    ts.forEachChild(node, (child) => walk(child, nextChain, nextInFunction));
  };
  ts.forEachChild(source, (child) => walk(child, [], false));
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
// repo owns `@shadowcat/formula` (at `src/client/formula/`), while the Nightfox rules-engine and
// sheets code the rest of the skill documents lives in a SEPARATE repository, nested into a dev
// checkout only (`src/modules/nightfox` does not exist here). Its markdown is scanned like every
// other skill file: RESOLUTION decides per citation, never membership in a file. Excluding the
// whole file left this repo's OWN symbols unchecked purely because they shared a page with names
// this tree cannot see - the same defect, in a different exemption, that dropping the shape
// exclusion removed from the classifier.
const NIGHTFOX_SKILL_DIR = "shadowcat-codebase-nightfox";

// The names the SEPARATE Nightfox repository declares, cited by the skill that documents it. Each
// is verified absent from this tree - which is the half of the claim this checkout can settle -
// and is acknowledged ONLY in that skill's own file, so a citation of one anywhere else is still
// reported broken. Hit-counted and zero-hit-fatal like every other acknowledgement list, so an
// entry dies the day its citation goes away. This remains a standing review obligation rather than
// a settled exemption: nothing here can be checked against the repository that owns it.
export const ACKNOWLEDGED_CROSS_REPO = new Set([
  // Rules-engine entry points and result types.
  "parseNightfox", "resolveNightfox", "ResolveWarning", "Collected.docs", "statRefResolver",
  "isParseError",
  // The stat/modifier data model and its caps.
  "Stat", "ModifierContribution", "MAX_STATS", "MAX_MODIFIERS",
  "RESERVED_STAT_KEYS", "validateStatKey", "maxBase", "maxFormula", "effectiveCurrent",
  "clampToMax", "mulAdditive",
  // Sheet components and the editing commands they issue.
  "StatRow", "StatTable", "ModifiersEditor", "EffectSheet", "addStat", "removeStat",
  "setStatOrder", "editStatField", "addModifier", "removeModifier", "editModifierField",
  "setMechanicsFlag", "buildStatRollContent", "effectReadOnly", "embedReadOnly",
  // The module's own document-type constant and i18n catalog.
  "EFFECT_DOC_TYPE", "NF_MESSAGES", "nfT",
  // The Nightfox document body's own bands, stat types and doc types, cited as the wire values
  // they are: the `mechanics` band and its `system.mechanics` path, the `resource` stat type, the
  // `effect` doc type and the `embedded.effect` slot that carries it, and the `format` module.
  "mechanics", "system.mechanics", "resource", "effect", "embedded.effect", "format",
]);

/**
 * Finds every markdown file under the tracked skill directories. No file is excluded: the
 * cross-repo skill is scanned like any other, and its unresolvable citations are acknowledged
 * per NAME via `ACKNOWLEDGED_CROSS_REPO`.
 * @param {string} repoRoot - absolute path to the repository root.
 * @param {Set<string>} trackedDirs - tracked directory names, from `listSkillDirs`.
 * @returns {string[]} absolute paths, sorted.
 */
export function findMarkdownFiles(repoRoot, trackedDirs) {
  const out = [];
  for (const root of MD_ROOTS) {
    const abs = join(repoRoot, ...root.split("/"));
    for (const entry of readdirSync(abs, { withFileTypes: true })) {
      if (!entry.isDirectory() || !trackedDirs.has(entry.name)) continue;
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

// A span that writes a name TOGETHER WITH its value (`MAX_GATE_WALK_SAMPLES=4096`,
// `autoUpdate = false`) is a citation of that name, not a config value: the value is there to
// save the reader a lookup. Without this, the whole span fails the citation shape and lands in
// `nonCandidates`, so the NAME goes unchecked — which is how a constant that no longer exists
// stayed cited with the gate reporting 0 broken. `=(?![=>])` is what keeps a comparison
// (`w <= 0`, `a == b`) and an arrow (`k => v`) out: neither writes a value to a name. A VALUE must
// actually follow, which is what keeps a bare syntax fragment (`` `href=` ``, quoted as the literal
// text a scan would match) from being read as a citation of the name in front of it. A VALUE is
// one token: a multi-token right-hand side is an expression or an illustrative formula
// (`attack = dex + str`), where the name on the left is the author's example rather than a
// citation of anything the tree declares.
const ASSIGNMENT_SHAPE =
  /^([A-Za-z_][A-Za-z0-9_]*(?:(?:::|\.)[A-Za-z_][A-Za-z0-9_]*)*)\s*=(?![=>])\s*\S+\s*$/;

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
 * Pairing is bounded at a BLANK LINE, which is CommonMark's own rule (a code span lives inside one
 * paragraph's inline content and cannot cross a paragraph break) and is what caps the blast radius
 * of an unpaired delimiter. Unbounded, a single stray backtick re-pairs every later span in the
 * file one delimiter off: the real citations land in the gaps between spans, which nothing counts,
 * while the prose between them becomes the spans — a file the gate reports 0 broken for while
 * checking almost none of it. Bounding costs the wrapped-span fix nothing, since a wrapped span
 * never crosses a blank line either.
 *
 * @param {string} text - Markdown prose, fenced and indented code blocks already stripped.
 * @returns {{content: string, index: number}[]} each span's raw content and start offset.
 */
export function extractCodeSpans(text) {
  const spans = [];
  // Paragraph boundaries: a run of one or more blank lines. Offsets stay absolute so the caller's
  // line attribution needs no adjustment.
  let paraStart = 0;
  const flush = (end) => {
    const para = text.slice(paraStart, end);
    const runs = [];
    for (const m of para.matchAll(/`+/g)) runs.push({ index: m.index, len: m[0].length });
    let i = 0;
    while (i < runs.length) {
      const open = runs[i];
      let j = i + 1;
      while (j < runs.length && runs[j].len !== open.len) j += 1;
      if (j >= runs.length) {
        i += 1;
        continue;
      }
      spans.push({
        content: para.slice(open.index + open.len, runs[j].index),
        index: paraStart + open.index,
      });
      i = j + 1;
    }
  };
  const blankLine = /^[ \t]*$/;
  let offset = 0;
  for (const line of text.split("\n")) {
    if (blankLine.test(line)) {
      flush(offset);
      paraStart = offset + line.length + 1;
    }
    offset += line.length + 1;
  }
  flush(text.length);
  return spans;
}

/**
 * Every token a document offers for classification, each with the 1-based line its span opens on.
 * A span whose content itself contains backticks is a longer-run span quoting other spans: it
 * yields BOTH itself (its full content is prose, and counting it is what keeps every span
 * accounted for) and each span nested inside it, which are the real citations. Nesting recurses to
 * any depth — a triple-run span quoting a double-run span quoting a citation would otherwise lose
 * the innermost token, which is the only real one, into no bucket at all.
 * @param {string} text - Markdown prose, fenced and indented code blocks already stripped.
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
  const emit = (content, line) => {
    out.push({ token: content.trim(), line });
    if (!content.includes("`")) return;
    for (const inner of extractCodeSpans(content)) emit(inner.content, line);
  };
  for (const span of extractCodeSpans(text)) emit(span.content, lineOf(span.index));
  return out.filter((t) => t.token !== "");
}

// Inline code spans only — a fenced or indented code BLOCK is an illustrative snippet (BAD/GOOD
// examples, shell commands) rather than a point citation, and is stripped before span extraction
// runs.

// A fence delimiter OPENS its line, up to leading indentation. Matching a triple-backtick run
// anywhere on a line pairs an INLINE mention of one (prose naming the ```ts fence marker) with the
// next real fence opener, blanking every line of prose between them and dropping its citations
// uncounted — a failure in the hiding direction, which no output distinguishes from clean prose.
const FENCE_DELIMITER = /^[ \t]*(?:```|~~~)/;
// A list item's CONTENT column: the marker's indentation plus the marker and the space after it.
// An indented code block inside a list item is measured from that column, not from the margin —
// without which every wrapped continuation paragraph of a nested bullet (this corpus is full of
// them) reads as a code block and its real citations vanish.
const LIST_MARKER = /^([ \t]*)([-*+]|\d+[.)])([ \t]+)/;

/**
 * Blanks every fenced and indented code BLOCK in Markdown text, keeping one blank line per removed
 * line so the caller's line attribution stays aligned with the original file.
 *
 * @param {string} text - one Markdown document's raw contents.
 * @returns {string} the same document with code-block lines emptied.
 */
export function stripCodeBlocks(text) {
  const lines = text.split("\n");
  const out = [];
  const listContent = [];
  let inFence = false;
  let inIndented = false;
  let indentedFrom = 0;
  let prevBlank = true;
  for (const line of lines) {
    if (inFence) {
      out.push("");
      if (FENCE_DELIMITER.test(line)) inFence = false;
      prevBlank = false;
      continue;
    }
    if (FENCE_DELIMITER.test(line)) {
      out.push("");
      inFence = true;
      prevBlank = false;
      continue;
    }
    if (line.trim() === "") {
      out.push(line);
      prevBlank = true;
      continue;
    }
    const indent = line.length - line.trimStart().length;
    if (inIndented) {
      if (indent >= indentedFrom) {
        out.push("");
        prevBlank = false;
        continue;
      }
      inIndented = false;
    }
    while (listContent.length > 0 && indent < listContent[listContent.length - 1])
      listContent.pop();
    const required = (listContent[listContent.length - 1] ?? 0) + 4;
    if (prevBlank && indent >= required) {
      inIndented = true;
      indentedFrom = required;
      out.push("");
      prevBlank = false;
      continue;
    }
    const marker = LIST_MARKER.exec(line);
    if (marker !== null) listContent.push(marker[1].length + marker[2].length + marker[3].length);
    out.push(line);
    prevBlank = false;
  }
  return out.join("\n");
}

/**
 * Splits every code span in one skill file's prose (code blocks excluded) into three buckets:
 * `candidates` (resolved against the symbol index, and the gate fails on an unresolved one),
 * `nonCandidates` (a span whose shape rules it out entirely — a path, a type snippet, a prose
 * phrase), and `exampleExempt` (a span on an EXAMPLE-marked line, deliberately a specimen rather
 * than a citation). Shape decides only whether a span is a citation AT ALL; it never decides
 * whether a citation-shaped token gets checked. Nothing is silently dropped: all three buckets are
 * numbers the caller must print, because an exclusion nobody counts is a backdoor — a future line
 * carrying both a specimen and a genuine citation would otherwise take the citation out of the
 * gate with nothing in the output changing.
 *
 * @param {string} text - the skill file's raw contents.
 * @returns {{ candidates: {line: number, token: string}[], nonCandidates: number,
 *   exampleExempt: number }}
 */
export function extractCitationCandidates(text) {
  const body = stripCodeBlocks(text);
  const lines = body.split("\n");
  const candidates = [];
  let nonCandidates = 0;
  let exampleExempt = 0;
  for (const { token, line } of citationTokens(body)) {
    // The EXAMPLE marker exempts the line a span OPENS on: that is the line whose author wrote the
    // specimen, and a span is only ever a specimen because of the sentence introducing it.
    if (EXAMPLE_EXEMPT.test(lines[line - 1] ?? "")) {
      exampleExempt += 1;
      continue;
    }
    const assigned = CITATION_SHAPE.test(token) ? null : ASSIGNMENT_SHAPE.exec(token)?.[1] ?? null;
    const cited = assigned ?? token;
    if ((!CITATION_SHAPE.test(cited) && assigned === null) || FILE_EXTENSION_TOKEN.test(cited)) {
      nonCandidates += 1;
      continue;
    }
    candidates.push({ line, token: cited });
  }
  return { candidates, nonCandidates, exampleExempt };
}

// A citation-shaped token that is legitimately not a symbol this tree declares: a wire/serde
// keyword, a literal value, an external member name cited standalone, or a deliberate negative
// citation. An entry matches the WHOLE token and nothing less - see `checkFileCitations` for why a
// segment-wise fallback is what lets a dead qualified citation pass as acknowledged. A name that
// OWNS members belongs in `ACKNOWLEDGED_EXTERNAL_PREFIX` instead.
//
// Named and reasoned, exactly like `check-comment-refs.mjs`'s ACKNOWLEDGED list, and for the same
// reason: an unresolved candidate must be counted and either verified, acknowledged, or reported
// as broken - never silently dropped. Every entry is HIT-COUNTED on each run and a zero-hit entry
// fails the gate, so the list cannot quietly grow a slot that absorbs a future defect.
export const ACKNOWLEDGED_NON_SYMBOLS = new Set([
  // Rust std method/associated-fn names cited standalone (`checked_add`, `as_ref`) - this repo
  // declares no method of these names on its own types at the point cited.
  "checked_add", "checked_mul", "saturating_mul", "is_ascii_graphic",
  "max_by_key", "min_by_key", "binary_search_by_key", "remove_dir_all", "filter_map",
  // Literal/keyword values, not declarations.
  "true", "false", "NaN", "Infinity",
  // Non-Rust/TS/Svelte source this checker's symbol index does not cover (Python hook internals,
  // SQL/OS keywords, HTTP method/header names).
  "SUBSYSTEMS", "file_path", "NOCASE", "Referer", "GET", "POST", "SO_SNDBUF", "SO_RCVBUF",
  "EXEMPT", "TODO",
  // A DIRECTORY name cited as a value in a tree-walk description, which RULE 15 governs on its
  // own terms exactly as it does a filename cited as a value.
  "node_modules",
  // `#[serde(...)]` / rustdoc attribute keywords - serde's and rustdoc's own vocabulary, not a
  // symbol this repo declares.
  "deny_unknown_fields", "skip_serializing_if", "no_run", "tag",
  // A language keyword whose backticks are load-bearing: the same sentence also uses "this" as an
  // ordinary pronoun, and only the code font tells the reader which one is meant.
  "this",
  // Durable-tracker filenames RULE 15/16 already govern on their own terms (a "Pointers"-section
  // citation of a durable doc by name), not a code-symbol citation.
  "OPEN_BUGS", "CLOSED_BUGS",
  // The `EXAMPLE:` marker word itself, cited by `shadowcat-codebase-core` while explaining the
  // marker convention - not a code symbol.
  "EXAMPLE",
  // Web-platform API names: a DOM property, event handler or global this repo calls but does not
  // declare.
  "innerHTML", "onscroll", "onclick", "appendChild", "queueMicrotask", "encodeURIComponent",
  "getBoundingClientRect", "isComposing",
  // A PixiJS `AnimatedSprite` property this repo SETS but does not declare.
  "autoUpdate",
  // dockview-core's own API surface (the vendored panel library): methods and events this repo
  // calls or subscribes to, all declared inside the dependency.
  "addStyles", "addPopoutGroup", "onDidRemovePopoutGroup", "onDidRemovePanel", "onDidLayoutChange",
  "onDidDimensionsChange", "onDidChangeEnd", "mutation", "getNextGroupId",
  // Test-framework API: a Vitest matcher and a Playwright locator method, named while describing
  // what a test can and cannot observe.
  "toBe", "boundingBox",
  // A JSON-Schema combinator keyword, cited beside its siblings while describing what a validator
  // accepts. The schema vocabulary is the standard's, not a declaration in this tree.
  "enum", "anyOf", "oneOf",
  // URL grammar terms from the standard the SSRF guard parses against.
  "https", "userinfo",
  // A request API named beside the crate this repo actually uses for the same job, and the
  // filesystem RENAME syscall, whose backticks are what mark the one atomic operation apart from
  // the same sentence's ordinary use of the word.
  "fetch", "rename",
  // A filesystem copy command named while explaining what this repo's code does NOT shell out to.
  "xcopy", "robocopy",
  // A transitive dependency this repo never names in its own manifest, and two cargo FEATURES of
  // `polyanya` that its dependency declaration turns off - declared in the dependency graph and in
  // that crate's manifest, not in this tree.
  "spade", "i_overlay", "recast", "async",
  // An external crate's method, called on that crate's own builder type (`ammonia`'s URL-scheme
  // allowlist setter).
  "url_schemes",
  // A tooling option this repo cites while explaining another tool's behaviour but never sets
  // itself (TypeDoc's per-package option map), the plugin-cache state keys of the agent harness
  // (a file outside this repo), and the TypeScript config concept named as a whole.
  "packageOptions", "lastUpdated", "installedAt", "tsconfig",
  // Composed dice notation - a keyword and its numeric argument written together - which no
  // single declaration in the tree spells. (The bare keywords it composes, `kh`/`cs`/`cf`, are
  // declared by `NOTATION_KEYWORDS` and resolve there.)
  "e3", "d20", "kh3",
  // A derived quantity the implementation names only in its own explanatory comment - no binding
  // carries the name, so prose and code agree on a term the index cannot hold.
  "minConsecutiveDelta",
  // Deliberate NEGATIVE citations, each in a sentence asserting the named thing does NOT exist:
  // no back-compat `*System` aliases (verified against `src/server/src/data/engine/`, which
  // declares none of them), no sidebar `tab` field on a `Contribution`, no `TabbedSurface`
  // component, no `sys_f64` pointer-walk helper, no bespoke `get_pointer`, no polyanya
  // `blocked_layers` mechanism, and no `rand` dependency (determinism by construction). The
  // citation's own sentence is what makes the claim true, not a pointer to a live declaration.
  "ActorSystem", "TokenSystem", "FactionRegistrySystem", "ConditionRegistrySystem", "RegionSystem",
  "tab", "TabbedSurface", "sys_f64", "get_pointer", "blocked_layers", "rand",
]);

// The name of something EXTERNAL that owns members: a crate, a package, or a type declared inside
// one. Acknowledges the bare name and any `::`/`.` path headed by it, because enumerating the
// members an external owner exposes is unbounded while the owner's own name is bounded and
// reasoned - the one place a segment-wise acknowledgement is defensible. Everything here is
// declared in a dependency, in the standard library, or by the web platform; nothing in this tree
// declares any of it.
export const ACKNOWLEDGED_EXTERNAL_PREFIX = new Set([
  // Crates this repo depends on, named as the owner of whatever they expose.
  "tokio", "clap", "reqwest", "serde_json", "ammonia", "polyanya",
  // Rust primitive and std types.
  "f64", "u32", "i128", "Vec", "Option", "Some", "Result", "Err", "HashSet", "BTreeMap",
  "Uuid", "Arc", "Mutex", "Rc", "RefCell", "Sync", "Display", "PartialEq", "Path", "PathBuf",
  "SqlitePool", "SyntaxError", "Sink", "ConnectInfo", "JoinSet", "E", "i32",
  // TS/JS standard and Web globals, and the `crypto` global whose members this repo calls.
  "undefined", "Array", "Map", "Window", "IntersectionObserver", "structuredClone", "crypto",
  // Third-party UI libraries this repo depends on but does not declare (the dockview panel
  // library, TypeDoc's own internals).
  "DockviewComponent", "PopoutWindowService", "AsapEvent", "DockviewApi", "PanelApiImpl",
  "Overlay", "Options", "Vec2",
  // `serde_json::Number`'s own PRIVATE variant names, cited while describing serde_json's internal
  // representation split.
  "PosInt", "NegInt", "Float",
  // Further external crate/Web-API types (`hyper`'s `Host` header enum, `url`'s `Url`, PixiJS's
  // `Link` filter, the `ammonia` HTML-sanitizer's `PassThrough` element-handling mode,
  // `polyanya`'s `Mesh`/`Layer`, `axum`'s `DefaultBodyLimit`).
  "Host", "Link", "PassThrough", "SqlSafeStr", "Date", "DefaultBodyLimit", "Url", "Mesh", "Layer",
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
 * @param {{extra?: Set<string>, extraHits?: Map<string, number>}} [scoped] - an additional
 *   whole-token acknowledgement set that applies to THIS file only, with its own hit counter;
 *   `ACKNOWLEDGED_CROSS_REPO` is passed this way for the skill that documents another repository.
 * @returns {{ verified: number, acknowledged: number, broken: {line: number, token: string}[],
 *   nonCandidates: number, exampleExempt: number, crossRepo: number }}
 */
export function checkFileCitations(text, symbols, hits = new Map(), scoped = {}) {
  const { candidates, nonCandidates, exampleExempt } = extractCitationCandidates(text);
  const { extra = new Set(), extraHits = new Map() } = scoped;
  let verified = 0;
  let acknowledged = 0;
  let crossRepo = 0;
  const broken = [];
  const bump = (entry) => hits.set(entry, (hits.get(entry) ?? 0) + 1);
  for (const { line, token } of candidates) {
    const bare = token.endsWith("()") ? token.slice(0, -2) : token;
    if (resolvesAgainstIndex(bare, symbols)) {
      verified += 1;
      continue;
    }
    if (extra.has(bare)) {
      crossRepo += 1;
      extraHits.set(bare, (extraHits.get(bare) ?? 0) + 1);
      continue;
    }
    // An `ACKNOWLEDGED_NON_SYMBOLS` entry matches the WHOLE token and nothing less. Absorbing a
    // qualified token on one of its segments is what lets `MadeUpType.fetch` — a type that does
    // not exist, or a member renamed away, which `resolvesAgainstIndex` correctly refused — be
    // reported as acknowledged instead of broken, which is the exact class this gate exists to
    // catch; it also keeps the entry alive vacuously, on a token unrelated to the reason the
    // entry's own comment records. Only `ACKNOWLEDGED_EXTERNAL_PREFIX` matches by HEAD segment,
    // because the name of an external owner is a bounded, reasoned acknowledgement of everything
    // that owner exposes, where enumerating its members is not.
    const segments = bare.split(/::|\./);
    const entry = ACKNOWLEDGED_EXTERNAL_PREFIX.has(segments[0])
      ? segments[0]
      : ACKNOWLEDGED_NON_SYMBOLS.has(bare)
        ? bare
        : null;
    if (entry !== null) {
      acknowledged += 1;
      bump(entry);
      continue;
    }
    broken.push({ line, token });
  }
  return { verified, acknowledged, broken, nonCandidates, exampleExempt, crossRepo };
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
 *   exampleExempt: number, crossRepo: number, filesWithNoCandidates: string[],
 *   acknowledgedHits: Map<string, number>, crossRepoHits: Map<string, number>,
 *   unusedAcknowledgements: string[], untrackedDirs: string[] }}
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
  const crossRepoHits = new Map();
  let verified = 0;
  let acknowledged = 0;
  let nonCandidates = 0;
  let exampleExempt = 0;
  let crossRepo = 0;
  const broken = [];
  const filesWithNoCandidates = [];
  for (const file of files) {
    const text = readFileSync(file, "utf8");
    // The cross-repo acknowledgements apply to the ONE skill that documents another repository,
    // and nowhere else: a citation of a Nightfox name in any other skill is still broken.
    const scoped = norm(file).includes(`/${NIGHTFOX_SKILL_DIR}/`)
      ? { extra: ACKNOWLEDGED_CROSS_REPO, extraHits: crossRepoHits }
      : {};
    const result = checkFileCitations(text, symbols, acknowledgedHits, scoped);
    verified += result.verified;
    acknowledged += result.acknowledged;
    nonCandidates += result.nonCandidates;
    exampleExempt += result.exampleExempt;
    crossRepo += result.crossRepo;
    for (const b of result.broken) broken.push({ file, ...b });
    // A per-FILE floor. The global `candidatesChecked === 0` guard cannot see a single file that
    // stopped yielding candidates - one stray delimiter, or a fence-pairing slip, silently takes
    // that file's whole prose out of the gate while every other file keeps the totals healthy.
    // A file that carries backticks at all must yield at least one classified span.
    if (
      result.verified + result.acknowledged + result.crossRepo + result.broken.length === 0 &&
      result.nonCandidates === 0 &&
      text.includes("`")
    )
      filesWithNoCandidates.push(file);
  }

  const unusedAcknowledgements = [
    ...ACKNOWLEDGED_NON_SYMBOLS,
    ...ACKNOWLEDGED_EXTERNAL_PREFIX,
  ]
    .filter((entry) => !acknowledgedHits.has(entry))
    .concat([...ACKNOWLEDGED_CROSS_REPO].filter((entry) => !crossRepoHits.has(entry)));

  return {
    filesScanned: files.length,
    filesIndexed,
    symbolCount: symbols.size,
    candidatesChecked: verified + acknowledged + crossRepo + broken.length,
    verified,
    acknowledged,
    broken,
    nonCandidates,
    exampleExempt,
    crossRepo,
    filesWithNoCandidates,
    acknowledgedHits,
    crossRepoHits,
    unusedAcknowledgements,
    untrackedDirs: untrackedDirs ?? [],
  };
}
