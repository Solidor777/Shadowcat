import { describe, it, expect } from "vitest";
import { fileURLToPath } from "node:url";
import { resolve, join } from "node:path";
import {
  extractExamples,
  workspacePackageDirs,
  candidateFiles,
  svelteFiles,
  packageForFile,
  analyzeExample,
  workspacePaths,
  externalDepPaths,
  packageOwnName,
  hostTopLevelBindings,
  dedupeAgainstHost,
  hasTypeValueClash,
  upgradeTypeOnlyImports,
  resolveTypeValueClashes,
  buildVirtualText,
  EXAMPLE_HYGIENE_OVERRIDES,
  findEnclosingClassTarget,
  pickSyntheticMethodName,
  buildClassInjectionText,
  buildLineMap,
} from "./extract-ts-examples.mjs";
import ts from "typescript";

function parse(text) {
  return ts.createSourceFile("host.ts", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
}

describe("extractExamples", () => {
  it("extracts a tagged ```ts fence inside an @example tag with its line number", () => {
    const src = [
      "/**",
      " * Adds.",
      " * @example",
      " * ```ts",
      " * const x = add(1, 2);",
      " * ```",
      " */",
      "export function add(a: number, b: number): number { return a + b; }",
    ].join("\n");
    const got = extractExamples(src);
    expect(got).toHaveLength(1);
    expect(got[0].code).toBe("const x = add(1, 2);");
    expect(got[0].line).toBe(4);
    expect(got[0].symbol).toBe("add");
    expect(got[0].ordinal).toBe(1);
  });

  it("extracts an untagged fence — fence tagging is not opt-in", () => {
    const src = [
      "/**",
      " * @example",
      " * ```",
      " * const untaggedControl = 1;",
      " * ```",
      " */",
      "export function bare(): void {}",
    ].join("\n");
    const got = extractExamples(src);
    expect(got).toHaveLength(1);
    expect(got[0].code).toBe("const untaggedControl = 1;");
  });

  it("ignores svelte-tagged fences and fences outside a doc comment", () => {
    const src = [
      "/**",
      " * @example",
      " * ```svelte",
      " * <Foo />",
      " * ```",
      " */",
      "/* not a doc comment",
      " * @example",
      " * ```ts",
      " * notCollected();",
      " * ```",
      " */",
    ].join("\n");
    expect(extractExamples(src)).toHaveLength(0);
  });

  it("extracts multiple examples from one file", () => {
    const one = "/**\n * @example\n * ```ts\n * a();\n * ```\n */\n";
    expect(extractExamples(one + one)).toHaveLength(2);
  });

  it("handles CRLF-delimited fences", () => {
    const src = ["/**", " * @example", " * ```ts", " * const crlfControl = 1;", " * ```", " */", ""].join("\r\n");
    const got = extractExamples(src);
    expect(got).toHaveLength(1);
    expect(got[0].code).toBe("const crlfControl = 1;");
  });

  it("assigns per-symbol ordinals across multiple examples on the same symbol", () => {
    const src = [
      "/**",
      " * @example",
      " * ```ts",
      " * one();",
      " * ```",
      " * @example",
      " * ```ts",
      " * two();",
      " * ```",
      " */",
      "export function fn(): void {}",
    ].join("\n");
    const got = extractExamples(src);
    expect(got).toHaveLength(2);
    expect(got[0].symbol).toBe("fn");
    expect(got[0].ordinal).toBe(1);
    expect(got[1].symbol).toBe("fn");
    expect(got[1].ordinal).toBe(2);
  });

  it("falls back to a module-level marker when no declaration follows the block", () => {
    const src = "/**\n * @example\n * ```ts\n * a();\n * ```\n */\n";
    const got = extractExamples(src);
    expect(got[0].symbol).toBe("<module top-level>");
  });
});

describe("workspacePackageDirs", () => {
  it("lists every workspace package directory", () => {
    const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
    const dirs = workspacePackageDirs(repo);
    expect(dirs).toContain("src/client/core");
    expect(dirs).toContain("src/types");
  });
});

describe("packageForFile", () => {
  it("maps an absolute file path to its owning workspace package by longest-prefix match", () => {
    const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
    const dirs = workspacePackageDirs(repo);
    const file = join(repo, "src", "client", "core", "src", "index.ts");
    expect(packageForFile(repo, dirs, file)).toBe("src/client/core");
  });

  it("returns null for a file outside every workspace package", () => {
    const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
    const dirs = workspacePackageDirs(repo);
    const file = join(repo, "scripts", "extract-ts-examples.mjs");
    expect(packageForFile(repo, dirs, file)).toBeNull();
  });
});

describe("analyzeExample", () => {
  it("hoists a top-level import out of the wrapper", () => {
    const { hoisted, rest, classContext } = analyzeExample(
      'import { readFileSync } from "node:fs";\nconst n = readFileSync("x").length;',
    );
    expect(hoisted).toEqual(['import { readFileSync } from "node:fs";']);
    expect(rest.join("\n")).toContain("readFileSync");
    expect(classContext).toBe(false);
  });

  it("hoists an exported declaration (which cannot carry a modifier inside a function)", () => {
    const { hoisted, rest } = analyzeExample("export function helper(): void {}\nhelper();");
    expect(hoisted).toEqual(["export function helper(): void {}"]);
    expect(rest).toEqual(["helper();"]);
  });

  it("leaves a plain non-exported statement in the wrapper", () => {
    const { hoisted, rest } = analyzeExample("const x = 1;\nconsole.log(x);");
    expect(hoisted).toEqual([]);
    expect(rest).toHaveLength(2);
  });

  it("flags a bare `this.` reference as needing class context", () => {
    const { classContext } = analyzeExample("this.doThing();");
    expect(classContext).toBe(true);
  });

  it("flags a bare private-identifier reference as needing class context", () => {
    const { classContext } = analyzeExample("console.log(this.#count);");
    expect(classContext).toBe(true);
  });

  it("does NOT flag `this` inside the example's own self-contained class", () => {
    const { classContext } = analyzeExample("class Local {\n  m() { return this.x; }\n}\nnew Local().m();");
    expect(classContext).toBe(false);
  });

  it("still flags `this` used inside a nested arrow function (arrows do not rebind this)", () => {
    const { classContext } = analyzeExample("const run = () => this.value;\nrun();");
    expect(classContext).toBe(true);
  });

  it("hoists a `declare const` statement — illegal inside a function body", () => {
    const { hoisted, rest } = analyzeExample("declare const store: number;\nconsole.log(store);");
    expect(hoisted).toEqual(["declare const store: number;"]);
    expect(rest).toEqual(["console.log(store);"]);
  });
});

describe("hostTopLevelBindings / dedupeAgainstHost", () => {
  it("maps a host's own top-level import bindings to their module specifier", () => {
    const bindings = hostTopLevelBindings('import { Foo, Bar as Baz } from "mod";\nimport * as NS from "other";\n');
    expect(bindings.get("Foo")).toEqual({ specifier: "mod", typeOnly: false });
    expect(bindings.get("Baz")).toEqual({ specifier: "mod", typeOnly: false });
    expect(bindings.get("NS")).toEqual({ specifier: "other", typeOnly: false });
  });

  it("drops a hoisted import entirely when every binding it introduces already comes from the host's identical specifier", () => {
    const bindings = hostTopLevelBindings('import { Foo } from "mod";\n');
    expect(dedupeAgainstHost('import { Foo } from "mod";', bindings)).toBeNull();
  });

  it("narrows a hoisted import to only the bindings the host does not already provide", () => {
    const bindings = hostTopLevelBindings('import { Foo } from "mod";\n');
    const narrowed = dedupeAgainstHost('import { Foo, Other } from "mod";', bindings);
    expect(narrowed).not.toBeNull();
    expect(narrowed).toContain("Other");
    expect(narrowed).not.toContain("Foo");
  });

  it("leaves a same-named import from a DIFFERENT specifier untouched — a genuine clash, not a redundant re-import", () => {
    const bindings = hostTopLevelBindings('import { Foo } from "mod-a";\n');
    const untouched = dedupeAgainstHost('import { Foo } from "mod-b";', bindings);
    expect(untouched).toBe('import { Foo } from "mod-b";');
  });

  it("leaves a non-import hoisted statement untouched", () => {
    const bindings = hostTopLevelBindings("");
    expect(dedupeAgainstHost("export function f(): void {}", bindings)).toBe("export function f(): void {}");
  });

  it("drops a name imported from the package's own public barrel when the host already imports that name from ANYWHERE — a self-package re-export, not a genuine clash", () => {
    const bindings = hostTopLevelBindings('import { WireDocument } from "./wire";\n');
    const result = dedupeAgainstHost('import { WireDocument } from "@shadowcat/core";', bindings, "@shadowcat/core");
    expect(result).toBeNull();
  });

  it("drops a name imported from the package's own public barrel when the host LOCALLY DECLARES that name — the symbol's own doc example importing itself", () => {
    const bindings = hostTopLevelBindings("export function resolveTokenActor(): void {}\n");
    const result = dedupeAgainstHost(
      'import { resolveTokenActor } from "@shadowcat/core";',
      bindings,
      "@shadowcat/core",
    );
    expect(result).toBeNull();
  });

  it("does NOT drop a VALUE import when the host only has the name as a type-only import — a type-only host binding cannot satisfy a value need", () => {
    const bindings = hostTopLevelBindings('import type { AssetResolver } from "mod";\n');
    const result = dedupeAgainstHost('import { AssetResolver } from "mod";', bindings);
    expect(result).toBe('import { AssetResolver } from "mod";');
  });

  it("does NOT drop a self-barrel VALUE import when the host only locally declares the name as a type-only interface", () => {
    const bindings = hostTopLevelBindings("interface AssetResolver {}\n");
    const result = dedupeAgainstHost(
      'import { AssetResolver } from "@shadowcat/core";',
      bindings,
      "@shadowcat/core",
    );
    expect(result).toBe('import { AssetResolver } from "@shadowcat/core";');
  });

  it("drops a type-only import when the host already has the name as a VALUE import — a value binding satisfies a type-only need", () => {
    const bindings = hostTopLevelBindings('import { AssetResolver } from "mod";\n');
    const result = dedupeAgainstHost('import { type AssetResolver } from "mod";', bindings);
    expect(result).toBeNull();
  });

  it("does not apply the self-barrel relaxation without a selfPackageName", () => {
    const bindings = hostTopLevelBindings('import { WireDocument } from "./wire";\n');
    const result = dedupeAgainstHost('import { WireDocument } from "@shadowcat/core";', bindings);
    expect(result).toBe('import { WireDocument } from "@shadowcat/core";');
  });

  it("drops a relative self-import of the host file's OWN local declaration — the file-scoped analogue of a package self-barrel import", () => {
    const hostFile = join("src", "client", "core", "src", "mock-server.ts");
    const bindings = hostTopLevelBindings("export class MockServer {}\n");
    const result = dedupeAgainstHost('import { MockServer } from "./mock-server";', bindings, null, hostFile);
    expect(result).toBeNull();
  });

  it("does not treat a relative import of a DIFFERENT sibling file as a self-reference", () => {
    const hostFile = join("src", "client", "core", "src", "mock-server.ts");
    const bindings = hostTopLevelBindings("export class MockServer {}\n");
    const result = dedupeAgainstHost('import { MockServer } from "./other-file";', bindings, null, hostFile);
    expect(result).toBe('import { MockServer } from "./other-file";');
  });
});

describe("hasTypeValueClash", () => {
  it("flags a value import needing a name the host only imports as a type", () => {
    const bindings = hostTopLevelBindings('import type { AssetResolver } from "mod";\n');
    expect(hasTypeValueClash(['import { AssetResolver } from "mod";'], bindings)).toBe(true);
  });

  it("flags a self-barrel value import needing a name the host only locally declares as an interface", () => {
    const bindings = hostTopLevelBindings("interface AssetResolver {}\n");
    expect(
      hasTypeValueClash(['import { AssetResolver } from "@shadowcat/core";'], bindings, "@shadowcat/core"),
    ).toBe(true);
  });

  it("does not flag a type-only import against a type-only host binding", () => {
    const bindings = hostTopLevelBindings('import type { AssetResolver } from "mod";\n');
    expect(hasTypeValueClash(['import { type AssetResolver } from "mod";'], bindings)).toBe(false);
  });

  it("does not flag an unrelated name", () => {
    const bindings = hostTopLevelBindings('import type { AssetResolver } from "mod";\n');
    expect(hasTypeValueClash(['import { Other } from "mod";'], bindings)).toBe(false);
  });
});

describe("upgradeTypeOnlyImports", () => {
  it("upgrades a whole-clause type-only import naming a single clashing binding", () => {
    const { text, upgraded } = upgradeTypeOnlyImports('import type { AssetResolver } from "mod";\n', new Set(["AssetResolver"]));
    expect(text).toBe('import { AssetResolver } from "mod";\n');
    expect(upgraded).toEqual(new Set(["AssetResolver"]));
  });

  it("splits a whole-clause type-only import, upgrading ONLY the clashing name and leaving the rest type-only", () => {
    const { text, upgraded } = upgradeTypeOnlyImports(
      'import type { ReadableDocuments, AssetResolver, WireDocument } from "@shadowcat/core";\n',
      new Set(["AssetResolver"]),
    );
    expect(upgraded).toEqual(new Set(["AssetResolver"]));
    expect(text).toContain('import { AssetResolver } from "@shadowcat/core";');
    expect(text).toContain('import type { ReadableDocuments, WireDocument } from "@shadowcat/core";');
    // The untouched names must still be type-only — never broadly upgraded.
    expect(text).not.toMatch(/import \{[^}]*ReadableDocuments/);
  });

  it("upgrades only the specific per-element `type` marker in a mixed named import", () => {
    const { text, upgraded } = upgradeTypeOnlyImports(
      'import { type Foo, Bar } from "mod";\n',
      new Set(["Foo"]),
    );
    expect(upgraded).toEqual(new Set(["Foo"]));
    expect(text).toContain("import { Foo, Bar }");
  });

  it("upgrades a type-only default import", () => {
    const { text, upgraded } = upgradeTypeOnlyImports('import type Foo from "mod";\n', new Set(["Foo"]));
    expect(upgraded).toEqual(new Set(["Foo"]));
    expect(text).toBe('import Foo from "mod";\n');
  });

  it("does not upgrade a name that names no import at all — reports it as unresolved via the empty `upgraded` set", () => {
    const { text, upgraded } = upgradeTypeOnlyImports("interface AssetResolver {}\n", new Set(["AssetResolver"]));
    expect(upgraded.size).toBe(0);
    expect(text).toBe("interface AssetResolver {}\n");
  });

  it("leaves the host's real text alone when no requested name is found", () => {
    const original = 'import type { Foo } from "mod";\n';
    const { text, upgraded } = upgradeTypeOnlyImports(original, new Set(["NotPresent"]));
    expect(text).toBe(original);
    expect(upgraded.size).toBe(0);
  });
});

describe("resolveTypeValueClashes", () => {
  it("upgrades the host's type-only import and marks the example's own hoisted import as redundant", () => {
    const hostText = 'import type { AssetResolver } from "mod";\nexport {};\n';
    const bindings = hostTopLevelBindings(hostText);
    const resolved = resolveTypeValueClashes(
      hostText,
      bindings,
      ['import { AssetResolver } from "mod";'],
      null,
      null,
    );
    expect(resolved.unresolved.size).toBe(0);
    expect(resolved.hostText).toContain('import { AssetResolver } from "mod";');
    expect(resolved.hostBindings.get("AssetResolver")).toEqual({ specifier: "mod", typeOnly: false });
  });

  it("returns the unresolved name when the host declares it locally, not via import", () => {
    const hostText = "interface AssetResolver {}\n";
    const bindings = hostTopLevelBindings(hostText);
    const resolved = resolveTypeValueClashes(
      hostText,
      bindings,
      ['import { AssetResolver } from "@shadowcat/core";'],
      "@shadowcat/core",
      null,
    );
    expect(resolved.unresolved).toEqual(new Set(["AssetResolver"]));
    expect(resolved.hostText).toBe(hostText);
  });

  it("is a no-op when there is no clash", () => {
    const hostText = 'import { Foo } from "mod";\nexport {};\n';
    const bindings = hostTopLevelBindings(hostText);
    const resolved = resolveTypeValueClashes(hostText, bindings, [], null, null);
    expect(resolved.hostText).toBe(hostText);
    expect(resolved.hostBindings).toBe(bindings);
    expect(resolved.unresolved.size).toBe(0);
  });
});

describe("type/value clash — virtual text shape after resolution", () => {
  it("emits exactly one AssetResolver binding — the upgraded host import — never a competing duplicate", () => {
    const hostText = 'import type { AssetResolver } from "./__doctest_control_helper__";\nexport {};\n';
    const analyzed = analyzeExample(
      'import { AssetResolver } from "./__doctest_control_helper__";\nconst v = new AssetResolver();\nconsole.log(v);',
    );
    const hostBindings = hostTopLevelBindings(hostText);
    const resolved = resolveTypeValueClashes(hostText, hostBindings, analyzed.hoisted, null, null);
    expect(resolved.unresolved.size).toBe(0);
    const text = buildVirtualText(resolved.hostText, analyzed, resolved.hostBindings);
    expect(text.match(/import \{ AssetResolver \}/g)).toHaveLength(1);
  });
});

describe("packageOwnName", () => {
  it("reads the package name from a workspace package's package.json", () => {
    const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
    expect(packageOwnName(repo, "src/client/core")).toBe("@shadowcat/core");
  });

  it("returns null for a dir with no package.json", () => {
    const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
    expect(packageOwnName(repo, "scripts")).toBeNull();
  });
});

describe("buildVirtualText", () => {
  it("does not re-emit an import the host already provides from the same module", () => {
    const hostText = 'import { Foo } from "mod";\nexport {};\n';
    const analyzed = analyzeExample('import { Foo } from "mod";\nconsole.log(Foo);');
    const text = buildVirtualText(hostText, analyzed);
    expect(text.match(/from "mod"/g)).toHaveLength(1);
  });
});

describe("findEnclosingClassTarget", () => {
  it("finds the class enclosing a documented instance member, by position not by name", () => {
    const text = [
      "class Foo {",
      "  /**",
      "   * @example",
      "   * ```ts",
      "   * this.bar();",
      "   * ```",
      "   */",
      "  bar(): void {}",
      "}",
    ].join("\n");
    const [ex] = extractExamples(text);
    const target = findEnclosingClassTarget(parse(text), ex.commentEnd);
    expect(target).not.toBeNull();
    expect(target.classNode.name.text).toBe("Foo");
    expect(target.isStatic).toBe(false);
  });

  it("resolves to the SECOND class in a multi-class file, not the first or nearest-by-scan", () => {
    const text = [
      "class First {",
      "  same(): void {}",
      "}",
      "",
      "class Second {",
      "  /**",
      "   * @example",
      "   * ```ts",
      "   * this.same();",
      "   * ```",
      "   */",
      "  same(): void {}",
      "}",
    ].join("\n");
    const [ex] = extractExamples(text);
    const target = findEnclosingClassTarget(parse(text), ex.commentEnd);
    expect(target.classNode.name.text).toBe("Second");
  });

  it("flags a documented static member as static", () => {
    const text = [
      "class Foo {",
      "  /**",
      "   * @example",
      "   * ```ts",
      "   * this.bar();",
      "   * ```",
      "   */",
      "  static bar(): void {}",
      "}",
    ].join("\n");
    const [ex] = extractExamples(text);
    const target = findEnclosingClassTarget(parse(text), ex.commentEnd);
    expect(target.isStatic).toBe(true);
  });

  it("returns null for a documented declaration with no enclosing class", () => {
    const text = [
      "/**",
      " * @example",
      " * ```ts",
      " * this.bar();",
      " * ```",
      " */",
      "export function bar(): void {}",
    ].join("\n");
    const [ex] = extractExamples(text);
    expect(findEnclosingClassTarget(parse(text), ex.commentEnd)).toBeNull();
  });

  it("targets the class itself when the comment documents the class, not a member", () => {
    const text = [
      "/**",
      " * @example",
      " * ```ts",
      " * this.bar();",
      " * ```",
      " */",
      "export class Foo {",
      "  bar(): void {}",
      "}",
    ].join("\n");
    const [ex] = extractExamples(text);
    const target = findEnclosingClassTarget(parse(text), ex.commentEnd);
    expect(target).not.toBeNull();
    expect(target.classNode.name.text).toBe("Foo");
  });
});

describe("pickSyntheticMethodName", () => {
  it("avoids an existing member name", () => {
    const text = "class Foo {\n  __docExample0(): void {}\n  __docExample1(): void {}\n}\n";
    const sourceFile = parse(text);
    const classNode = sourceFile.statements[0];
    expect(pickSyntheticMethodName(classNode)).toBe("__docExample2");
  });

  it("picks __docExample0 for a class with no synthetic members yet", () => {
    const sourceFile = parse("class Foo {\n  bar(): void {}\n}\n");
    const classNode = sourceFile.statements[0];
    expect(pickSyntheticMethodName(classNode)).toBe("__docExample0");
  });
});

describe("buildLineMap", () => {
  it("maps lines outside a marked region to increasing host line numbers, and marked lines to null", () => {
    const text = [
      "line0",
      "// __doc_example_body_start__",
      "injected0",
      "injected1",
      "// __doc_example_body_end__",
      "line1",
    ].join("\n");
    const map = buildLineMap(text);
    expect(map[0]).toBe(0);
    expect(map[1]).toBeNull();
    expect(map[2]).toBeNull();
    expect(map[3]).toBeNull();
    expect(map[4]).toBeNull();
    expect(map[5]).toBe(1);
  });
});

describe("buildClassInjectionText — end-to-end shape and compilation", () => {
  it("injects a static synthetic method for a documented static member", () => {
    const hostText = [
      "export class Foo {",
      "  /**",
      "   * @example",
      "   * ```ts",
      "   * this.bar();",
      "   * ```",
      "   */",
      "  static bar(): void {}",
      "}",
      "",
    ].join("\n");
    const [ex] = extractExamples(hostText);
    const analyzed = analyzeExample(ex.code);
    expect(analyzed.classContext).toBe(true);
    const target = findEnclosingClassTarget(parse(hostText), ex.commentEnd);
    const hostBindings = hostTopLevelBindings(hostText);
    const built = buildClassInjectionText(hostText, { ...ex, ...analyzed }, target, hostBindings);
    expect(built.text).toMatch(/static async __docExample0/);
  });

  it("injects an instance synthetic method for a documented instance member", () => {
    const hostText = [
      "export class Foo {",
      "  /**",
      "   * @example",
      "   * ```ts",
      "   * this.bar();",
      "   * ```",
      "   */",
      "  bar(): void {}",
      "}",
      "",
    ].join("\n");
    const [ex] = extractExamples(hostText);
    const analyzed = analyzeExample(ex.code);
    const target = findEnclosingClassTarget(parse(hostText), ex.commentEnd);
    const hostBindings = hostTopLevelBindings(hostText);
    const built = buildClassInjectionText(hostText, { ...ex, ...analyzed }, target, hostBindings);
    expect(built.text).toMatch(/(?<!static )async __docExample0/);
    expect(built.text).not.toMatch(/static async __docExample0/);
  });

  function compileInjected(hostText) {
    const [ex] = extractExamples(hostText);
    const analyzed = analyzeExample(ex.code);
    const target = findEnclosingClassTarget(parse(hostText), ex.commentEnd);
    const hostBindings = hostTopLevelBindings(hostText);
    const built = buildClassInjectionText(hostText, { ...ex, ...analyzed }, target, hostBindings);
    const options = {
      target: ts.ScriptTarget.ES2022,
      module: ts.ModuleKind.ESNext,
      moduleResolution: ts.ModuleResolutionKind.Bundler,
      strict: true,
      noEmit: true,
      skipLibCheck: true,
      ...EXAMPLE_HYGIENE_OVERRIDES,
    };
    const path = "/virtual/injected.ts";
    const host = ts.createCompilerHost(options, true);
    const base = { fileExists: host.fileExists.bind(host), getSourceFile: host.getSourceFile.bind(host) };
    host.fileExists = (f) => f === path || base.fileExists(f);
    host.getSourceFile = (f, lv, onErr, sc) =>
      f === path ? ts.createSourceFile(f, built.text, lv, true) : base.getSourceFile(f, lv, onErr, sc);
    const program = ts.createProgram([path], options, host);
    return ts.getPreEmitDiagnostics(program, program.getSourceFile(path));
  }

  it("compiles GREEN when the example calls a real private method", () => {
    const hostText = [
      "export class Foo {",
      "  /**",
      "   * @example",
      "   * ```ts",
      "   * this.bar();",
      "   * ```",
      "   */",
      "  useBar(): void {",
      "    this.bar();",
      "  }",
      "",
      "  private bar(): void {}",
      "}",
      "",
    ].join("\n");
    expect(compileInjected(hostText)).toHaveLength(0);
  });

  it("FAILS when the example calls a private member that does not exist", () => {
    const hostText = [
      "export class Foo {",
      "  /**",
      "   * @example",
      "   * ```ts",
      "   * this.doesNotExist();",
      "   * ```",
      "   */",
      "  useBar(): void {}",
      "}",
      "",
    ].join("\n");
    expect(compileInjected(hostText).length).toBeGreaterThan(0);
  });
});

describe("EXAMPLE_HYGIENE_OVERRIDES", () => {
  const baseOptions = {
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    strict: true,
    noEmit: true,
    skipLibCheck: true,
  };

  function compile(code) {
    const options = { ...baseOptions, ...EXAMPLE_HYGIENE_OVERRIDES };
    const path = "/virtual/hygiene-test.ts";
    const analyzed = analyzeExample(code);
    const text = buildVirtualText("export {};\n", analyzed);
    const host = ts.createCompilerHost(options, true);
    const base = { fileExists: host.fileExists.bind(host), getSourceFile: host.getSourceFile.bind(host) };
    host.fileExists = (f) => f === path || base.fileExists(f);
    host.getSourceFile = (f, lv, onErr, sc) =>
      f === path ? ts.createSourceFile(f, text, lv, true) : base.getSourceFile(f, lv, onErr, sc);
    const program = ts.createProgram([path], options, host);
    return ts.getPreEmitDiagnostics(program, program.getSourceFile(path));
  }

  it("turns off noUnusedLocals and noUnusedParameters, and nothing else", () => {
    expect(EXAMPLE_HYGIENE_OVERRIDES).toEqual({ noUnusedLocals: false, noUnusedParameters: false });
  });

  it("compiles GREEN an example whose only issue is an unused binding", () => {
    expect(compile("const unusedOnly = 1;\n")).toHaveLength(0);
  });

  it("still FAILS an example with a genuine type error", () => {
    expect(compile('const typeError: number = "not a number";\n').length).toBeGreaterThan(0);
  });
});

describe("workspacePaths", () => {
  it("maps every workspace package name to a posix-relative entry path", () => {
    const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
    const paths = workspacePaths(repo, workspacePackageDirs(repo));
    expect(paths["@shadowcat/core"]).toEqual(["src/client/core/src/index.ts"]);
    for (const v of Object.values(paths)) expect(v[0]).not.toContain("\\");
  });
});

describe("externalDepPaths", () => {
  it("maps an installed external dependency to its on-disk location, never a workspace package", () => {
    const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
    const paths = externalDepPaths(repo, workspacePackageDirs(repo));
    expect(Object.keys(paths).some((k) => k.startsWith("@shadowcat/"))).toBe(false);
  });
});

describe("candidateFiles / svelteFiles", () => {
  it("candidateFiles excludes *.test.ts and returns only .ts files", () => {
    const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
    const files = candidateFiles(repo, ["src/client/core"]);
    expect(files.length).toBeGreaterThan(0);
    for (const f of files) {
      expect(f.endsWith(".ts")).toBe(true);
      expect(f.endsWith(".test.ts")).toBe(false);
    }
  });

  it("svelteFiles returns only .svelte files", () => {
    const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
    const files = svelteFiles(repo, ["src/modules"]);
    for (const f of files) expect(f.endsWith(".svelte")).toBe(true);
  });
});
