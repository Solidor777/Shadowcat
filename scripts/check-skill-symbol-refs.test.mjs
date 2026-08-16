import { describe, it, expect, afterEach } from "vitest";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  extractRustSymbols,
  extractRustReexports,
  rustModulePath,
  extractTsExports,
  extractTsMembers,
  extractTsStringLiteralUnionMembers,
  extractSvelteScript,
  dedent,
  extractCitationCandidates,
  checkFileCitations,
  checkSkillSymbolRefs,
  deriveSnakeCaseAliases,
} from "./check-skill-symbol-refs.mjs";

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

describe("extractTsExports", () => {
  it("indexes an exported declaration and a non-exported module-level one", () => {
    const text = "export function foo() {}\nconst BAR = 1;\nfunction inner() {\n  const local = 1;\n}\n";
    const names = extractTsExports(text);
    expect(names.has("foo")).toBe(true);
    expect(names.has("BAR")).toBe(true);
    expect(names.has("inner")).toBe(true);
    expect(names.has("local")).toBe(false);
  });

  it("indexes the EXPORTED alias of a named re-export, not the local name", () => {
    const text = "export { a as b };";
    const names = extractTsExports(text);
    expect(names.has("b")).toBe(true);
    expect(names.has("a")).toBe(false);
  });
});

describe("extractTsMembers", () => {
  it("indexes an interface property, a class method (incl. async/private), and an arrow field", () => {
    const text = [
      "export interface Foo {",
      "  bar: string;",
      "}",
      "export class Baz {",
      "  private async open(): Promise<void> {}",
      "  private readonly viewedScene = (): string | null => null;",
      "}",
    ].join("\n");
    const names = extractTsMembers(text);
    expect(names.has("bar")).toBe(true);
    expect(names.has("Foo.bar")).toBe(true);
    expect(names.has("Baz.open")).toBe(true);
    expect(names.has("Baz.viewedScene")).toBe(true);
  });
});

describe("extractTsStringLiteralUnionMembers", () => {
  it("indexes every member of an exported string-literal union type", () => {
    const text = 'export type SyncState = "none" | "up_to_date" | "template_changed";';
    const names = extractTsStringLiteralUnionMembers(text);
    expect([...names].sort()).toEqual(["none", "template_changed", "up_to_date"]);
  });
});

describe("deriveSnakeCaseAliases", () => {
  it("derives a multi-word compound's snake_case wire form, never a single word's", () => {
    const derived = deriveSnakeCaseAliases(new Set(["GmOnly", "Value"]));
    expect(derived).toContain("gm_only");
    expect(derived).not.toContain("value");
  });
});

describe("extractSvelteScript / dedent", () => {
  it("dedents the script body so a top-level statement lands at column 0", () => {
    const svelte = "<div></div>\n<script>\n  function boot() {}\n</script>\n";
    const script = extractSvelteScript(svelte);
    const exported = extractTsExports(script);
    expect(exported.has("boot")).toBe(true);
  });

  it("dedent is a no-op on already-flush text", () => {
    expect(dedent("a\nb\n")).toBe("a\nb\n");
  });
});

describe("extractCitationCandidates", () => {
  it("classifies a qualified citation as a candidate, a flat lowercase word as ambiguous, and a non-identifier span as a non-candidate", () => {
    const text = "`RegionField::is_arrest` and `item` and `Option<&RegionField>`.";
    const { candidates, ambiguous, nonCandidates } = extractCitationCandidates(text);
    expect(candidates.map((c) => c.token)).toEqual(["RegionField::is_arrest"]);
    expect(ambiguous.map((a) => a.token)).toEqual(["item"]);
    expect(nonCandidates).toBe(1);
  });

  it("excludes a fenced code block from candidates entirely", () => {
    const text = "```\n`NotACitation`\n```\n`RealCitation::method`";
    const { candidates } = extractCitationCandidates(text);
    expect(candidates.map((c) => c.token)).toEqual(["RealCitation::method"]);
  });

  it("skips a line carrying the EXAMPLE: marker", () => {
    const text = "EXAMPLE: `NotReal::Thing` demonstrates the shape.";
    const { candidates, ambiguous, nonCandidates } = extractCitationCandidates(text);
    expect(candidates.length + ambiguous.length + nonCandidates).toBe(0);
  });

  it("excludes a filename-extension token as a non-candidate, not a broken citation", () => {
    const text = "See `CLAUDE.md` and `_semantic.scss`.";
    const { candidates } = extractCitationCandidates(text);
    expect(candidates).toEqual([]);
  });

  it("treats a short bare snake_case word and a lowercase-first dot chain as ambiguous, but a long compound as a candidate", () => {
    const text = "`db_path` and `doc.engine` and `frozen_parity_king_step_grid_outcomes`.";
    const { candidates, ambiguous } = extractCitationCandidates(text);
    expect(ambiguous.map((a) => a.token).sort()).toEqual(["db_path", "doc.engine"]);
    expect(candidates.map((c) => c.token)).toEqual(["frozen_parity_king_step_grid_outcomes"]);
  });
});

describe("checkFileCitations", () => {
  it("verifies a real symbol, acknowledges a named non-symbol, and reports an unresolved one as broken", () => {
    const symbols = new Set(["RegionField::is_arrest"]);
    const text = "`RegionField::is_arrest` `Uuid` `TotallyMadeUp::NotReal`";
    const result = checkFileCitations(text, symbols);
    expect(result.verified).toBe(1);
    expect(result.acknowledged).toBe(1);
    expect(result.broken).toEqual([{ line: 1, token: "TotallyMadeUp::NotReal" }]);
  });
});

describe("checkSkillSymbolRefs", () => {
  let repoRoot;
  let skillsRoot;
  afterEach(() => {
    if (repoRoot) rmSync(repoRoot, { recursive: true, force: true });
  });

  function makeRepo() {
    repoRoot = mkdtempSync(join(tmpdir(), "symbol-refs-repo-"));
    mkdirSync(join(repoRoot, "src", "server", "src", "scene"), { recursive: true });
    writeFileSync(
      join(repoRoot, "src", "server", "src", "scene", "regions.rs"),
      "impl RegionField {\n    pub fn is_arrest(&self) -> bool { true }\n}\n",
    );
    for (const dir of ["client", "modules", "types"])
      mkdirSync(join(repoRoot, "src", dir), { recursive: true });
    mkdirSync(join(repoRoot, "scripts"), { recursive: true });
    skillsRoot = join(repoRoot, ".claude", "skills");
    mkdirSync(join(skillsRoot, "shadowcat-codebase-example"), { recursive: true });
  }

  it("reports zero broken when every citation resolves — the positive-control CLEAN direction", () => {
    makeRepo();
    writeFileSync(
      join(skillsRoot, "shadowcat-codebase-example", "SKILL.md"),
      "See `RegionField::is_arrest`.",
    );
    const result = checkSkillSymbolRefs(skillsRoot, repoRoot);
    expect(result.broken).toEqual([]);
    expect(result.verified).toBeGreaterThan(0);
  });

  // Non-vacuity / positive-control BROKEN direction: mutating a real citation to a name that
  // does not exist must surface it, verbatim, as a broken citation — never a silent pass.
  it("reports a broken citation, verbatim, when a citation is mutated to a nonexistent name", () => {
    makeRepo();
    const skillFile = join(skillsRoot, "shadowcat-codebase-example", "SKILL.md");
    writeFileSync(skillFile, "See `RegionField::is_arrest_MUTATED_DOES_NOT_EXIST`.");
    const result = checkSkillSymbolRefs(skillsRoot, repoRoot);
    expect(result.broken).toEqual([
      { file: skillFile, line: 1, token: "RegionField::is_arrest_MUTATED_DOES_NOT_EXIST" },
    ]);
  });

  it("excludes shadowcat-codebase-nightfox from the fatal scan but still counts its unresolved candidates", () => {
    makeRepo();
    mkdirSync(join(skillsRoot, "shadowcat-codebase-nightfox"), { recursive: true });
    writeFileSync(
      join(skillsRoot, "shadowcat-codebase-nightfox", "SKILL.md"),
      "See `NightfoxOnlyType::NotInThisRepo`.",
    );
    const result = checkSkillSymbolRefs(skillsRoot, repoRoot);
    expect(result.broken).toEqual([]);
    expect(result.nightfoxExcludedFiles).toBe(1);
    expect(result.nightfoxExcludedBroken).toBe(1);
  });

  it("does not scan a non-shadowcat-codebase skill (e.g. a vendored tool skill)", () => {
    makeRepo();
    writeFileSync(
      join(skillsRoot, "shadowcat-codebase-example", "SKILL.md"),
      "See `RegionField::is_arrest`.",
    );
    mkdirSync(join(skillsRoot, "graphify"), { recursive: true });
    writeFileSync(
      join(skillsRoot, "graphify", "SKILL.md"),
      "See `some_python_function` and `AnotherPythonThing`.",
    );
    const result = checkSkillSymbolRefs(skillsRoot, repoRoot);
    expect(result.broken).toEqual([]);
    expect(result.filesScanned).toBe(1); // only the shadowcat-codebase-example file
  });
});
