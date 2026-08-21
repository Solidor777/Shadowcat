import { describe, it, expect } from "vitest";
import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve, basename } from "node:path";
import { fileURLToPath } from "node:url";
import {
  extractRustSymbols,
  extractRustReexports,
  extractSqlSymbols,
  extractTomlKeys,
  extractJsonKeys,
  extractJsonDependencyNames,
  moduleNameOf,
  applyRenameAll,
  splitSourceLines,
  rustModulePath,
  extractTsSymbols,
  extractSvelteScript,
  importBindingNames,
  extractCodeSpans,
  stripCodeBlocks,
  citationTokens,
  countBacktickRuns,
  spanAccountingDelta,
  extractCitationCandidates,
  resolvesAgainstIndex,
  checkFileCitations,
  checkSkillSymbolRefs,
  buildSymbolIndex,
} from "./check-skill-symbol-refs.mjs";
import { listSkillDirs } from "./lib/gate-corpus.mjs";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// The fixture directory name is DERIVED from this file's own basename rather than written out, so
// the one-fixed-path-per-suite rule is structural: two suites cannot name the same directory
// without sharing a filename, which the filesystem already forbids.
const FIXTURE_ROOT = join(
  tmpdir(),
  `shadowcat-${basename(fileURLToPath(import.meta.url), ".test.mjs")}-fixture`,
);


describe("extractRustSymbols", () => {
  it("indexes fn/struct/enum/trait/const/static items bare and module-path-qualified", () => {
    const text = [
      "pub const MAX_FOO: u32 = 4;",
      "pub struct Widget {",
      "    pub id: u32,",
      "}",
      "pub enum Shape {",
      "    Circle,",
      "    Square,",
      "}",
      "pub fn build() -> Widget { todo!() }",
    ].join("\n");
    const names = extractRustSymbols(text, ["scene", "widget"]);
    expect(names.has("MAX_FOO")).toBe(true);
    expect(names.has("scene::widget::MAX_FOO")).toBe(true);
    expect(names.has("widget::MAX_FOO")).toBe(true);
    expect(names.has("Widget")).toBe(true);
    expect(names.has("Widget.id")).toBe(true);
    expect(names.has("Shape::Circle")).toBe(true);
    expect(names.has("build")).toBe(true);
  });

  it("indexes a `type` alias item", () => {
    const names = extractRustSymbols("pub type SceneFootprints = Vec<(f64, f64)>;\n", ["scene"]);
    expect(names.has("SceneFootprints")).toBe(true);
    expect(names.has("scene::SceneFootprints")).toBe(true);
  });

  it("qualifies an impl method as Type::method, and a trait method the same way", () => {
    const text = [
      "pub trait GridShape {",
      "    fn line_traversal(&self) -> bool;",
      "}",
      "impl GridShape for SquareGrid {",
      "    fn line_traversal(&self) -> bool { true }",
      "}",
    ].join("\n");
    const names = extractRustSymbols(text);
    expect(names.has("GridShape::line_traversal")).toBe(true);
    expect(names.has("SquareGrid::line_traversal")).toBe(true);
  });

  it("qualifies an inline mod's items under its own name (cap::WRITE_FIELDS)", () => {
    const text = ["pub mod cap {", "    pub const WRITE_FIELDS: &str = \"x\";", "}"].join("\n");
    const names = extractRustSymbols(text, ["data", "permission"]);
    expect(names.has("cap::WRITE_FIELDS")).toBe(true);
    expect(names.has("permission::cap::WRITE_FIELDS")).toBe(true);
  });

  it("does not let a lone unbalanced brace inside a char literal corrupt later container tracking", () => {
    // A char literal containing an unmatched `{` (as in a real `contains('{')` guard) must not
    // shift the brace-depth count for the rest of the file — regression for the bug that made
    // `notation::parse` unresolvable.
    const text = [
      "mod tests {",
      "    fn has_brace(s: &str) -> bool { s.contains('{') }",
      "}",
      "pub fn build() {}",
    ].join("\n");
    const names = extractRustSymbols(text, ["dice", "notation"]);
    expect(names.has("notation::build")).toBe(true);
  });

  it("qualifies an enum variant with both . and :: separators, and with the module path", () => {
    const text = ["pub enum RecalcOp {", "    ReplaceDie { id: u32 },", "}"].join("\n");
    const names = extractRustSymbols(text, ["dice", "recalc"]);
    expect(names.has("RecalcOp::ReplaceDie")).toBe(true);
    expect(names.has("RecalcOp.ReplaceDie")).toBe(true);
    expect(names.has("recalc::RecalcOp::ReplaceDie")).toBe(true);
  });

  it("indexes EVERY field and variant written on one source line, not just the first", () => {
    const struct = extractRustSymbols("pub struct Seg {\n    pub x1: f64, pub y1: f64,\n}\n");
    expect(struct.has("Seg.x1")).toBe(true);
    expect(struct.has("Seg.y1")).toBe(true);
    const enumNames = extractRustSymbols("pub enum Dir {\n    Up, Down,\n}\n");
    expect(enumNames.has("Dir::Up")).toBe(true);
    expect(enumNames.has("Dir::Down")).toBe(true);
  });

  // The value-set pass reads CODE, and a comment is a lexical REGION rather than a line shape: a
  // trailing comment and a block comment carry prose past a leading-marker test, and an
  // alternation harvested out of that prose then resolves a citation against a description of the
  // code instead of a declaration - the silent direction, since the gate reports it verified.
  it("harvests no value set out of a trailing or block comment", () => {
    const text = [
      "pub fn classify(t: &str) -> bool {",
      '    matches!(t, "real" | "member") // "trailingfake" | "trailingprose"',
      '    /* "blockfake" | "blockprose" */',
      "}",
    ].join("\n");
    const names = extractRustSymbols(text);
    expect(names.has("classify::real")).toBe(true);
    expect(names.has("classify::member")).toBe(true);
    expect(names.has("classify::trailingfake")).toBe(false);
    expect(names.has("classify::trailingprose")).toBe(false);
    expect(names.has("classify::blockfake")).toBe(false);
    expect(names.has("classify::blockprose")).toBe(false);
  });

  // The DECLARATION path reads the same code span the value-set pass does. A block comment does
  // not open its line with a line-comment marker and a trailing comment does not open its line at
  // all, so a leading-marker test hands both to every extractor: a commented-out binding is then
  // indexed as a real declaration under the enclosing `fn`, and a skill citing that name reports
  // verified against prose. Revert direction: restore the leading-marker test, or hand the
  // extractors the raw line, and the three `false` expectations below turn true.
  it("indexes no declaration written inside a block or trailing comment", () => {
    const text = [
      "pub fn outer() {",
      "    /* let block_binding = 1; */",
      "    let real_binding = 2; // let trailing_binding = 3;",
      "}",
      "/*",
      "pub fn commented_out_item() {}",
      "*/",
    ].join("\n");
    const names = extractRustSymbols(text);
    expect(names.has("outer")).toBe(true);
    expect(names.has("outer::real_binding")).toBe(true);
    expect(names.has("outer::block_binding")).toBe(false);
    expect(names.has("outer::trailing_binding")).toBe(false);
    expect(names.has("commented_out_item")).toBe(false);
  });

  // The residual bounding the claim above, pinned to what the extractor does TODAY. `splitLine`
  // reads a lone single quote as a string opener, so a Rust line carrying an ODD number of them
  // consumes its trailing comment into the CODE span: a declaration commented out there is
  // extracted as real, and a skill citing that name reports verified against prose. A PAIRED
  // lifetime is the control that keeps this one shape rather than "quotes break the extractor".
  it("still indexes a declaration commented out after an ODD lifetime quote", () => {
    const odd = [
      "impl Footprint {",
      "    fn span<'a>(&self) -> u32 { // let swallowed_binding = 5;",
      "        0",
      "    }",
      "}",
    ].join("\n");
    expect(extractRustSymbols(odd).has("span::swallowed_binding")).toBe(true);
    const paired = odd.replace("<'a>(&self)", "<'a>(&self, r: &'a str)");
    expect(extractRustSymbols(paired).has("span::swallowed_binding")).toBe(false);
  });

  it("indexes a variant on a CRLF line with no trailing comma", () => {
    const names = extractRustSymbols("pub enum Mode {\r\n    Strict\r\n}\r\n");
    expect(names.has("Mode::Strict")).toBe(true);
  });

  it("indexes a field's serde WIRE name from the container's rename_all", () => {
    const text = [
      '#[serde(deny_unknown_fields, rename_all = "camelCase")]',
      "pub struct WallEngine {",
      "    /// Blocks token movement.",
      "    #[serde(default)]",
      "    pub blocks_move: Option<bool>,",
      "}",
    ].join("\n");
    const names = extractRustSymbols(text);
    expect(names.has("blocksMove")).toBe(true);
    expect(names.has("WallEngine.blocksMove")).toBe(true);
    expect(names.has("blocks_move")).toBe(true);
  });

  it("indexes a STRUCT-VARIANT's own fields, and their explicit serde wire names", () => {
    const text = [
      "pub enum TokenVisual {",
      "    Faces {",
      "        faces: BTreeMap<String, RenderVisual>,",
      '        #[serde(default, rename = "faceMap")]',
      "        face_map: Option<BTreeMap<String, String>>,",
      "    },",
      "}",
    ].join("\n");
    const names = extractRustSymbols(text);
    expect(names.has("TokenVisual::Faces")).toBe(true);
    expect(names.has("faceMap")).toBe(true);
    expect(names.has("TokenVisual.face_map")).toBe(true);
  });

  it("indexes a field's explicit serde rename over the container default", () => {
    const text = [
      '#[serde(rename_all = "camelCase")]',
      "pub struct Doc {",
      '    #[serde(rename = "type")]',
      "    pub doc_kind: String,",
      "}",
    ].join("\n");
    expect(extractRustSymbols(text).has("type")).toBe(true);
  });

  it("indexes let bindings and function parameters, including a wrapped signature", () => {
    const text = [
      "pub fn restore_backup(",
      "    db_path: &Path,",
      "    out_dir: &Path,",
      ") -> Result<()> {",
      "    let mut t_max_i = 0.0;",
      "    let (leg_start, leg_end) = split();",
      "    Ok(())",
      "}",
    ].join("\n");
    const names = extractRustSymbols(text, ["backup"]);
    expect(names.has("restore_backup::db_path")).toBe(true);
    expect(names.has("backup::restore_backup::out_dir")).toBe(true);
    expect(names.has("restore_backup::t_max_i")).toBe(true);
    expect(names.has("restore_backup::leg_start")).toBe(true);
    expect(names.has("restore_backup::leg_end")).toBe(true);
  });

  it("never indexes a function-local name BARE, which would own-nothing and absorb anything", () => {
    const text = "pub fn execute_move(scene_id: Uuid) -> bool {\n    let chord_ok = true;\n}";
    const names = extractRustSymbols(text, ["move_exec"]);
    expect(names.has("chord_ok")).toBe(false);
    expect(names.has("scene_id")).toBe(false);
    expect(names.has("execute_move::chord_ok")).toBe(true);
  });

  it("indexes a for-loop pattern's bindings, which declare names exactly as a let does", () => {
    const text = "pub fn astar_leg() {\n    for (next, sc, parity) in edges {}\n    for cell in cells {}\n}";
    const names = extractRustSymbols(text);
    expect(names.has("astar_leg::sc")).toBe(true);
    expect(names.has("astar_leg::cell")).toBe(true);
  });

  // A body-less trait method owns only the parameters on its own line: the next declaration's
  // entry goes on top of it, and only the top is ever read.
  it("gives each body-less trait method its own parameters and no sibling's", () => {
    const text = [
      "pub trait Repo {",
      "    fn first(&self, only_here: u32);",
      "    fn second(&self, elsewhere: u32);",
      "}",
    ].join("\n");
    const names = extractRustSymbols(text);
    expect(names.has("first::only_here")).toBe(true);
    expect(names.has("second::only_here")).toBe(false);
    expect(names.has("second::elsewhere")).toBe(true);
  });

  // The enclosing item's closing brace drains every buried entry at once, so a local AFTER a trait
  // is owned by whatever declares it and not by the trait's last method.
  it("drains every buried body-less entry at the enclosing item's closing brace", () => {
    const text = [
      "pub trait Repo {",
      "    fn first(&self, a: u32);",
      "    fn second(&self, b: u32);",
      "}",
      "pub fn after() {",
      "    let owned_by_after = 1;",
      "}",
    ].join("\n");
    const names = extractRustSymbols(text);
    expect(names.has("after::owned_by_after")).toBe(true);
    expect(names.has("second::owned_by_after")).toBe(false);
  });

  it("returns to the ENCLOSING fn when a nested fn's body closes (the brace-time pop)", () => {
    const text = [
      "pub fn outer() {",
      "    fn inner() {",
      "        let deep = 1;",
      "    }",
      "    let shallow = 2;",
      "}",
    ].join("\n");
    const names = extractRustSymbols(text);
    expect(names.has("inner::deep")).toBe(true);
    expect(names.has("outer::shallow")).toBe(true);
    expect(names.has("inner::shallow")).toBe(false);
  });
});

describe("applyRenameAll", () => {
  it("renders each serde style from the declared identifier", () => {
    expect(applyRenameAll("blocks_move", "camelCase")).toBe("blocksMove");
    expect(applyRenameAll("GmOnly", "snake_case")).toBe("gm_only");
    expect(applyRenameAll("world_settings", "kebab-case")).toBe("world-settings");
    expect(applyRenameAll("x", "unknown-style")).toBeNull();
  });
});

describe("splitSourceLines", () => {
  it("strips a carriage return so a line-end anchored read is not silently short", () => {
    expect(splitSourceLines("a\r\nb\n")).toEqual(["a", "b", ""]);
  });
});

describe("extractRustReexports", () => {
  it("extracts a braced pub use list, and a bare pub use path", () => {
    const text = [
      "pub use commands::{parse_command, ParsedCommand};",
      "pub use sanitize::sanitize;",
    ].join("\n");
    expect(extractRustReexports(text).sort()).toEqual(
      ["ParsedCommand", "parse_command", "sanitize"].sort(),
    );
  });

  it("uses the alias target, not the local name, for `as`", () => {
    const text = "pub use foo::{bar as baz};";
    expect(extractRustReexports(text)).toEqual(["baz"]);
  });
});

describe("Rust value sets", () => {
  // A closed value set is indexed under the function that declares it, never bare: bare, a
  // one- or two-letter member answers "the tree declares that" to any citation spelling it.
  it("indexes an alternation member and a match arm under the declaring fn, not bare", () => {
    const text = [
      "fn classify(t: &str) -> bool {",
      '    let known = matches!(t, "token" | "scene");',
      "    match t {",
      '        "kh" => true,',
      "        _ => known,",
      "    }",
      "}",
    ].join("\n");
    const names = extractRustSymbols(text);
    expect(names.has("classify::token")).toBe(true);
    expect(names.has("classify::scene")).toBe(true);
    expect(names.has("classify::kh")).toBe(true);
    expect(names.has("token")).toBe(false);
    expect(names.has("kh")).toBe(false);
  });

  it("ignores a lone string literal, which declares no value set", () => {
    const names = extractRustSymbols('fn f() {\n    let s = "drawing";\n}');
    expect(names.has("f::drawing")).toBe(false);
  });

  // A doc comment's prose is not a declaration. Harvesting an alternation out of one resolves a
  // citation against the sentence describing the code rather than against the code.
  it("does not harvest a value set out of a doc comment", () => {
    const names = extractRustSymbols('fn f() {\n    /// accepts "hex" | "square" here\n    let x = 1;\n}');
    expect(names.has("f::hex")).toBe(false);
    expect(names.has("f::square")).toBe(false);
  });
});

describe("rustModulePath", () => {
  it("returns [] for lib.rs, [\"main\"] for main.rs (doc-convention exception)", () => {
    expect(rustModulePath("/repo/src", "/repo/src/lib.rs")).toEqual([]);
    expect(rustModulePath("/repo/src", "/repo/src/main.rs")).toEqual(["main"]);
  });

  it("contributes no segment of its own for a mod.rs, but does for a plain file", () => {
    expect(rustModulePath("/repo/src", "/repo/src/chat/mod.rs")).toEqual(["chat"]);
    expect(rustModulePath("/repo/src", "/repo/src/data/sqlite.rs")).toEqual(["data", "sqlite"]);
  });
});

describe("extractSqlSymbols", () => {
  it("indexes table and column names from a CREATE TABLE body", () => {
    const text = [
      "CREATE TABLE IF NOT EXISTS explored_fog (",
      "  world_id TEXT NOT NULL,",
      "  cells BLOB NOT NULL,",
      "  PRIMARY KEY (world_id)",
      ");",
    ].join("\n");
    const names = extractSqlSymbols(text);
    expect(names.has("explored_fog")).toBe(true);
    expect(names.has("world_id")).toBe(true);
    expect(names.has("explored_fog.cells")).toBe(true);
    expect(names.has("PRIMARY")).toBe(false);
  });
});

describe("extractTomlKeys", () => {
  it("indexes a dependency name in both its manifest and Rust-path spelling", () => {
    const names = extractTomlKeys('[dependencies]\naxum-test = "1"\nhecs = "0.10"\n');
    expect(names.has("dependencies")).toBe(true);
    expect(names.has("axum-test")).toBe(true);
    expect(names.has("axum_test")).toBe(true);
    expect(names.has("hecs")).toBe(true);
  });
});

describe("extractJsonKeys", () => {
  it("indexes nested keys bare and owner-qualified, never values", () => {
    const names = extractJsonKeys('{"validation": {"notDocumented": true}, "name": "shadowcat"}');
    expect(names.has("notDocumented")).toBe(true);
    expect(names.has("validation.notDocumented")).toBe(true);
    expect(names.has("shadowcat")).toBe(false);
  });
});

describe("extractJsonDependencyNames", () => {
  // A dependency is a REFERENCE in a `package.json` for the same reason it is one in a
  // `Cargo.toml`; classifying the one category two ways lets an external package name pass the
  // acknowledgement assertion on one side and fail it on the other.
  it("names the packages a manifest depends on, and nothing else it declares", () => {
    const names = extractJsonDependencyNames(
      '{"name": "shadowcat", "dependencies": {"typescript": "^5"}, "devDependencies": {"vitest": "^4"}, "scripts": {"build": "vite"}}',
    );
    expect([...names].sort()).toEqual(["typescript", "vitest"]);
  });
});

// A dependency is a REFERENCE, and `extractJsonKeys` emits every key in both a bare and an
// owner-qualified spelling. Routing only the bare form leaves the qualified one in the DECLARED
// half, so the same package name is a reference in one spelling and a declaration in the other,
// and an acknowledgement of it reports as also-declared on the strength of the spelling nobody
// looked at.
describe("a manifest dependency is a reference in BOTH its spellings", () => {
  it("keeps no qualified dependency key in the declared half", () => {
    const manifest = readFileSync(join(REPO_ROOT, "package.json"), "utf8");
    const deps = extractJsonDependencyNames(manifest);
    const qualified = [...extractJsonKeys(manifest)].filter(
      (k) => k.includes(".") && deps.has(k.slice(k.lastIndexOf(".") + 1)),
    );
    expect(qualified.length, "the manifest declares no dependency to check").toBeGreaterThan(0);
    const { symbols, declared } = buildSymbolIndex(REPO_ROOT);
    expect(qualified.filter((k) => declared.has(k))).toEqual([]);
    // Still INDEXED, so a citation of the qualified spelling resolves: only its half changed.
    expect(qualified.filter((k) => !symbols.has(k))).toEqual([]);
  });
});

describe("moduleNameOf", () => {
  it("strips every extension, and rejects a non-identifier stem", () => {
    expect(moduleNameOf("/a/b/sheetsController.svelte.ts")).toBe("sheetsController");
    expect(moduleNameOf("/a/b/wall-view.ts")).toBe("");
  });
});

describe("extractTsSymbols", () => {
  it("indexes an exported declaration, a module-level one, and a local binding", () => {
    const text = [
      "export function foo() {}",
      "const BAR = 1;",
      "function inner() {",
      "  const local = 1;",
      "}",
    ].join("\n");
    const names = extractTsSymbols(text);
    expect(names.has("foo")).toBe(true);
    expect(names.has("BAR")).toBe(true);
    expect(names.has("inner")).toBe(true);
    expect(names.has("inner.local")).toBe(true);
    expect(names.has("local")).toBe(false);
  });

  it("owns a function-local binding and a parameter by the function that declares them", () => {
    const text = "export function checkFile(hits) {\n  const absorbed = 1;\n  return absorbed + hits;\n}";
    const names = extractTsSymbols(text);
    expect(names.has("checkFile.absorbed")).toBe(true);
    expect(names.has("checkFile.hits")).toBe(true);
    expect(names.has("absorbed")).toBe(false);
  });

  it("indexes the EXPORTED alias of a named re-export, and a type-only re-export", () => {
    const names = extractTsSymbols('export { a as b };\nexport type { SheetRef } from "./sheets";');
    expect(names.has("b")).toBe(true);
    expect(names.has("SheetRef")).toBe(true);
  });

  it("indexes `export const enum` under its own name, never the word `enum`", () => {
    const names = extractTsSymbols("export const enum Mode { Fast = 1 }");
    expect(names.has("Mode")).toBe(true);
    expect(names.has("Mode.Fast")).toBe(true);
    // Regression marker, not coverage: no mutation of the current AST reader can make the parser
    // hand back the keyword as a declared name. It pins the behaviour a pattern reader got wrong.
    expect(names.has("enum")).toBe(false);
  });

  it("indexes a namespace and a module-level `let`", () => {
    const names = extractTsSymbols("export namespace Wire { export const V = 1; }\nlet cursor = 0;");
    expect(names.has("Wire")).toBe(true);
    expect(names.has("cursor")).toBe(true);
  });

  it("indexes an interface member nested in an inline object type, under its own owner chain", () => {
    const text = [
      "export interface UiState {",
      "  global: {",
      "    lastWorld: string | null;",
      "  };",
      "}",
    ].join("\n");
    const names = extractTsSymbols(text);
    expect(names.has("lastWorld")).toBe(true);
    expect(names.has("global.lastWorld")).toBe(true);
    expect(names.has("UiState.global.lastWorld")).toBe(true);
  });

  it("indexes a class member, an arrow field and a parameter", () => {
    const text = [
      "export class Baz {",
      "  private async open(panelId: string): Promise<void> {}",
      "  private readonly viewedScene = (): string | null => null;",
      "}",
    ].join("\n");
    const names = extractTsSymbols(text);
    expect(names.has("Baz.open")).toBe(true);
    expect(names.has("Baz.viewedScene")).toBe(true);
    expect(names.has("Baz.open.panelId")).toBe(true);
    expect(names.has("panelId")).toBe(false);
  });

  it("indexes an object-literal key, including a quoted dotted catalog key", () => {
    const text = 'export const messages = { "panels.popoutRestoredFloating": "x", plain: 1 };';
    const names = extractTsSymbols(text);
    expect(names.has("panels.popoutRestoredFloating")).toBe(true);
    expect(names.has("messages.plain")).toBe(true);
  });

  // Value-set members carry their declaring constant. Bare, a one- or two-letter notation keyword
  // answers "the tree declares that" to any citation spelling `d`, `t` or `e` anywhere.
  it("indexes an array-literal value set's members UNDER the constant, never bare", () => {
    const text = 'export const NOTATION_KEYWORDS: readonly string[] = ["d", "kh", "cs"];';
    const names = extractTsSymbols(text, "template.ts", { valueSets: true });
    expect(names.has("NOTATION_KEYWORDS.kh")).toBe(true);
    expect(names.has("kh")).toBe(false);
    expect(names.has("d")).toBe(false);
  });

  // A build script's string literals are its own configuration, not wire values. Indexed, they let
  // a skill citation resolve against the tooling that checks it.
  it("extracts no value set at all when valueSets is off", () => {
    const text = 'export const ROOTS = ["src", "scripts", "examples"];';
    const names = extractTsSymbols(text, "check-lint-allowances.mjs");
    expect(names.has("ROOTS")).toBe(true);
    expect(names.has("ROOTS.src")).toBe(false);
    expect(names.has("src")).toBe(false);
  });

  it("ignores an array literal that is not a module-level declaration's value set", () => {
    const names = extractTsSymbols('function f() {\n  accept(["kh", "kl"]);\n}', "f.ts", {
      valueSets: true,
    });
    expect(names.has("kh")).toBe(false);
    expect(names.has("f.kh")).toBe(false);
  });

  it("does NOT index a collection constructor's members, which would index this gate's own lists", () => {
    // `scripts/` is an indexed root and `ACKNOWLEDGED_NON_SYMBOLS` is declared as `new Set([...])`.
    // Unwrapping the constructor makes every acknowledged token resolve as a declared name, so the
    // list goes dead and the gate reports as verified the exact citations it exists to flag.
    const names = extractTsSymbols('export const ACK = new Set(["Uuid"]);', "a.ts", {
      valueSets: true,
    });
    expect(names.has("Uuid")).toBe(false);
    expect(names.has("ACK.Uuid")).toBe(false);
  });

  it("indexes the wire VALUE a module-level string constant declares, under that constant", () => {
    const names = extractTsSymbols('export const ITEM_DOC_TYPE = "item";', "docs.ts", {
      valueSets: true,
    });
    expect(names.has("ITEM_DOC_TYPE")).toBe(true);
    expect(names.has("ITEM_DOC_TYPE.item")).toBe(true);
    expect(names.has("item")).toBe(false);
  });

  it("ignores a function-local string constant, which publishes no value", () => {
    const names = extractTsSymbols('function f() {\n  const k = "item";\n}', "f.ts", {
      valueSets: true,
    });
    expect(names.has("item")).toBe(false);
    expect(names.has("f.k.item")).toBe(false);
  });

  // A local's owner chain is registered WHOLE. Its head is a name invisible outside the function,
  // so every suffix of it is a path no citation could legitimately be naming — `cfg.host` below is
  // the shape that let an owner-less resolution survive one level down.
  it("registers a function-local object's members under the FULL chain, not every suffix", () => {
    const names = extractTsSymbols("function f() {\n  const cfg = { host: 1 };\n}");
    expect(names.has("f.cfg.host")).toBe(true);
    expect(names.has("cfg.host")).toBe(false);
    expect(names.has("host")).toBe(false);
  });

  // A string-literal type declares one member of a closed value set, so it is indexed under the
  // type that declares it and only under `valueSets` — the same rule the array-literal and
  // single-string forms obey. Bare, a discriminant answers for any prose word that spells it.
  it("indexes a string-literal type member under its type alias, never bare", () => {
    const text = 'export type SyncState = "none" | "up_to_date";';
    const names = extractTsSymbols(text, "wire.ts", { valueSets: true });
    expect(names.has("SyncState.up_to_date")).toBe(true);
    expect(names.has("up_to_date")).toBe(false);
    expect(extractTsSymbols(text).has("SyncState.up_to_date")).toBe(false);
  });
});

describe("extractSvelteScript", () => {
  it("feeds an indented script body to the parser without a dedent step", () => {
    const svelte = "<div></div>\n<script>\n  function boot() {}\n</script>\n";
    expect(extractTsSymbols(extractSvelteScript(svelte)).has("boot")).toBe(true);
  });
});

describe("extractCodeSpans / citationTokens", () => {
  // Pins CommonMark's run-length rule against the construction RULE 15's own worked example uses:
  // a DOUBLE-backtick span quoting two single-backtick citations. A single-backtick reader loses
  // both of them into the gap between matches, where nothing counts them.
  // The assertion is the FULL token list, not just the two citations: a single-backtick reader
  // happens to recover those two anyway on this line, because the delimiters pair up evenly by
  // coincidence. What it CANNOT produce is the quoting span's own content, so only an exact list
  // discriminates a run-length reader from the pattern this replaces.
  it("recovers both citations from the double-backtick span that quotes them", () => {
    const line = "Write ``see `egress_loop`'s `SceneSubscribe` arm``, never a line number.";
    expect(citationTokens(line).tokens.map((t) => t.token)).toEqual([
      "see `egress_loop`'s `SceneSubscribe` arm",
      "egress_loop",
      "SceneSubscribe",
    ]);
  });

  it("pairs a span that WRAPS a line break, so later spans on the line stay aligned", () => {
    // A line-scoped reader pairs the wrap's ORPHAN closing backtick with the next span's opener:
    // it emits the prose BETWEEN the two spans ("and") as a token the author never wrote, and
    // loses the real citation entirely. The exact list is what discriminates that reader: it
    // fails on the manufactured token AND on the lost citation, where a containment check on
    // either one alone leaves the other side untested.
    const tokens = citationTokens("a `foo ==\nbar` and `MoveOutcome.cost` here.").tokens.map(
      (t) => t.token,
    );
    expect(tokens).toEqual(["foo ==\nbar", "MoveOutcome.cost"]);
  });

  it("stops pairing at a blank line, so one stray backtick cannot unpair the rest of a file", () => {
    const text = "A stray ` backtick ends the paragraph.\n\nThen `MoveOutcome.cost` and `Grid`.";
    expect(citationTokens(text).tokens.map((t) => t.token)).toEqual(["MoveOutcome.cost", "Grid"]);
  });

  it("recovers a citation nested two spans deep, not just one", () => {
    expect(citationTokens("```A ``B `C` `` ```").tokens.map((t) => t.token)).toContain("C");
  });

  it("reports the line a span OPENS on", () => {
    expect(citationTokens("x\ny `Foo::bar`").tokens).toEqual([{ token: "Foo::bar", line: 2 }]);
  });

  it("leaves an unmatched backtick run as literal text, and COUNTS it as unpaired", () => {
    const { spans, runs, unpairedRuns } = extractCodeSpans("a ` b");
    expect(spans).toEqual([]);
    expect({ runs, unpairedRuns }).toEqual({ runs: 1, unpairedRuns: 1 });
  });
});

describe("stripCodeBlocks", () => {
  it("pairs only fence delimiters that OPEN a line, so an inline mention blanks no prose", () => {
    const text = "An inline ```ts fence mention.\n\n`Grid.cellCenter` is cited here.\n";
    expect(stripCodeBlocks(text).body).toContain("`Grid.cellCenter`");
  });

  it("blanks a four-space-indented code block that follows a blank line", () => {
    const text = "Prose.\n\n    let notACitation = 1;\n\nMore prose.\n";
    expect(stripCodeBlocks(text).body).toBe("Prose.\n\n\n\nMore prose.\n");
  });

  it("keeps an indented CONTINUATION of a list item, whose citations are real prose", () => {
    const text = "  - A bullet whose body wraps.\n\n    `Whisper` is cited in the continuation.\n";
    expect(stripCodeBlocks(text).body).toContain("`Whisper`");
  });

  // The delimiter test allows leading indentation, so a lone fence line inside an INDENTED code
  // block would open a real fence if it were tested first — and with nothing later closing it, a
  // document whose fences are balanced gets reported as ending inside an unclosed one. Revert
  // direction: move the delimiter test back above the indented-block branch and both expectations
  // below flip.
  it("does not open a fence on a delimiter line INSIDE an indented code block", () => {
    const text = [
      "Prose.",
      "",
      "    let x = 1;",
      "    ```",
      "    let y = 2;",
      "",
      "More `B` prose.",
      "",
    ].join("\n");
    const result = stripCodeBlocks(text);
    expect(result.unterminatedFence).toBe(false);
    expect(result.body).toContain("`B`");
  });

  // Block stripping is the widest exclusion here and the only one that removes whole LINES. A
  // fence misdetection has to move a number someone can see, or it removes arbitrary prose from
  // the gate with every printed total unchanged.
  it("REPORTS the lines it blanked and the backtick runs that went with them", () => {
    const text = "Prose `A`.\n\n```\n`InsideAFence`\n```\n\nMore `B`.\n";
    const { blankedLines, blankedRuns } = stripCodeBlocks(text);
    expect(blankedLines).toBe(3);
    expect(blankedRuns).toBe(4);
  });

  // Markdown gives a fence no terminator at end of file, so an unclosed one blanks the whole
  // remainder of the document. Conservation balances on it and `bodyRuns` falls to 0, which is the
  // measurement the per-file floor reads - the defect is invisible from every printed count unless
  // the strip itself says it ended inside a fence.
  it("reports a fence still open at end of file, and a closed one as closed", () => {
    const unclosed = stripCodeBlocks("Prose `A`.\n\n```\nlet x = 1;\n\nMore `B`.\n");
    expect(unclosed.unterminatedFence).toBe(true);
    expect(unclosed.body).not.toContain("`B`");
    const closed = stripCodeBlocks("Prose `A`.\n\n```\nlet x = 1;\n```\n\nMore `B`.\n");
    expect(closed.unterminatedFence).toBe(false);
    expect(closed.body).toContain("`B`");
  });
});

describe("extractCitationCandidates", () => {
  it("classifies a qualified citation as a candidate and a non-identifier span as a non-candidate", () => {
    const text = "`RegionField::is_arrest` and `Option<&RegionField>`.";
    const { candidates, nonCandidates } = extractCitationCandidates(text);
    expect(candidates.map((c) => c.token)).toEqual(["RegionField::is_arrest"]);
    expect(nonCandidates).toBe(1);
  });

  it("keeps a bare snake_case and a bare camelCase token as CANDIDATES, not an excluded shape", () => {
    const text = "`region_arrests` and `fakeCamelCitation` and `doc.engine`.";
    expect(extractCitationCandidates(text).candidates.map((c) => c.token)).toEqual([
      "region_arrests",
      "fakeCamelCitation",
      "doc.engine",
    ]);
  });

  it("excludes a fenced code block from candidates entirely", () => {
    const text = "```\n`NotACitation`\n```\n`RealCitation::method`";
    const { candidates } = extractCitationCandidates(text);
    expect(candidates.map((c) => c.token)).toEqual(["RealCitation::method"]);
  });

  it("COUNTS every span an EXAMPLE: marker exempts rather than dropping it into no bucket", () => {
    const text = "EXAMPLE: `NotReal::Thing` and `Option<&T>` demonstrate the shape.";
    const { candidates, nonCandidates, exampleExempt } = extractCitationCandidates(text);
    expect(candidates.length + nonCandidates).toBe(0);
    expect(exampleExempt).toBe(2);
  });

  it("checks the NAME in a `NAME=value` span, which is a citation with its value attached", () => {
    const { candidates } = extractCitationCandidates("The cap is `MAX_GATE_WALK_SAMPLES=4096`.");
    expect(candidates.map((c) => c.token)).toEqual(["MAX_GATE_WALK_SAMPLES"]);
  });

  it("does not read a comparison or an arrow as an assignment", () => {
    const { candidates, nonCandidates } = extractCitationCandidates("`w <= 0` and `k => v`");
    expect(candidates).toEqual([]);
    expect(nonCandidates).toBe(2);
  });

  it("excludes a filename-extension token as a non-candidate, not a broken citation", () => {
    const text = "See `CLAUDE.md` and `_semantic.scss` and `index.js`.";
    const { candidates, nonCandidates } = extractCitationCandidates(text);
    expect(candidates).toEqual([]);
    expect(nonCandidates).toBe(3);
  });
});

// `Array.sort` reaches its acknowledgement only because nothing in the tree declares `sort`:
// `Array` IS indexed (from `SchemaType::Array`), so the capitalized-head guard does not fire, and
// the token survives to the whole-token acknowledgement list. The day anything declares `sort`,
// the citation VERIFIES — against an unrelated declaration — and the entry dies zero-hit, so the
// obvious repair (deleting the dead entry) would cement a false verify. This test is the tripwire:
// it passes while that precondition holds and fails the moment it breaks, so the arrangement
// cannot change silently.
describe("the Array.sort acknowledgement's precondition", () => {
  it("holds only while the tree declares no `sort`", () => {
    const { declared } = buildSymbolIndex(REPO_ROOT);
    expect(declared.has("Array")).toBe(true);
    expect(
      declared.has("sort"),
      "the tree now declares `sort`, so the `Array.sort` citation resolves against it instead of " +
        "reaching its acknowledgement — the entry will die zero-hit and deleting it would leave " +
        "the false verify in place. Qualify the citation by its real owner.",
    ).toBe(false);
  });
});

describe("resolvesAgainstIndex", () => {
  const symbols = new Set(["AppContext", "AppContext.chat", "send", "documents", "RegionField"]);

  it("resolves an exact match and refuses an unindexed bare identifier", () => {
    expect(resolvesAgainstIndex("RegionField", symbols)).toBe(true);
    expect(resolvesAgainstIndex("region_arrests", symbols)).toBe(false);
  });

  it("resolves a value path by its member names when the head is a lowercase value", () => {
    expect(resolvesAgainstIndex("ctx.documents", symbols)).toBe(true);
    expect(resolvesAgainstIndex("ctx.notAMember", symbols)).toBe(false);
  });

  it("resolves past the longest indexed prefix", () => {
    expect(resolvesAgainstIndex("AppContext.chat.send", symbols)).toBe(true);
    expect(resolvesAgainstIndex("AppContext.chat.notAMember", symbols)).toBe(false);
  });

  it("refuses a capitalized head the index does not know, member name notwithstanding", () => {
    expect(resolvesAgainstIndex("MadeUpType.documents", symbols)).toBe(false);
  });
});

describe("checkFileCitations", () => {
  it("verifies a real symbol, acknowledges a named non-symbol, and reports an unresolved one", () => {
    const symbols = new Set(["RegionField::is_arrest"]);
    const text = "`RegionField::is_arrest` `Uuid` `TotallyMadeUp::NotReal`";
    const hits = new Map();
    const result = checkFileCitations(text, symbols, hits);
    expect(result.verified).toBe(1);
    expect(result.acknowledged).toBe(1);
    expect(hits.get("Uuid")).toBe(1);
    expect(result.broken).toEqual([{ line: 1, token: "TotallyMadeUp::NotReal" }]);
  });

  it("reports a QUALIFIED dead citation as broken, never absorbed by a bare-word entry", () => {
    // `fetch` and `mutation` are both acknowledged bare. A citation naming a type that does not
    // exist must still be BROKEN — a member-name fallback would keep the entry alive on a token
    // that has nothing to do with the reason the entry records.
    const result = checkFileCitations("`MadeUpType.fetch` `Gone::mutation`", new Set());
    expect(result.broken.map((b) => b.token)).toEqual(["MadeUpType.fetch", "Gone::mutation"]);
    expect(result.acknowledged).toBe(0);
  });

  // The other branch, and the shape that motivated the rule: the HEAD is indexed and the MEMBER is
  // dead — a renamed method on a type that still exists. The unknown-head case above takes an
  // earlier exit, so it leaves this path untested.
  it("reports a dead MEMBER of an INDEXED type as broken, not acknowledged", () => {
    const result = checkFileCitations(
      "`PanelsController.mutation`",
      new Set(["PanelsController"]),
    );
    expect(result.broken.map((b) => b.token)).toEqual(["PanelsController.mutation"]);
    expect(result.acknowledged).toBe(0);
  });

  it("still acknowledges a qualified path by its external-crate PREFIX", () => {
    const hits = new Map();
    const result = checkFileCitations("`tokio::spawn_blocking`", new Set(), hits);
    expect(result.acknowledged).toBe(1);
    expect(hits.get("tokio")).toBe(1);
  });

  it("carries the EXAMPLE-exempt span count through to its caller", () => {
    const result = checkFileCitations("EXAMPLE: `M13-0` is a specimen.", new Set());
    expect(result.exampleExempt).toBe(1);
  });
});

describe("importBindingNames", () => {
  // The acknowledgement assertion asks whether this tree DECLARES a name. Importing one is not
  // declaring it, whatever the specifier — an internal import's target is declared by the file that
  // declares it, and reading the import as a second declaration is what the split avoids.
  it("collects every import binding and no local declaration", () => {
    const text = [
      'import { DockviewApi } from "dockview-core";',
      'import ts from "typescript";',
      'import * as fs from "node:fs";',
      'import { pickSheet } from "@shadowcat/core";',
      'import { local } from "./neighbour";',
      "export function declaredHere() {}",
    ].join("\n");
    const names = importBindingNames(text);
    expect([...names].sort()).toEqual(["DockviewApi", "fs", "local", "pickSheet", "ts"]);
  });

  // The routing invariant behind the split: one file that imports a name AND declares a member of
  // the same name would send both to the reference half, masking a real declaration from the
  // assertion that reads it. `extractTsSymbols` emits no import binding, so the two cannot
  // collide.
  it("extractTsSymbols emits a re-export but not an import binding", () => {
    const text = [
      'import { Collision } from "dockview-core";',
      "export interface Holder { Collision: string }",
      'export { inner as CollisionAlias } from "./neighbour";',
    ].join("\n");
    const names = extractTsSymbols(text);
    expect(names.has("Holder.Collision")).toBe(true);
    expect(names.has("CollisionAlias")).toBe(true);
    expect(importBindingNames(text).has("Collision")).toBe(true);
  });
});

describe("span conservation", () => {
  it("counts every backtick RUN, whatever its length", () => {
    expect(countBacktickRuns("a `b` and ``c `d` c`` and a stray `")).toBe(7);
  });

  // The accounting identity itself, on a hand-computed set of terms: every run in the document is
  // either blanked with its block, left unpaired, or one of the two delimiters of a span that
  // landed in exactly one bucket.
  it("is satisfied when every run is accounted for, and violated when one bucket is dropped", () => {
    const accounting = {
      rawRuns: 12,
      blankedRuns: 2,
      unpairedRuns: 2,
      emptySpans: 1,
      exampleExempt: 1,
      nonCandidates: 1,
      verified: 1,
      acknowledged: 0,
      broken: 0,
    };
    expect(spanAccountingDelta(accounting)).toBe(0);
    // The leak this invariant exists to make impossible: a bucket that stops being counted while
    // the gate's own output stays green.
    expect(spanAccountingDelta({ ...accounting, emptySpans: 0 })).toBe(2);
  });

  // The end-to-end identity over one document that exercises EVERY term at once. A term that stops
  // being counted anywhere in the pipeline shows up here as a non-zero delta, whatever produced it.
  it("balances a document carrying a block, an unpaired run, an empty span and every bucket", () => {
    const text = [
      "```",
      "`InsideAFence`",
      "```",
      "",
      "A stray ` run.",
      "",
      "An empty `` `` span, `RegionField::is_arrest`, `Uuid`, `NotAThing::atAll`,",
      "`Option<&T>`.",
      "",
      "EXAMPLE: `M13-0` is a specimen.",
    ].join("\n");
    const result = checkFileCitations(text, new Set(["RegionField::is_arrest"]));
    expect(result.accounting.blankedBlockLines).toBe(3);
    expect(result.accounting.blankedRuns).toBe(4);
    expect(result.accounting.unpairedRuns).toBe(1);
    expect(result.accounting.emptySpans).toBe(1);
    expect(result.verified).toBe(1);
    expect(result.acknowledged).toBe(1);
    expect(result.broken).toHaveLength(1);
    expect(result.nonCandidates).toBe(1);
    expect(result.exampleExempt).toBe(1);
    expect(spanAccountingDelta(result.accounting)).toBe(0);
  });

  it("balances every tracked skill file in this repo, and every file individually", () => {
    const result = checkSkillSymbolRefs(REPO_ROOT);
    expect(result.conservationDelta).toBe(0);
    expect(result.conservationFailures).toEqual([]);
  });
});

describe("listSkillDirs", () => {
  it("includes a committed skill directory and excludes an untracked one", () => {
    // A fixed, reused-in-place directory under .claude/skills (this repo permits no
    // permanent-deletion call, so a per-run temp directory would accumulate forever) — gitignored
    // by name, so it is real, untracked filesystem state `listSkillDirs` must classify correctly,
    // without depending on the graphify tool actually having been run locally (a fresh CI checkout
    // never has that directory).
    mkdirSync(join(REPO_ROOT, ".claude", "skills", "__listskilldirs_test_fixture__"), {
      recursive: true,
    });
    const dirs = listSkillDirs(REPO_ROOT);
    expect(dirs).not.toBeNull();
    expect(dirs.tracked.has("shadowcat-codebase-core")).toBe(true);
    expect(dirs.untracked).toContain("__listskilldirs_test_fixture__");
  });
});

describe("checkSkillSymbolRefs", () => {
  // One fixture tree for the whole suite, at ONE path rewritten in place: this repo permits no
  // permanent-deletion call, so a per-run temp directory accumulates forever. Reuse is safe
  // because every test writes the prose it needs and the corpus-size case below asserts the scan
  // saw exactly one file, so a file left behind by an older fixture turns a test red rather than
  // quietly joining the corpus.
  const repoRoot = FIXTURE_ROOT;
  const skillsRoot = join(repoRoot, ".claude", "skills");
  const skillFile = join(skillsRoot, "shadowcat-codebase-example", "SKILL.md");
  const trackedDirs = new Set(["shadowcat-codebase-example"]);

  mkdirSync(join(repoRoot, "src", "server", "src", "scene"), { recursive: true });
  mkdirSync(join(repoRoot, "src", "server", "migrations"), { recursive: true });
  writeFileSync(
    join(repoRoot, "src", "server", "src", "scene", "regions.rs"),
    "impl RegionField {\n    pub fn is_arrest(&self) -> bool { true }\n}\n",
  );
  for (const dir of ["client", "modules", "types"])
    mkdirSync(join(repoRoot, "src", dir), { recursive: true });
  mkdirSync(join(repoRoot, "scripts"), { recursive: true });
  mkdirSync(join(skillsRoot, "shadowcat-codebase-example"), { recursive: true });
  mkdirSync(join(skillsRoot, "graphify"), { recursive: true });
  writeFileSync(
    join(skillsRoot, "graphify", "SKILL.md"),
    "See `some_python_function` and `AnotherPythonThing`.",
  );

  const run = (prose) => {
    writeFileSync(skillFile, prose);
    return checkSkillSymbolRefs(repoRoot, { trackedDirs, untrackedDirs: ["graphify"] });
  };

  it("reports zero broken when every citation resolves — the positive-control CLEAN direction", () => {
    const result = run("See `RegionField::is_arrest`.");
    expect(result.broken).toEqual([]);
    expect(result.verified).toBeGreaterThan(0);
  });

  // Non-vacuity / positive-control BROKEN direction, on BOTH specimen shapes: a bare snake_case
  // token with one underscore and a bare camelCase token. Neither shape may be excluded from
  // resolution — a shape exclusion would pass both silently.
  it("reports a bare snake_case and a bare camelCase citation as broken, verbatim", () => {
    const result = run("The `region_arrests` predicate and the `fakeCamelCitation` helper.");
    expect(result.broken).toEqual([
      { file: skillFile, line: 1, token: "region_arrests" },
      { file: skillFile, line: 1, token: "fakeCamelCitation" },
    ]);
  });

  it("does not scan an untracked (vendored) skill directory", () => {
    const result = run("See `RegionField::is_arrest`.");
    expect(result.filesScanned).toBe(1);
  });

  it("names an acknowledgement entry the corpus never reaches, and spares one it does", () => {
    const result = run("See `RegionField::is_arrest` and `Uuid`.");
    expect(result.unusedAcknowledgements).not.toContain("Uuid");
    expect(result.unusedAcknowledgements).toContain("NOCASE");
  });

  // The hole a body-run measurement opens on its own: an unclosed fence with no citation above it
  // blanks every remaining line, so `bodyRuns` is 0, the floor stays silent, conservation balances
  // because those runs are counted as blanked, and the global candidate guard is held up by the
  // other files. The fence signal is the only thing that fails on it.
  it("FAILS a file that ends inside an unclosed fence, which no other count can see", () => {
    const result = run("```\nlet x = 1;\n\n`RegionField::is_arrest` is swallowed whole.\n");
    expect(result.filesWithUnterminatedFence).toEqual([skillFile]);
    expect(result.filesWithNoCandidates).toEqual([]);
    expect(result.conservationDelta).toBe(0);
    // The citation the fence swallowed: the same prose without the fence verifies.
    expect(result.verified).toBe(0);
  });

  it("names no file when every fence closes", () => {
    const result = run("```\nlet x = 1;\n```\n\n`RegionField::is_arrest` survives.\n");
    expect(result.filesWithUnterminatedFence).toEqual([]);
    expect(result.verified).toBeGreaterThan(0);
  });

  it("floors a file that carries backticks but yields no classified span", () => {
    const result = run("An unpaired ` delimiter and nothing else.");
    expect(result.filesWithNoCandidates.map((f) => f.file)).toEqual([skillFile]);
  });

  // The floor's ACTUAL failure mode, which the demonstrated one does not reach: a stray delimiter
  // shifts pairing so the prose between the real citations becomes the spans. That prose is full of
  // spaces, so every shifted span fails both shapes and climbs `nonCandidates` — a floor that stays
  // silent while `nonCandidates` is non-zero is held shut on precisely the case it exists for.
  it("floors a file whose spans are ALL non-candidates, and reports what they were", () => {
    const result = run("`a stray shifted span` and `another shifted phrase here`.");
    expect(result.filesWithNoCandidates).toEqual([
      { file: skillFile, nonCandidates: 2, exampleExempt: 0, emptySpans: 0, unpairedRuns: 0 },
    ]);
  });

  // The case the per-file floor cannot reach, and the reason the top-level unpaired count is
  // separate and fatal: one paragraph's pairing is shifted by a stray delimiter while the other
  // paragraphs keep yielding real citations, so the floor stays silent, conservation still
  // balances, and the shifted spans quietly climb the not-citation-shaped bucket.
  it("counts a top-level unpaired run in a paragraph the other paragraphs keep healthy", () => {
    const result = run(
      "A stray ` delimiter opens here and `RegionField::is_arrest` is swallowed by it.\n\n" +
        "This paragraph cites `RegionField::is_arrest` and is unaffected.",
    );
    expect(result.filesWithNoCandidates).toEqual([]);
    expect(result.conservationDelta).toBe(0);
    expect(result.accounting.topLevelUnpairedRuns).toBe(1);
  });

  it("counts an EXAMPLE-exempt span rather than dropping it into no bucket", () => {
    const result = run("EXAMPLE: `M13-0` is a specimen.\n\nSee `RegionField::is_arrest`.");
    expect(result.exampleExempt).toBe(1);
    expect(result.verified).toBeGreaterThan(0);
  });
});
