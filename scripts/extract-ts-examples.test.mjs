import { describe, it, expect } from "vitest";
import { fileURLToPath } from "node:url";
import { resolve, join } from "node:path";
import { extractExamples, workspacePaths, workspacePackageDirs } from "./extract-ts-examples.mjs";

describe("extractExamples", () => {
  it("extracts a ts fence inside an @example tag with its line number", () => {
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
  });
  it("ignores svelte fences and fences outside @example", () => {
    const src = [
      "/**",
      " * @example",
      " * ```svelte",
      " * <Foo />",
      " * ```",
      " */",
      "/** ```ts",
      " * notAnExample();",
      " * ``` */",
    ].join("\n");
    expect(extractExamples(src)).toHaveLength(0);
  });
  it("extracts multiple examples from one file", () => {
    const one = "/**\n * @example\n * ```ts\n * a();\n * ```\n */\n";
    expect(extractExamples(one + one)).toHaveLength(2);
  });
});

describe("workspacePaths", () => {
  it("maps every workspace package name to a forward-slash entry path", () => {
    const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
    const outDir = join(repo, ".docs-tmp", "examples");
    const paths = workspacePaths(repo, outDir, workspacePackageDirs(repo));
    expect(paths["@shadowcat/core"]).toEqual(["../../src/client/core/src/index.ts"]);
    for (const v of Object.values(paths)) expect(v[0]).not.toContain("\\");
  });
});
