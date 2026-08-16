// Verifies every code-symbol citation in a `shadowcat-codebase-*` skill resolves to a symbol
// that actually exists in the tree. RULE 15 requires skill prose to cite symbols rather than
// file names or line numbers, on the argument that "a symbol breaks only on rename, which a grep
// finds" — nobody ran that grep, so two dead citations (a renamed test function, a shorthand that
// was never a real symbol) sat undetected. `check-skill-api-refs.mjs` verifies a DIFFERENT
// corpus — `/api/...` doc-site pointers against `dist-docs/` — and does not see this class at
// all. This script is the missing check for the class RULE 15 actually argues about: a backticked
// identifier or `Owner::member`/`Owner.member` path cited as if it names live code.
//
// Pure library: no top-level side effects. `check-skill-symbol-refs-cli.mjs` is the executable
// entry point.
//
// Cross-platform: node:path/node:fs only.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, basename } from "node:path";

const SKIP_DIRS = new Set(["node_modules", "dist", "target", ".git", "dist-docs"]);

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

/** Recursively collects paths matching `exts` under `dir`. Mirrors `check-comment-refs.mjs`'s
 * `sources`, kept as an independent copy because this walk additionally needs to exclude
 * `GENERATED_ROOT`, which the caller does by prefix rather than by a second parameter here. */
export function sources(dir, exts) {
  const out = [];
  for (const name of readdirSync(dir)) {
    if (SKIP_DIRS.has(name)) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...sources(p, exts));
    else if (exts.some((e) => name.endsWith(e))) out.push(p);
  }
  return out;
}

const norm = (p) => p.split("\\").join("/");
const under = (p, prefix) => p === norm(prefix) || p.startsWith(norm(prefix) + "/");

// ts-rs output — never hand-written, generated FROM the Rust source this index already covers
// under its own name. Excluding it avoids indexing the same declaration twice under two paths
// with no benefit: a citation of a generated-file symbol is exactly as well served by the
// Rust-side entry, and the generated tree is regenerated wholesale rather than edited.
export const GENERATED_ROOT = "src/types/generated";

// ---------------------------------------------------------------------------------------------
// Rust symbol index: fn / struct / enum / trait items, plus impl-block methods (both bare and
// `Type::method` qualified, so a citation in either style resolves).
// ---------------------------------------------------------------------------------------------

const RUST_ITEM = /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:default\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]*"\s+)?(fn|struct|enum|trait|const|static)\s+([A-Za-z_][A-Za-z0-9_]*)/;
const RUST_IMPL = /^\s*(?:unsafe\s+)?impl(?:<[^{]*?>)?\s+(?:[A-Za-z_][A-Za-z0-9_:]*(?:<[^{]*?>)?\s+for\s+)?([A-Za-z_][A-Za-z0-9_]*)/;
const RUST_TRAIT = /^\s*(?:pub(?:\([^)]*\))?\s+)?trait\s+([A-Za-z_][A-Za-z0-9_]*)/;
const RUST_MOD = /^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*[;{]/;
const RUST_STRUCT_FIELD = /^\s*(?:pub(?:\([^)]*\))?\s+)?([a-zA-Z_][a-zA-Z0-9_]*)\s*:(?!:)/;
const RUST_ENUM_VARIANT = /^\s*([A-Za-z_][A-Za-z0-9_]*)\s*(?:[,(){]|$)/;

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
 * `fn`/`struct`/`enum`/`trait`/`const`/`static` item names (bare, and — when `moduleStem` is
 * known — as `stem::NAME`, since every item in `pathfinding.rs` is reachable from outside as
 * `pathfinding::NAME`); every `fn` SIGNATURE found inside an `impl <Type>` (`impl Trait for
 * <Type>`) block OR a `trait <Name> { … }` body — a trait method has no body of its own but is
 * exactly as real a citable symbol as an implementation's — indexed both bare and as
 * `Owner::method`; every `mod NAME` declaration, indexed bare and as `stem::NAME`; every struct
 * FIELD and enum VARIANT declared in a brace-bodied item, indexed both bare and as `Type.field` /
 * `Type::Variant` — these are real symbols the tree declares exactly as much as a function name
 * is, and a skill legitimately cites `RegionField.terrain_multiplier`-style field access and
 * `MoveReject::SceneUnknown`-style variant names.
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
  const lines = text.split("\n");
  // Method-owner container (`impl <Type>` or `trait <Name>`): qualifies a `fn` signature found
  // inside its body as `Owner::method`, whether that signature has a body (`impl`) or is a bare
  // declaration (`trait`).
  let methodOwner = null; // { name, depth }
  // Struct/enum field-and-variant container: at most one active at a time, since a struct or
  // enum body cannot itself open a SECOND struct/enum body at the same nesting level the
  // container tracks — a nested brace (a struct-variant's own fields) simply sits one level
  // deeper than `depth === container.depth + 1` and is correctly excluded without a stack.
  let container = null; // { kind: "struct" | "enum", name, depth }
  let depth = 0;
  for (const line of lines) {
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
        container = { kind, name, depth };
      }
    } else if (container === null) {
      const modMatch = RUST_MOD.exec(line);
      if (modMatch) {
        addQualified(modMatch[1]);
        if (line.trimEnd().endsWith("{")) modStack.push({ name: modMatch[1], depth });
      }
    }
    if (container !== null && depth === container.depth + 1) {
      const trimmed = line.trim();
      if (!trimmed.startsWith("#") && !trimmed.startsWith("//")) {
        const fieldOrVariant =
          container.kind === "struct"
            ? RUST_STRUCT_FIELD.exec(line)
            : RUST_ENUM_VARIANT.exec(line);
        if (fieldOrVariant) {
          const member = fieldOrVariant[1];
          // Both separators, regardless of container kind: Markdown prose is not held to strict
          // Rust syntax, and a skill citing a struct field as `RollSpec::direction` (real syntax:
          // `.direction`) means the same thing to a reader as `RollSpec.direction` — the
          // conceptual owner-qualification, not the literal access-operator choice. Routed through
          // `addQualified` so a module-path-prefixed form (`recalc::RecalcOp::ReplaceDie`)
          // resolves too, exactly as a plain item's qualification does.
          addQualified(member);
          addQualified(`${container.name}.${member}`);
          addQualified(`${container.name}::${member}`);
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
// TS/JS symbol index: exported function/class/interface/type/const/enum declarations, plus
// named re-exports (`export { a, b as c }`), plus a `.svelte` file's own component name (its
// default export, named by the file's basename — Svelte declares no `export class`/`function`
// for the component itself).
// ---------------------------------------------------------------------------------------------

const TS_EXPORT_DECL =
  /\bexport\s+(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(?:function\*?|class|interface|type|const|let|enum)\s+([A-Za-z_$][A-Za-z0-9_$]*)/g;
const TS_EXPORT_LIST = /\bexport\s*\{([^}]*)\}/g;
// A MODULE-LEVEL declaration with no `export` keyword — anchored at the start of the line (no
// leading indentation) so a same-shaped LOCAL declaration inside a function/method body is never
// matched. Mirrors Rust's inclusive indexing (which does not require `pub`): a skill citing a
// private module-level `const` is citing a real, if non-exported, symbol.
const TS_MODULE_LEVEL_DECL =
  /^(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(?:function\*?|class|interface|type|const|enum)\s+([A-Za-z_$][A-Za-z0-9_$]*)/gm;

/**
 * Extracts every module-level TS/JS symbol name from one file's text: declaration-form exports
 * (`export function foo`, `export class Bar`, `export const BAZ = …`), named-export-list exports
 * (`export { a, b as c }`, indexing the EXPORTED name `c`, not the local alias `b`), and a
 * non-exported module-level declaration (`const CHAT_ERROR_WINDOW_MS = …`, no `export`) — a
 * skill may legitimately cite an internal, module-private symbol, not only the module's public
 * surface.
 *
 * @param {string} text - one `.ts`/`.mjs`/`.js` file's contents (or a `.svelte` file's
 *   extracted `<script>` text).
 * @returns {Set<string>} every module-level symbol name declared in this file.
 */
export function extractTsExports(text) {
  const names = new Set();
  for (const m of text.matchAll(TS_EXPORT_DECL)) names.add(m[1]);
  for (const m of text.matchAll(TS_MODULE_LEVEL_DECL)) names.add(m[1]);
  for (const m of text.matchAll(TS_EXPORT_LIST)) {
    for (const item of m[1].split(",")) {
      const trimmed = item.trim();
      if (!trimmed) continue;
      const asMatch = /\bas\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*$/.exec(trimmed);
      const exported = asMatch ? asMatch[1] : trimmed.split(/\s+/)[0];
      if (/^[A-Za-z_$][A-Za-z0-9_$]*$/.test(exported)) names.add(exported);
    }
  }
  return names;
}

const TS_CONTAINER_OPEN =
  /\b(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(interface|class)\s+([A-Za-z_$][A-Za-z0-9_$]*)|\b(?:export\s+)?type\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*\{/;
const TS_MEMBER =
  /^\s*(?:(?:readonly|public|private|protected|static)\s+)*(?:async\s+)?(?:get\s+|set\s+)?([A-Za-z_$][A-Za-z0-9_$]*)\??\s*(?::|\(|=(?!=))/;

/**
 * Extracts every member (property or method) name declared inside a TS `interface`, `class`, or
 * object-shaped `type` alias body — the TS-side counterpart of `extractRustSymbols`'s struct
 * field / enum variant extraction, since a skill citing `AppContext.documents` or
 * `EngineAdapter.dispose` is naming a real declared member exactly as much as a citation of an
 * exported top-level symbol.
 *
 * Brace-depth tracking is line-based; see `extractRustSymbols`'s doc for why a rare
 * string/comment miscount only ever widens, never narrows, what gets indexed.
 *
 * @param {string} text - one `.ts`/`.svelte`-script file's contents.
 * @returns {Set<string>} every bare and `Owner.member` qualified name found.
 */
export function extractTsMembers(text) {
  const names = new Set();
  const lines = text.split("\n");
  let container = null; // { name, depth }
  let depth = 0;
  for (const line of lines) {
    if (container === null) {
      const open = TS_CONTAINER_OPEN.exec(line);
      if (open && line.includes("{")) container = { name: open[2] ?? open[3], depth };
    }
    if (container !== null && depth === container.depth + 1) {
      const trimmed = line.trim();
      if (!trimmed.startsWith("//") && !trimmed.startsWith("*")) {
        const member = TS_MEMBER.exec(line);
        if (member) {
          names.add(member[1]);
          names.add(`${container.name}.${member[1]}`);
        }
      }
    }
    for (const ch of line) {
      if (ch === "{") depth += 1;
      else if (ch === "}") {
        depth -= 1;
        if (container !== null && depth <= container.depth) container = null;
      }
    }
  }
  return names;
}

// A wire-protocol discriminated union's member is a STRING LITERAL, not a declared identifier
// (`type SyncState = "none" | "up_to_date" | "template_changed"`), and a skill legitimately cites
// the wire VALUE a document/message actually carries. Matches only an EXPORTED `type` alias whose
// body is entirely string-literal alternatives, so an ordinary string constant used as one
// argument among several is not mistaken for a discriminant declaration.
const TS_STRING_LITERAL_UNION =
  /\bexport\s+type\s+[A-Za-z_$][A-Za-z0-9_$]*\s*=\s*((?:"[^"]*"|'[^']*')(?:\s*\|\s*(?:"[^"]*"|'[^']*'))*)\s*;?/g;

/**
 * Extracts every member of an exported string-literal union type — the wire-protocol
 * discriminant-VALUE counterpart of `extractTsExports`'s declaration-name extraction.
 * @param {string} text - one `.ts` file's contents.
 * @returns {Set<string>} every string literal appearing as a union member.
 */
export function extractTsStringLiteralUnionMembers(text) {
  const names = new Set();
  for (const m of text.matchAll(TS_STRING_LITERAL_UNION)) {
    for (const lit of m[1].matchAll(/"([^"]*)"|'([^']*)'/g)) names.add(lit[1] ?? lit[2]);
  }
  return names;
}

/**
 * Extracts a `.svelte` file's `<script>` and `<script module>` block contents (each may appear
 * once), for feeding to `extractTsExports`. Returns "" when neither block is present.
 * @param {string} text - one `.svelte` file's raw contents.
 * @returns {string} the concatenated script block contents.
 */
export function extractSvelteScript(text) {
  const out = [];
  for (const m of text.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g)) out.push(dedent(m[1]));
  return out.join("\n");
}

/**
 * Strips the common leading-whitespace indent every non-blank line shares, so a `.svelte` file's
 * `<script>` body (conventionally indented to match its tag) lands at column 0 exactly like an
 * ordinary top-level `.ts` module — the anchor `extractTsExports`'s module-level regex requires.
 * @param {string} text - block of source text.
 * @returns {string} the same text with the shared indent removed from every line.
 */
export function dedent(text) {
  const lines = text.split("\n");
  const indents = lines
    .filter((l) => l.trim() !== "")
    .map((l) => l.match(/^[ \t]*/)[0].length);
  const common = indents.length > 0 ? Math.min(...indents) : 0;
  return lines.map((l) => l.slice(common)).join("\n");
}

/**
 * Builds the full code-symbol index this checker resolves citations against: every Rust
 * `fn`/`struct`/`enum`/`trait` item and impl method (bare + `Type::method`) under
 * `src/server/src`, every TS/JS export under `src/client`, `src/modules`, `src/types` (excluding
 * `GENERATED_ROOT`) and `scripts`, and every `.svelte` file's component name (its basename) plus
 * its script-block exports.
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
    for (const name of extractRustSymbols(text, rustModulePath(rustRoot, file))) symbols.add(name);
  }

  const tsRoots = ["src/client", "src/modules", "src/types", "scripts"];
  for (const root of tsRoots) {
    const abs = join(repoRoot, ...root.split("/"));
    for (const file of sources(abs, [".ts", ".mjs", ".js"])) {
      const rel = norm(join(root, file.slice(abs.length + 1)));
      if (under(rel, GENERATED_ROOT)) continue;
      filesIndexed += 1;
      const text = readFileSync(file, "utf8");
      for (const name of extractTsExports(text)) symbols.add(name);
      for (const name of extractTsMembers(text)) symbols.add(name);
      for (const name of extractTsStringLiteralUnionMembers(text)) symbols.add(name);
    }
    for (const file of sources(abs, [".svelte"])) {
      filesIndexed += 1;
      const componentName = basename(file, ".svelte");
      symbols.add(componentName);
      const script = extractSvelteScript(readFileSync(file, "utf8"));
      const scriptNames = new Set([
        ...extractTsExports(script),
        ...extractTsMembers(script),
      ]);
      for (const name of scriptNames) {
        symbols.add(name);
        // A skill legitimately cites a script-level function as the COMPONENT's own behaviour
        // (`SystemTreeEditor.removeField`) — the component name is this file's de facto exported
        // owner, exactly as a TS class or interface name owns its members.
        symbols.add(`${componentName}.${name}`);
      }
      for (const name of extractTsStringLiteralUnionMembers(script)) symbols.add(name);
    }
  }

  for (const name of deriveSnakeCaseAliases(symbols)) symbols.add(name);
  return { symbols, filesIndexed };
}

// Rust's `#[serde(rename_all = "snake_case")]` (and this repo's matching client-side Zod
// literals) mechanically derive a wire-protocol VALUE from a declared PascalCase type/variant
// name — `GmOnly` → `"gm_only"` — with no independent declaration of the derived form anywhere
// to index directly. The derivation is a pure, deterministic function of a name already proven to
// exist in the tree, so adding it is completing the recognizer for a real, mechanical rule this
// codebase actually follows — not guessing a shape from specimens.
function pascalToSnakeCase(name) {
  return name.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
}

/**
 * Derives the `snake_case` wire-value form of every bare PascalCase name already in `symbols`.
 * @param {Set<string>} symbols - the index built so far.
 * @returns {string[]} every derived snake_case alias.
 */
export function deriveSnakeCaseAliases(symbols) {
  const derived = [];
  for (const name of symbols) {
    if (!/^[A-Z][A-Za-z0-9]*$/.test(name)) continue;
    const snake = pascalToSnakeCase(name);
    // Only a genuine multi-word compound (`GmOnly` -> `gm_only`) is derived — a single-word
    // PascalCase name (`Value` -> `value`) would flood the index with every ordinary lowercase
    // noun that happens to share a type's name, which the ambiguous-token bucket already governs
    // more precisely than a blanket alias could.
    if (snake.includes("_")) derived.push(snake);
  }
  return derived;
}

// ---------------------------------------------------------------------------------------------
// Citation extraction and classification, over `.claude/skills/**/*.md`.
// ---------------------------------------------------------------------------------------------

export const SKILLS_MD_EXT = ".md";

// Scoped to the `shadowcat-codebase-*` family, not the whole `.claude/skills` tree. RULE 15's
// citation requirement is about THIS repo's own code, whose churn is what makes a path/line
// citation rot; a vendored third-party skill (`graphify`) documents an external tool's own stable
// API (Python function/variable names, its own config keys), and checking those backtick tokens
// against Shadowcat's Rust/TS symbol index is a category error, not a coverage gap — every one of
// them is systematically unresolvable regardless of whether the vendored doc is accurate. This is
// the same distinction `check-comment-refs.mjs`'s own ACKNOWLEDGED list already draws by name
// ("a vendored tool skill's own procedural step heading", "a vendored skill's own internal
// structure"): vendored skill prose is a recognized, separate citation domain.
const SHADOWCAT_SKILL_PREFIX = "shadowcat-codebase-";

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

/** Finds every `SKILL.md` under a `shadowcat-codebase-*` directory inside `.claude/skills`,
 * excluding `NIGHTFOX_SKILL_DIR` (see its comment) unless `includeNightfox` is set. */
export function findMarkdownFiles(skillsRoot, { includeNightfox = false } = {}) {
  const out = [];
  for (const entry of readdirSync(skillsRoot, { withFileTypes: true })) {
    if (!entry.isDirectory() || !entry.name.startsWith(SHADOWCAT_SKILL_PREFIX)) continue;
    if (!includeNightfox && entry.name === NIGHTFOX_SKILL_DIR) continue;
    out.push(...sources(join(skillsRoot, entry.name), [SKILLS_MD_EXT]));
  }
  return out.sort();
}

// A citation candidate is the ENTIRE content of an inline backtick span, once trimmed, matching
// this shape: an identifier, optionally chained by `::` or `.` segments (`Owner::member`,
// `Owner.member`), optionally followed by `()` (a call-site citation, `find()`). Anchored full-
// match, not a substring search — a span containing anything else (whitespace, `<>`, `&`, `:`
// single-colon type annotations, `/` paths) is a TYPE SNIPPET, CONFIG VALUE, PATH or PROSE
// PHRASE, not a symbol citation, and is excluded from the candidate set rather than guessed at.
const CITATION_SHAPE =
  /^[A-Za-z_][A-Za-z0-9_]*(?:(?:::|\.)[A-Za-z_][A-Za-z0-9_]*)*(?:\(\))?$/;

// A token ending in a recognized non-code file extension (`CLAUDE.md`, `_semantic.scss`,
// `typedoc.json`) is a FILENAME cited as a value or a durable-document pointer — RULE 15/16
// already govern that citation shape on their own terms — not a code-symbol citation, and is
// excluded from the candidate set entirely rather than checked against the symbol index.
const FILE_EXTENSION_TOKEN = /\.(?:md|scss|json|toml|ya?ml|py|txt|html?|svg|png|db)$/i;

// The same EXAMPLE-exempt convention `check-comment-refs.mjs` uses: a line demonstrating a
// specimen rather than making a real claim is exempt from THIS checker too, for the identical
// reason — the specimen is deliberately not-real, so resolving it against the tree is
// meaningless.
const EXAMPLE_EXEMPT_LINE = /\bEXAMPLE:/;

// Inline backtick spans only — excludes fenced ``` code blocks, which are illustrative snippets
// (BAD/GOOD examples, shell commands) rather than point citations, and excludes a span crossing
// a fence boundary by never matching across a line containing an odd number of backticks inside
// a fence (fences are stripped from the text before this regex runs — see `stripFences`).
const BACKTICK_SPAN = /`([^`\n]+)`/g;

/** Strips fenced ``` code blocks (any info string) from Markdown text, replacing each with blank
 * lines so line numbers in the caller's reporting stay aligned with the original file. */
export function stripFences(text) {
  return text.replace(/```[\s\S]*?```/g, (m) => m.replace(/[^\n]/g, ""));
}

// RULE 15's own GOOD examples are always either owner-qualified by a CAPITALIZED type
// (`AssetResolver.revs`, `egress_loop`'s `SceneSubscribe` arm) or a `::`-qualified Rust path, or a
// distinctively compound standalone symbol (`snake_case` with an underscore, `SCREAMING_SNAKE`, or
// `PascalCase`). Three shapes never appear as a GOOD example and are genuinely ambiguous between
// "citing the real declaration" and "naming an ordinary in-context value" — mechanical resolution
// cannot tell those apart without the scope/type awareness only a real compiler has, the same
// structural limit RULE 16 already accepted for the lowercase-hyphenated marker shape ("no
// pattern can detect it... a review obligation, not a gate obligation"):
//  - a bare, flat, all-lowercase word with no underscore and no internal capital (`item`, `find`);
//  - a bare camelCase word (`basePrefix`, `activeScene`) — this repo's LOCAL-variable convention,
//    never a citation of an EXPORTED declaration in this skill family's actual usage;
//  - a lowercase-first `.`-chain (`doc.engine`, `ctx.documents`, `grid.size`) — this skill
//    family's established WIRE/JSON-path notation for a runtime VALUE's shape, distinct from the
//    `Owner.member` declaration-citation shape, whose owner segment is always capitalized.
// A call-site citation (`boot()`) is excluded from all four regardless of case — the trailing
// `()` is unambiguous prose-vs-citation evidence on its own, and `::` is exclusively a Rust
// namespace/type separator with no value-access reading, so a `::`-chain is never ambiguous.
const FLAT_LOWERCASE_TOKEN = /^[a-z][a-z0-9]*$/;
const BARE_CAMEL_CASE_TOKEN = /^[a-z][a-z0-9]*(?:[A-Z][a-z0-9]*)+$/;
// A lowercase-first `.`-chain stays ambiguous with or without a trailing call — `doc.owner.is_some()`
// is a Rust method-chain narrated in prose exactly as much as `doc.owner` is a JSON-path narrated in
// prose; the `()` on a bare token is what disambiguates a bare token, not a chained one.
const LOWERCASE_DOT_CHAIN = /^[a-z][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)+(?:\(\))?$/;
// A bare `snake_case` word with FEW underscores is this repo's Rust LOCAL-variable convention as
// much as its declared-item convention — shape alone cannot tell `db_path` (a local) from
// `parse_command` (a real function) apart, and measured directly against this codebase's actual
// index: every short (<=2-underscore) bare snake_case token found broken in this corpus resolved
// to a real LOCAL variable, never a dead citation. A LONG compound (3+ underscores) stays a fatal
// candidate — it is the shape of the actual defect this task fixed
// (`frozen_parity_king_step_grid_outcomes`), and RULE 15's own convention (qualify a common bare
// name by its owner/module) means a genuinely bare top-level citation is already the exceptional
// case, distinctively long by construction.
const SHORT_BARE_SNAKE_CASE = /^[a-z][a-z0-9]*(?:_[a-z0-9]+){1,2}$/;

/** True when `token` matches one of the four ambiguous shapes documented above. */
function isAmbiguousToken(token) {
  if (token.includes("::")) return false;
  if (LOWERCASE_DOT_CHAIN.test(token)) return true;
  if (token.endsWith("()")) return false;
  return (
    FLAT_LOWERCASE_TOKEN.test(token) ||
    BARE_CAMEL_CASE_TOKEN.test(token) ||
    SHORT_BARE_SNAKE_CASE.test(token)
  );
}

/**
 * Extracts every citation-shaped backtick span from one skill file's prose (fenced code blocks
 * excluded), split into three buckets: `candidates` (checked against the symbol index, and the
 * gate fails on an unresolved one), `ambiguous` (a flat bare lowercase word — see
 * `FLAT_AMBIGUOUS_TOKEN` — counted and reported, never gated), and `nonCandidates` (a backtick
 * span whose shape rules it out entirely — a path, a type snippet, a prose phrase). Nothing here
 * is silently dropped: every bucket is a number the caller must print.
 *
 * @param {string} text - the skill file's raw contents.
 * @returns {{ candidates: {line: number, token: string}[], ambiguous: {line: number, token: string}[], nonCandidates: number }}
 */
export function extractCitationCandidates(text) {
  const body = stripFences(text);
  const lines = body.split("\n");
  const candidates = [];
  const ambiguous = [];
  let nonCandidates = 0;
  lines.forEach((line, i) => {
    if (EXAMPLE_EXEMPT_LINE.test(line)) return;
    for (const m of line.matchAll(BACKTICK_SPAN)) {
      const token = m[1].trim();
      if (token === "") continue;
      if (!CITATION_SHAPE.test(token) || FILE_EXTENSION_TOKEN.test(token)) {
        nonCandidates += 1;
        continue;
      }
      if (isAmbiguousToken(token)) {
        ambiguous.push({ line: i + 1, token });
        continue;
      }
      candidates.push({ line: i + 1, token });
    }
  });
  return { candidates, ambiguous, nonCandidates };
}

// A citation-shaped token that is legitimately not a code symbol: an external crate/std/Web API
// name, a wire/serde-attribute keyword, or a config value written in a symbol-like shape. Named
// and reasoned, exactly like `check-comment-refs.mjs`'s ACKNOWLEDGED list, and for the same
// reason: an unresolved candidate must be counted and either verified, acknowledged, or reported
// as broken — never silently dropped.
export const ACKNOWLEDGED_NON_SYMBOLS = new Set([
  // Rust primitive/std types this repo's own code never declares — cited as VALUES.
  "bool", "f64", "f32", "i32", "i64", "u32", "u64", "usize", "isize", "str", "String", "Vec",
  "Option", "Some", "None", "Result", "Ok", "Err", "HashMap", "HashSet", "BTreeMap", "BTreeSet",
  "Uuid", "Arc", "Mutex", "RwLock", "Box", "Cow", "Rc", "RefCell", "Sync", "Send", "Display",
  "Debug", "Clone", "PartialEq", "Eq", "Path", "PathBuf", "SqlitePool", "SyntaxError", "Sink",
  "ConnectInfo", "JoinSet", "Value", "E",
  // Rust std method/associated-fn names cited standalone (`Vec::new()`, `checked_add`) — this
  // repo declares no method of these names on its own types at the point cited.
  "new", "default", "checked_add", "checked_mul", "saturating_mul", "is_ascii_graphic",
  "max_by_key", "min_by_key", "binary_search_by_key",
  // TS/JS primitives and standard/Web globals cited without meaning "this repo declares it".
  "string", "number", "boolean", "object", "unknown", "never", "void", "null", "undefined",
  "Array", "Map", "Set", "Promise", "Error", "Record", "Window", "IntersectionObserver",
  "SyntaxError2", "structuredClone",
  // Literal/keyword values, not declarations.
  "true", "false", "NaN", "Infinity",
  // Non-Rust/TS/Svelte source this checker's symbol index does not cover (Python hook internals,
  // SQL/OS keywords, HTTP method/header names).
  "SUBSYSTEMS", "file_path", "NOCASE", "Referer", "GET", "POST", "SO_SNDBUF", "SO_RCVBUF",
  "EXEMPT", "TODO",
  // `#[serde(...)]` / rustdoc attribute keywords — serde's and rustdoc's own vocabulary, not a
  // symbol this repo declares.
  "deny_unknown_fields", "skip_serializing_if", "rename_all", "no_run",
  // Third-party UI libraries this repo depends on but does not declare (dockview panel library,
  // PixiJS rendering, TypeDoc's own internals) — their own type/class names, cited as VALUES.
  "DockviewComponent", "PopoutWindowService", "AsapEvent", "DockviewApi", "PanelApiImpl",
  "Overlay", "DockviewWillDropEvent", "AnimatedSprite", "Sprite", "Graphics", "RenderTexture",
  "BlurFilter", "Options", "SvelteSet", "SvelteMap", "Vec2",
  // `serde_json::Number`'s own PRIVATE variant names (`PosInt`/`NegInt`/`Float`), cited in a
  // comment describing serde_json's internal representation split — not a type this repo declares.
  "PosInt", "NegInt", "Float",
  // Further external crate/Web-API types this repo cites as values (`hyper`'s `Host` header
  // enum, `url`'s `Url`, PixiJS's `Link` filter, the `ammonia` HTML-sanitizer's `PassThrough`
  // element-handling mode).
  "Host", "Link", "PassThrough", "SqlSafeStr", "Date", "DefaultBodyLimit", "Url", "Mesh",
  // Durable-tracker filenames RULE 15/16 already govern on their own terms (a "Pointers"-section
  // citation of a durable doc by name), not a code-symbol citation.
  "OPEN_BUGS", "CLOSED_BUGS",
  // The `EXAMPLE:` marker word itself, cited by `shadowcat-codebase-core` while explaining the
  // marker convention — not a code symbol.
  "EXAMPLE",
  // Deliberate NEGATIVE citations: `shadowcat-codebase-actors-tokens`/`-scene-rendering` name
  // these to state there is NO such back-compat alias (verified against
  // `src/server/src/data/engine/`, which declares none of them) — the citation's own sentence is
  // the thing making the claim true, not a pointer to a live declaration.
  "ActorSystem", "TokenSystem", "FactionRegistrySystem", "ConditionRegistrySystem", "RegionSystem",
  // dockview panel library internals (vendored dependency, not this repo's own declaration) and
  // the DOM `Element.getBoundingClientRect` Web API.
  "mutation", "getNextGroupId", "getBoundingClientRect",
  // A temporary `#[cfg(test)]`-only frozen-copy oracle, deliberately added and then deleted once
  // its parity proof was captured as literal fixtures — verified real by its own surrounding
  // sentence, which states exactly this; citing its former name is not a dead pointer, it is the
  // record of what the frozen fixture test now stands in for.
  "execute_move_kingstep_oracle",
  // A local closure narrated in prose with a module-style qualifier for readability
  // (`los_smooth::chord_ok`) — `chord_ok` is a real `let chord_ok = |a, b| …` inside
  // `los_smooth`, not a declared item this checker's index can qualify.
  "chord_ok",
  // Real, verified callback-handler members conceptually owned by the client they configure, not
  // literally declared on the class itself (`onReject` is a field of `WsClient`'s handlers
  // options type; `send`/`toggle` are `AppContext.chat`/`AppContext.panels` sub-object methods).
  "onReject", "send", "toggle", "message", "recipients", "scene_id", "schema_declarations",
  "server_version", "regionShapeMode", "scale", "read",
  // The wire-boundary error TYPE NAME (`PathError`) is a documentation label for what
  // `PathFail` (the Rust enum) maps to at serialization — verified: no `PathError` type is
  // separately declared in either `src/server/src` or `src/client/core/src/wire.ts`.
  "PathError",
  // A citation of the FILE/MODULE as a whole ("the `bin::test_server` module carries..."), not of
  // one declared item inside it — this checker resolves item citations, not module-as-a-whole
  // mentions, which is a structurally different (and much rarer) citation shape.
  "test_server", "TabbedSurface", "ActorVisual", "running_",
]);

// Regex forms, for a qualified path whose PREFIX names a known external crate/package rather
// than this repo's own code — checking a full qualified token against a flat Set would need one
// entry per method the crate exposes, which is unbounded; the crate/package NAME is the bounded,
// reasoned thing to acknowledge.
const ACKNOWLEDGED_EXTERNAL_PREFIX = new Set([
  "tokio", "clap", "axum", "reqwest", "serde_json", "ammonia", "polyanya", "geo", "i_overlay",
  "hecs", "spade", "axum_test", "tower_sessions", "std", "core", "alloc", "typedoc",
]);

/**
 * Resolves every citation candidate in one skill file's text against `symbols`, classifying each
 * as verified, acknowledged (a named non-symbol token), or broken (an unresolved candidate that
 * is neither). The `ambiguous` bucket from `extractCitationCandidates` passes through uncounted
 * against `symbols` — it is a review obligation, never gated — but its count is always returned.
 *
 * @param {string} text - the skill file's raw contents.
 * @param {Set<string>} symbols - the built symbol index.
 * @returns {{ verified: number, acknowledged: number, broken: {line: number, token: string}[], ambiguous: number, nonCandidates: number }}
 */
export function checkFileCitations(text, symbols) {
  const { candidates, ambiguous, nonCandidates } = extractCitationCandidates(text);
  let verified = 0;
  let acknowledged = 0;
  const broken = [];
  for (const { line, token } of candidates) {
    const bare = token.endsWith("()") ? token.slice(0, -2) : token;
    if (symbols.has(bare)) {
      verified += 1;
      continue;
    }
    const segments = bare.split(/::|\./);
    const lastSegment = segments[segments.length - 1];
    const isAcknowledged =
      ACKNOWLEDGED_NON_SYMBOLS.has(bare) ||
      (segments.length > 1 &&
        (ACKNOWLEDGED_EXTERNAL_PREFIX.has(segments[0]) ||
          ACKNOWLEDGED_NON_SYMBOLS.has(segments[0]) ||
          ACKNOWLEDGED_NON_SYMBOLS.has(lastSegment)));
    if (isAcknowledged) {
      acknowledged += 1;
      continue;
    }
    broken.push({ line, token });
  }
  return { verified, acknowledged, broken, ambiguous: ambiguous.length, nonCandidates };
}

/**
 * Runs the full gate: builds the symbol index, then checks every `.md` skill file under
 * `skillsRoot` against it.
 *
 * @param {string} skillsRoot - absolute path to `.claude/skills`.
 * @param {string} repoRoot - absolute path to the repository root.
 * @returns {{ filesScanned: number, filesIndexed: number, symbolCount: number,
 *   candidatesChecked: number, verified: number, acknowledged: number,
 *   broken: {file: string, line: number, token: string}[], ambiguous: number,
 *   nonCandidates: number, nightfoxExcludedFiles: number, nightfoxExcludedBroken: number }}
 */
export function checkSkillSymbolRefs(skillsRoot, repoRoot) {
  const { symbols, filesIndexed } = buildSymbolIndex(repoRoot);
  const files = findMarkdownFiles(skillsRoot);
  let verified = 0;
  let acknowledged = 0;
  let ambiguous = 0;
  let nonCandidates = 0;
  const broken = [];
  for (const file of files) {
    const result = checkFileCitations(readFileSync(file, "utf8"), symbols);
    verified += result.verified;
    acknowledged += result.acknowledged;
    ambiguous += result.ambiguous;
    nonCandidates += result.nonCandidates;
    for (const b of result.broken) broken.push({ file, ...b });
  }

  // The excluded Nightfox skill still gets counted — never silently dropped — as its own
  // structurally-unresolvable review obligation, per `NIGHTFOX_SKILL_DIR`'s comment.
  const nightfoxFiles = findMarkdownFiles(skillsRoot, { includeNightfox: true }).filter(
    (f) => !files.includes(f),
  );
  let nightfoxExcludedBroken = 0;
  for (const file of nightfoxFiles)
    nightfoxExcludedBroken += checkFileCitations(readFileSync(file, "utf8"), symbols).broken.length;

  return {
    filesScanned: files.length,
    filesIndexed,
    symbolCount: symbols.size,
    candidatesChecked: verified + acknowledged + broken.length,
    verified,
    acknowledged,
    broken,
    ambiguous,
    nonCandidates,
    nightfoxExcludedFiles: nightfoxFiles.length,
    nightfoxExcludedBroken,
  };
}
