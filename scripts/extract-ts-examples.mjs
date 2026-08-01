// Staleness gate for TS doc examples: every @example ```ts fence in workspace
// sources is extracted to .docs-tmp/examples/ and typechecked (compile-checked,
// not executed — the TS analogue of `no_run` doctests). ```svelte and untagged
// fences are ignored by design.
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
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
    // `\r?\n`: a CRLF working copy (Windows editor, or a tool writing without newline
    // translation disabled) otherwise matches nothing, silently dropping EVERY example in
    // that file while the gate still reports green — a false pass with no signal. Git
    // normalizes to LF on commit via .gitattributes, so this only bites local runs, which
    // is precisely where it is least likely to be noticed.
    for (const fence of body.matchAll(/```ts\r?\n([\s\S]*?)```/g)) {
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

/** Maps every workspace package name to its TS entry (package.json `main`, else
 * exports["."], else src/index.ts) so extracted examples can import ANY workspace
 * package by name — examples must be self-contained (import what they use). */
export function workspacePaths(repoRoot, outDir, pkgDirs) {
  const paths = {};
  for (const dir of pkgDirs) {
    const pkgFile = join(repoRoot, dir, "package.json");
    let pkg;
    try { pkg = JSON.parse(readFileSync(pkgFile, "utf8")); } catch { continue; }
    const entry = pkg.main ?? (typeof pkg.exports?.["."] === "string" ? pkg.exports["."] : "src/index.ts");
    const abs = join(repoRoot, dir, entry);
    paths[pkg.name] = [relative(outDir, abs).split("\\").join("/")];
  }
  return paths;
}

/** Maps each workspace package's EXTERNAL dependencies to their on-disk location under that
 * package's own `node_modules`. pnpm installs a dependency into the dependent package's
 * `node_modules`, not the workspace root, so an extracted example living in `.docs-tmp/examples/`
 * resolves nothing by walking up — without these entries a `@example` that imports a third-party
 * type (`pixi.js` for `@shadowcat/render`) fails with "Cannot find module", which would push a
 * whole package's examples onto untagged fences and silently drop them from typechecking.
 *
 * A name declared by two packages resolves to whichever is visited last; acceptable because this
 * tsconfig only ever compiles doc examples, never shipped code. Only deps that actually exist on
 * disk are mapped, so a pruned/optional install degrades to the previous "unresolvable" behavior
 * rather than a broken path. */
export function externalDepPaths(repoRoot, outDir, pkgDirs) {
  const paths = {};
  for (const dir of pkgDirs) {
    let pkg;
    try { pkg = JSON.parse(readFileSync(join(repoRoot, dir, "package.json"), "utf8")); } catch { continue; }
    for (const dep of Object.keys(pkg.dependencies ?? {})) {
      if (dep.startsWith("@shadowcat/")) continue; // workspace packages: mapped to source by workspacePaths
      const abs = join(repoRoot, dir, "node_modules", dep);
      if (!existsSync(abs)) continue;
      paths[dep] = [relative(outDir, abs).split("\\").join("/")];
    }
  }
  return paths;
}

/** Direct child package dirs under the workspace roots (mirrors pnpm-workspace.yaml). */
export function workspacePackageDirs(repoRoot) {
  const dirs = ["src/types"];
  for (const parent of ["src/client", "src/modules", "examples"]) {
    try {
      for (const e of readdirSync(join(repoRoot, parent), { withFileTypes: true })) {
        if (e.isDirectory()) dirs.push(`${parent}/${e.name}`);
      }
    } catch { /* an optional root (examples/) may be absent */ }
  }
  return dirs;
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
  // Purge stale examples first. The generated tsconfig compiles the whole directory
  // (`include: ["*.ts"]`), so an exNNNN.ts left by an earlier run is still typechecked:
  // a deleted @example keeps reporting as covered, and one referencing a since-renamed
  // symbol fails the gate pointing at a file no current source doc produces. Example
  // numbering is positional, so any content change also reshuffles which file holds what.
  rmSync(outDir, { recursive: true, force: true });
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
  // Workspace sources pulled in via paths import .svelte files; the scratch
  // program has no svelte ambient types, so a default-export shim stands in
  // (component types are irrelevant to example typechecking). Typed `any`,
  // not `unknown`: a workspace package whose real source INTERNALLY consumes
  // one of its own .svelte imports as a value (e.g. passing it to Svelte's
  // `mount()`) needs the shim to be freely assignable everywhere a real
  // component type would be — `unknown` fails that the moment such a call
  // site is pulled into the compiled graph by an unrelated example importing
  // that package by name, even though the example itself never touches the
  // component.
  writeFileSync(
    join(outDir, "_svelte-shim.d.ts"),
    'declare module "*.svelte" {\n  const component: any;\n  export default component;\n}\n',
  );
  const template = JSON.parse(readFileSync(join(repo, "scripts", "ts-examples-tsconfig.template.json"), "utf8"));
  const pkgDirs = workspacePackageDirs(repo);
  template.compilerOptions.paths = {
    // External deps first: a workspace package of the same name must win, since
    // examples should typecheck against workspace SOURCE, not an installed copy.
    ...template.compilerOptions.paths,
    ...externalDepPaths(repo, outDir, pkgDirs),
    ...workspacePaths(repo, outDir, pkgDirs),
  };
  writeFileSync(join(outDir, "tsconfig.json"), JSON.stringify(template, null, 2) + "\n");
  if (index.length === 0) { console.log("no @example ts blocks found — trivially green"); process.exit(0); }
  // tsc's JS entry runs under the current node — no shell, no PATH lookup.
  const tsc = join(repo, "node_modules", "typescript", "bin", "tsc");
  const res = spawnSync(process.execPath, [tsc, "-p", outDir], { stdio: "inherit" });
  if (res.status !== 0) {
    console.error(`example typecheck FAILED — map exNNNN.ts to sources via the header comment in each file (${index.length} examples)`);
    process.exit(res.status ?? 1);
  }
  console.log(`${index.length} TS doc examples typecheck OK`);
}
