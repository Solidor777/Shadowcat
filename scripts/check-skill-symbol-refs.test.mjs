import { describe, it, expect } from "vitest";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  extractRustSymbols,
  extractRustReexports,
  extractRustLiteralAlternatives,
  extractSqlSymbols,
  extractTomlKeys,
  extractJsonKeys,
  moduleNameOf,
  applyRenameAll,
  splitSourceLines,
  rustModulePath,
  extractTsSymbols,
  extractSvelteScript,
  extractCodeSpans,
  stripCodeBlocks,
  citationTokens,
  extractCitationCandidates,
  resolvesAgainstIndex,
  checkFileCitations,
  checkSkillSymbolRefs,
  listSkillDirs,
} from "./check-skill-symbol-refs.mjs";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

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

  it("scopes a local to the fn that declares it, not to a sibling fn", () => {
    const text = [
      "pub fn first() {",
      "    let only_here = 1;",
      "}",
      "pub fn second() {",
      "    let elsewhere = 2;",
      "}",
    ].join("\n");
    const names = extractRustSymbols(text);
    expect(names.has("first::only_here")).toBe(true);
    expect(names.has("second::only_here")).toBe(false);
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

describe("extractRustLiteralAlternatives", () => {
  it("indexes each member of a string-literal alternation pattern", () => {
    const text = 'matches!(doc_type, "token" | "scene" | "drawing")';
    expect([...extractRustLiteralAlternatives(text)].sort()).toEqual([
      "drawing",
      "scene",
      "token",
    ]);
  });

  it("ignores a lone string literal, which declares no value set", () => {
    expect(extractRustLiteralAlternatives('let s = "drawing";').size).toBe(0);
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

  it("indexes each identifier-shaped string in a module-level array-literal value set", () => {
    const text = 'export const NOTATION_KEYWORDS: readonly string[] = ["d", "kh", "cs"];';
    const names = extractTsSymbols(text);
    expect(names.has("kh")).toBe(true);
    expect(names.has("cs")).toBe(true);
  });

  it("ignores an array literal that is not a module-level declaration's value set", () => {
    const names = extractTsSymbols('function f() {\n  accept(["kh", "kl"]);\n}');
    expect(names.has("kh")).toBe(false);
  });

  it("does NOT index a collection constructor's members, which would index this gate's own lists", () => {
    // `scripts/` is an indexed root and `ACKNOWLEDGED_NON_SYMBOLS` is declared as `new Set([...])`.
    // Unwrapping the constructor makes every acknowledged token resolve as a declared name, so the
    // list goes dead and the gate reports as verified the exact citations it exists to flag.
    expect(extractTsSymbols('export const ACK = new Set(["Uuid"]);').has("Uuid")).toBe(false);
  });

  it("indexes the wire VALUE a module-level string constant declares", () => {
    const names = extractTsSymbols('export const ITEM_DOC_TYPE = "item";');
    expect(names.has("ITEM_DOC_TYPE")).toBe(true);
    expect(names.has("item")).toBe(true);
  });

  it("ignores a function-local string constant, which publishes no value", () => {
    expect(extractTsSymbols('function f() {\n  const k = "item";\n}').has("item")).toBe(false);
  });

  it("indexes an imported name and a string-literal type member", () => {
    const text = [
      'import { createSubscriber } from "svelte/reactivity";',
      'export type SyncState = "none" | "up_to_date";',
    ].join("\n");
    const names = extractTsSymbols(text);
    expect(names.has("createSubscriber")).toBe(true);
    expect(names.has("up_to_date")).toBe(true);
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
    expect(citationTokens(line).map((t) => t.token)).toEqual([
      "see `egress_loop`'s `SceneSubscribe` arm",
      "egress_loop",
      "SceneSubscribe",
    ]);
  });

  it("pairs a span that WRAPS a line break, so later spans on the line stay aligned", () => {
    // A line-scoped reader pairs the wrap's ORPHAN closing backtick with the next span's opener:
    // it emits the prose BETWEEN the two spans ("and") as a token the author never wrote, and
    // loses the real citation entirely. Both assertions below discriminate that reader.
    const tokens = citationTokens("a `foo ==\nbar` and `MoveOutcome.cost` here.").map((t) => t.token);
    expect(tokens).toEqual(["foo ==\nbar", "MoveOutcome.cost"]);
    expect(tokens).not.toContain("and");
  });

  it("stops pairing at a blank line, so one stray backtick cannot unpair the rest of a file", () => {
    const text = "A stray ` backtick ends the paragraph.\n\nThen `MoveOutcome.cost` and `Grid`.";
    expect(citationTokens(text).map((t) => t.token)).toEqual(["MoveOutcome.cost", "Grid"]);
  });

  it("recovers a citation nested two spans deep, not just one", () => {
    expect(citationTokens("```A ``B `C` `` ```").map((t) => t.token)).toContain("C");
  });

  it("reports the line a span OPENS on", () => {
    expect(citationTokens("x\ny `Foo::bar`")).toEqual([{ token: "Foo::bar", line: 2 }]);
  });

  it("leaves an unmatched backtick run as literal text rather than opening a span", () => {
    expect(extractCodeSpans("a ` b")).toEqual([]);
  });
});

describe("stripCodeBlocks", () => {
  it("pairs only fence delimiters that OPEN a line, so an inline mention blanks no prose", () => {
    const text = "An inline ```ts fence mention.\n\n`Grid.cellCenter` is cited here.\n";
    expect(stripCodeBlocks(text)).toContain("`Grid.cellCenter`");
  });

  it("blanks a four-space-indented code block that follows a blank line", () => {
    const text = "Prose.\n\n    let notACitation = 1;\n\nMore prose.\n";
    expect(stripCodeBlocks(text)).toBe("Prose.\n\n\n\nMore prose.\n");
  });

  it("keeps an indented CONTINUATION of a list item, whose citations are real prose", () => {
    const text = "  - A bullet whose body wraps.\n\n    `Whisper` is cited in the continuation.\n";
    expect(stripCodeBlocks(text)).toContain("`Whisper`");
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

describe("listSkillDirs", () => {
  it("includes a committed skill directory and excludes the untracked vendored one", () => {
    const dirs = listSkillDirs(REPO_ROOT);
    expect(dirs).not.toBeNull();
    expect(dirs.tracked.has("shadowcat-codebase-core")).toBe(true);
    expect(dirs.untracked).toContain("graphify");
  });
});

describe("checkSkillSymbolRefs", () => {
  // One fixture tree for the whole suite: each test writes the skill prose it needs. Nothing is
  // deleted afterwards — the OS owns the reclamation of its own temp directory, and this repo
  // permits no permanent-deletion call.
  const repoRoot = mkdtempSync(join(tmpdir(), "symbol-refs-repo-"));
  const skillsRoot = join(repoRoot, ".claude", "skills");
  const skillFile = join(skillsRoot, "shadowcat-codebase-example", "SKILL.md");
  const trackedDirs = new Set(["shadowcat-codebase-example", "shadowcat-codebase-nightfox"]);

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
  mkdirSync(join(skillsRoot, "shadowcat-codebase-nightfox"), { recursive: true });
  mkdirSync(join(skillsRoot, "graphify"), { recursive: true });
  const nightfoxFile = join(skillsRoot, "shadowcat-codebase-nightfox", "SKILL.md");
  writeFileSync(nightfoxFile, "See `parseNightfox`.");
  writeFileSync(
    join(skillsRoot, "graphify", "SKILL.md"),
    "See `some_python_function` and `AnotherPythonThing`.",
  );

  const run = (prose, nightfoxProse = "See `parseNightfox`.") => {
    writeFileSync(skillFile, prose);
    writeFileSync(nightfoxFile, nightfoxProse);
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

  it("acknowledges a NAMED cross-repo symbol in the skill that documents the other repo", () => {
    const result = run("See `RegionField::is_arrest`.", "See `parseNightfox`.");
    expect(result.broken).toEqual([]);
    expect(result.crossRepo).toBe(1);
    expect(result.crossRepoHits.get("parseNightfox")).toBe(1);
  });

  it("reports an UNNAMED cross-repo citation as broken — the file is no longer exempt", () => {
    const result = run("See `RegionField::is_arrest`.", "See `NightfoxOnlyType::NotInThisRepo`.");
    expect(result.broken.map((b) => b.token)).toEqual(["NightfoxOnlyType::NotInThisRepo"]);
  });

  it("acknowledges a cross-repo name ONLY inside that skill's own file", () => {
    const result = run("See `parseNightfox`.", "See `parseNightfox`.");
    expect(result.broken.map((b) => b.token)).toEqual(["parseNightfox"]);
  });

  it("does not scan an untracked (vendored) skill directory", () => {
    const result = run("See `RegionField::is_arrest`.");
    expect(result.filesScanned).toBe(2);
  });

  it("names an acknowledgement entry the corpus never reaches, and spares one it does", () => {
    const result = run("See `RegionField::is_arrest` and `Uuid`.");
    expect(result.unusedAcknowledgements).not.toContain("Uuid");
    expect(result.unusedAcknowledgements).toContain("NOCASE");
  });

  it("floors a file that carries backticks but yields no classified span", () => {
    const result = run("An unpaired ` delimiter and nothing else.");
    expect(result.filesWithNoCandidates).toEqual([skillFile]);
  });

  it("counts an EXAMPLE-exempt span rather than dropping it into no bucket", () => {
    const result = run("EXAMPLE: `M13-0` is a specimen.\n\nSee `RegionField::is_arrest`.");
    expect(result.exampleExempt).toBe(1);
    expect(result.verified).toBeGreaterThan(0);
  });
});
