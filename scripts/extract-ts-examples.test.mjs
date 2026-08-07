import { describe, it, expect } from "vitest";
import { fileURLToPath } from "node:url";
import { resolve, join } from "node:path";
import {
  extractExamples,
  workspacePackageDirs,
  candidateFiles,
  svelteFiles,
  packageForFile,
} from "./extract-ts-examples.mjs";

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
