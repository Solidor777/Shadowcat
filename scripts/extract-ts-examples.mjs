// Staleness gate for TS doc examples: every @example ```ts fence in workspace
// sources is extracted to .docs-tmp/examples/ and typechecked (compile-checked,
// not executed — the TS analogue of `no_run` doctests). ```svelte and untagged
// fences are ignored by design.
import { mkdirSync, readdirSync, readFileSync, writeFileSync, copyFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import process from "node:process";

/** @example ```ts fences in one source text, with the fence's 1-based line. */
export function extractExamples(sourceText) {
  const out = [];
  for (const block of sourceText.matchAll(/\/\*\*[\s\S]*?\*\//g)) {
    const body = block[0];
    if (!/@example/.test(body)) continue;
    const offsetLine = sourceText.slice(0, block.index).split("\n").length;
    for (const fence of body.matchAll(/```ts\n([\s\S]*?)```/g)) {
      const code = fence[1]
        .split("\n")
        .map((l) => l.replace(/^\s*\* ?/, ""))
        .join("\n")
        .trim();
      const fenceLine = offsetLine + body.slice(0, fence.index).split("\n").length - 1;
      if (code !== "") out.push({ code, line: fenceLine });
    }
  }
  return out;
}

/** All candidate .ts files under the given roots (skips node_modules/dist/tests/generated). */
export function candidateFiles(repoRoot, roots) {
  const files = [];
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      if (e.name === "node_modules" || e.name === "dist" || e.name === "generated") continue;
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.name.endsWith(".ts") && !e.name.endsWith(".test.ts")) files.push(p);
    }
  };
  for (const r of roots) {
    const abs = join(repoRoot, r);
    try { walk(abs); } catch { /* an optional root (examples/) may be absent */ }
  }
  return files;
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
  const outDir = join(repo, ".docs-tmp", "examples");
  mkdirSync(outDir, { recursive: true });
  const files = candidateFiles(repo, ["src/types", "src/client", "src/modules", "examples"]);
  const index = [];
  let n = 0;
  for (const f of files) {
    for (const ex of extractExamples(readFileSync(f, "utf8"))) {
      const name = `ex${String(n++).padStart(4, "0")}.ts`;
      writeFileSync(join(outDir, name), `// source: ${relative(repo, f)}:${ex.line}\nexport {};\n${ex.code}\n`);
      index.push({ name, source: `${relative(repo, f)}:${ex.line}` });
    }
  }
  copyFileSync(join(repo, "scripts", "ts-examples-tsconfig.template.json"), join(outDir, "tsconfig.json"));
  if (index.length === 0) { console.log("no @example ts blocks found — trivially green"); process.exit(0); }
  const pnpm = process.platform === "win32" ? "pnpm.cmd" : "pnpm";
  const res = spawnSync(pnpm, ["exec", "tsc", "-p", outDir], { stdio: "inherit", shell: process.platform === "win32" });
  if (res.status !== 0) {
    console.error(`example typecheck FAILED — map exNNNN.ts to sources via the header comment in each file (${index.length} examples)`);
    process.exit(res.status ?? 1);
  }
  console.log(`${index.length} TS doc examples typecheck OK`);
}
