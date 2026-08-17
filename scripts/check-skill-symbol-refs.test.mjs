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
    const names = extractRustSymbols(text);
    expect(names.has("db_path")).toBe(true);
    expect(names.has("out_dir")).toBe(true);
    expect(names.has("t_max_i")).toBe(true);
    expect(names.has("leg_start")).toBe(true);
    expect(names.has("leg_end")).toBe(true);
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
    expect(names.has("local")).toBe(true);
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
    expect(names.has("panelId")).toBe(true);
  });

  it("indexes an object-literal key, including a quoted dotted catalog key", () => {
    const text = 'export const messages = { "panels.popoutRestoredFloating": "x", plain: 1 };';
    const names = extractTsSymbols(text);
    expect(names.has("panels.popoutRestoredFloating")).toBe(true);
    expect(names.has("messages.plain")).toBe(true);
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
  it("recovers both citations from a double-backtick span that quotes them", () => {
    const line = "Write ``see `egress_loop`'s `SceneSubscribe` arm``, never a line number.";
    expect(citationTokens(line).map((t) => t.token)).toContain("egress_loop");
    expect(citationTokens(line).map((t) => t.token)).toContain("SceneSubscribe");
  });

  it("counts the quoting span itself too, so no span goes unaccounted for", () => {
    const line = "``a `b` c``";
    expect(citationTokens(line).map((t) => t.token)).toEqual(["a `b` c", "b"]);
  });

  it("pairs a span that WRAPS a line break, so later spans on the line stay aligned", () => {
    const text = "even though `stop_index ==\npath.len()-1`. `MoveOutcome.cost` accumulates.";
    const tokens = citationTokens(text).map((t) => t.token);
    expect(tokens).toContain("MoveOutcome.cost");
    expect(tokens).not.toContain("accumulates");
  });

  it("reports the line a span OPENS on", () => {
    expect(citationTokens("x\ny `Foo::bar`")).toEqual([{ token: "Foo::bar", line: 2 }]);
  });

  it("leaves an unmatched backtick run as literal text rather than opening a span", () => {
    expect(extractCodeSpans("a ` b")).toEqual([]);
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

  it("skips a line carrying the EXAMPLE: marker", () => {
    const text = "EXAMPLE: `NotReal::Thing` demonstrates the shape.";
    const { candidates, nonCandidates } = extractCitationCandidates(text);
    expect(candidates.length + nonCandidates).toBe(0);
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
});

describe("listSkillDirs", () => {
  it("includes a committed skill directory and excludes the untracked vendored one", () => {
    const dirs = listSkillDirs(REPO_ROOT);
    expect(dirs).not.toBeNull();
    expect(dirs.tracked.has("shadowcat-codebase-core")).toBe(true);
    expect(dirs.tracked.has("graphify")).toBe(false);
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
  writeFileSync(
    join(skillsRoot, "shadowcat-codebase-nightfox", "SKILL.md"),
    "See `NightfoxOnlyType::NotInThisRepo`.",
  );
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

  it("excludes shadowcat-codebase-nightfox from the fatal scan but still counts its candidates", () => {
    const result = run("See `RegionField::is_arrest`.");
    expect(result.broken).toEqual([]);
    expect(result.nightfoxExcludedFiles).toBe(1);
    expect(result.nightfoxExcludedBroken).toBe(1);
  });

  it("does not scan an untracked (vendored) skill directory", () => {
    const result = run("See `RegionField::is_arrest`.");
    expect(result.filesScanned).toBe(1);
    expect(result.untrackedDirs).toEqual(["graphify"]);
  });

  it("names every acknowledgement entry the corpus never reaches", () => {
    const result = run("See `RegionField::is_arrest`.");
    expect(result.unusedAcknowledgements).toContain("Uuid");
    expect(result.acknowledgedHits.size).toBe(0);
  });
});
