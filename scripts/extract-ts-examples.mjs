// Staleness gate for doc examples: every @example fence in a workspace .ts source is
// compiled inside a virtual sibling of its host module, so the example sees exactly the
// symbols its host module sees — including non-exported ones. .svelte sources carry
// @example blocks too but are never compiled here; their count is reported separately
// so the category stays visible instead of silently passing over it.
import { readdirSync, readFileSync } from "node:fs";
import { basename, dirname, extname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import ts from "typescript";

const toPosix = (p) => p.split(sep).join("/");

const DECLARATION_RE =
  /^(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(?:function\s*\*?|class|interface|type|enum|const|let|var)\s+([A-Za-z_$][\w$]*)/;
const MEMBER_RE =
  /^(?:public\s+|private\s+|protected\s+|static\s+|readonly\s+|async\s+|get\s+|set\s+|\*\s*)*([A-Za-z_$][\w$]*)\s*[(<:=]/;

/** Best-effort name of the declaration a doc comment documents, read from the text
 * immediately following the comment block. Falls back to a module-level marker when
 * nothing recognizable follows (e.g. a comment documenting the module itself). */
function symbolAfter(sourceText, index) {
  const rest = sourceText.slice(index).replace(/^\s+/, "");
  const declared = DECLARATION_RE.exec(rest);
  if (declared) return declared[1];
  const member = MEMBER_RE.exec(rest);
  if (member) return member[1];
  return "<module top-level>";
}

/** @example fences in one source text: every fence inside a `/** ... *\/` doc-comment
 * block tagged @example, tagged ` ```ts ` or untagged (fence tagging is not opt-in —
 * only a fence tagged for a DIFFERENT language, e.g. ` ```svelte `, is excluded). Each
 * entry carries the host symbol it documents and the example's 1-based ordinal within
 * that symbol, so a compile failure can be reported without a file:line citation. */
export function extractExamples(sourceText) {
  const out = [];
  const ordinals = new Map();
  for (const block of sourceText.matchAll(/\/\*\*[\s\S]*?\*\//g)) {
    const body = block[0];
    if (!/@example/.test(body)) continue;
    const offsetLine = sourceText.slice(0, block.index).split("\n").length;
    const symbol = symbolAfter(sourceText, block.index + body.length);
    // `\r?\n`: a CRLF working copy (Windows editor, or a tool writing without newline
    // translation disabled) otherwise matches nothing, silently dropping EVERY example in
    // that file while the gate still reports green — a false pass with no signal. Git
    // normalizes to LF on commit via .gitattributes, so this only bites local runs, which
    // is precisely where it is least likely to be noticed.
    for (const fence of body.matchAll(/```(?:ts)?\r?\n([\s\S]*?)```/g)) {
      const code = fence[1]
        .split("\n")
        .map((l) => l.replace(/^\s*\* ?/, ""))
        .join("\n")
        .trim();
      if (code === "") continue;
      const fenceLine = offsetLine + body.slice(0, fence.index).split("\n").length - 1;
      const ordinal = (ordinals.get(symbol) ?? 0) + 1;
      ordinals.set(symbol, ordinal);
      out.push({ code, line: fenceLine, symbol, ordinal });
    }
  }
  return out;
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

/** The workspace package dir (from `workspacePackageDirs`) that owns an absolute file
 * path, by longest-prefix match — needed because a package's own tsconfig.json is the
 * only source of the compiler options and module resolution its examples must be
 * checked under. Returns null for a file outside every workspace package. */
export function packageForFile(repoRoot, pkgDirs, absFile) {
  let best = null;
  for (const dir of pkgDirs) {
    const abs = resolve(repoRoot, dir);
    if (absFile === abs || absFile.startsWith(abs + sep)) {
      if (best === null || abs.length > resolve(repoRoot, best).length) best = dir;
    }
  }
  return best;
}

function walk(repoRoot, roots, predicate) {
  const files = [];
  const visit = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      if (e.name === "node_modules" || e.name === "dist" || e.name === "generated") continue;
      const p = join(d, e.name);
      if (e.isDirectory()) visit(p);
      else if (predicate(e.name)) files.push(p);
    }
  };
  for (const r of roots) {
    try { visit(join(repoRoot, r)); } catch { /* an optional root (examples/) may be absent */ }
  }
  return files;
}

/** All candidate .ts files under the given roots (skips node_modules/dist/generated,
 * and excludes *.test.ts — test files document nothing). */
export function candidateFiles(repoRoot, roots) {
  return walk(repoRoot, roots, (name) => name.endsWith(".ts") && !name.endsWith(".test.ts"));
}

/** All .svelte files under the given roots. Never compiled — @example blocks found here
 * are reported as an explicitly unchecked count, never silently passed over. */
export function svelteFiles(repoRoot, roots) {
  return walk(repoRoot, roots, (name) => name.endsWith(".svelte"));
}

/** A package's real compiler options, resolved via `ts.parseJsonConfigFileContent`
 * against that package's own tsconfig.json (which extends tsconfig.base.json) — so
 * examples are checked under the same `strict`, `noUnusedLocals` and
 * `verbatimModuleSyntax` settings, and the same `types`/module resolution, as the code
 * they document. */
function packageCompilerOptions(repoRoot, pkgDir) {
  const configPath = join(repoRoot, pkgDir, "tsconfig.json");
  const parseConfigHost = {
    useCaseSensitiveFileNames: ts.sys.useCaseSensitiveFileNames,
    readDirectory: ts.sys.readDirectory,
    fileExists: ts.sys.fileExists,
    readFile: ts.sys.readFile,
    getCurrentDirectory: () => join(repoRoot, pkgDir),
    onUnRecoverableConfigFileDiagnostic: (d) => {
      throw new Error(ts.flattenDiagnosticMessageText(d.messageText, "\n"));
    },
  };
  const parsed = ts.getParsedCommandLineOfConfigFile(configPath, {}, parseConfigHost);
  if (!parsed) throw new Error(`unable to parse ${configPath}`);
  return { ...parsed.options, noEmit: true };
}

/** A CompilerHost that overlays virtual files held only in memory on top of the real
 * filesystem, so an example never touches disk (`git status --short` stays empty even
 * on an interrupted run — there is nothing written to clean up). */
function createOverlayHost(options, overlay) {
  const host = ts.createCompilerHost(options, true);
  const baseFileExists = host.fileExists.bind(host);
  const baseReadFile = host.readFile.bind(host);
  const baseGetSourceFile = host.getSourceFile.bind(host);
  host.fileExists = (fileName) => overlay.has(fileName) || baseFileExists(fileName);
  host.readFile = (fileName) => (overlay.has(fileName) ? overlay.get(fileName) : baseReadFile(fileName));
  host.getSourceFile = (fileName, languageVersionOrOptions, onError, shouldCreateNewSourceFile) => {
    if (overlay.has(fileName)) {
      return ts.createSourceFile(fileName, overlay.get(fileName), languageVersionOrOptions, true);
    }
    return baseGetSourceFile(fileName, languageVersionOrOptions, onError, shouldCreateNewSourceFile);
  };
  return host;
}

const WRAPPER_OPEN = "async function __docExample(): Promise<void> {\n";
const WRAPPER_CLOSE = "\n}\nvoid __docExample();\n";

/** Compiles every extracted example for one package inside a virtual sibling of its
 * host module: the virtual file's text is the host module's full text, unchanged
 * (so its own relative imports keep resolving exactly as they do for the real host
 * file, and any diagnostic landing inside that portion carries the host's own line
 * numbers), followed by the example wrapped in an async function referenced once (so
 * `noUnusedLocals` stays satisfied) and never executed — this is compile-checked, the
 * TS analogue of a `no_run` doctest. Returns per-example pass/fail with host symbol,
 * ordinal and mapped diagnostics. */
function compilePackageExamples(repoRoot, pkgDir, examplesByFile) {
  const options = packageCompilerOptions(repoRoot, pkgDir);
  const overlay = new Map();
  const meta = new Map();
  for (const [hostFile, examples] of examplesByFile) {
    const hostText = readFileSync(hostFile, "utf8");
    const hostLineCount = hostText.split("\n").length;
    const dir = dirname(hostFile);
    const base = basename(hostFile, extname(hostFile));
    examples.forEach((ex, i) => {
      const virtualPath = toPosix(join(dir, `${base}.doctest${i}${extname(hostFile)}`));
      overlay.set(virtualPath, `${hostText}\n${WRAPPER_OPEN}${ex.code}${WRAPPER_CLOSE}`);
      meta.set(virtualPath, { hostFile, hostLineCount, ...ex });
    });
  }
  const host = createOverlayHost(options, overlay);
  const roots = [...overlay.keys()];
  const program = ts.createProgram(roots, options, host);
  const results = [];
  for (const virtualPath of roots) {
    const sourceFile = program.getSourceFile(virtualPath);
    const diagnostics = ts.getPreEmitDiagnostics(program, sourceFile);
    results.push({ ...meta.get(virtualPath), diagnostics, sourceFile });
  }
  return results;
}

/** Formats a diagnostic against the host module symbol and the example's ordinal
 * within that symbol, distinguishing a diagnostic inside the host's own copied text
 * (real host line — actionable against the actual source) from one inside the example
 * body itself. Never cites the virtual path alone. */
function formatFailure(result) {
  const lines = result.diagnostics.map((d) => {
    const message = ts.flattenDiagnosticMessageText(d.messageText, "\n");
    let where = "example body";
    if (d.file && d.start !== undefined) {
      const { line } = ts.getLineAndCharacterOfPosition(d.file, d.start);
      if (line < result.hostLineCount) where = `host line ${line + 1}`;
    }
    return `    [${where}] ${message}`;
  });
  return `  ${result.hostFile} :: ${result.symbol} :: example #${result.ordinal}\n${lines.join("\n")}`;
}

// --- Extraction controls -----------------------------------------------------------
// Proves the extraction predicate detects what it claims. Falsified once during
// development by deliberately breaking each behavior in turn and confirming the
// control failed before restoring it — an unfalsified control is decoration.
function runExtractionControls() {
  const checks = [
    ["tagged ```ts fence", () => {
      const got = extractExamples("/**\n * @example\n * ```ts\n * const controlTagged = 1;\n * ```\n */\n");
      return got.length === 1 && got[0].code.includes("controlTagged");
    }],
    ["untagged fence", () => {
      const got = extractExamples("/**\n * @example\n * ```\n * const controlUntagged = 1;\n * ```\n */\n");
      return got.length === 1 && got[0].code.includes("controlUntagged");
    }],
    ["fence calling a non-exported symbol", () => {
      const got = extractExamples(
        "/**\n * @example\n * ```ts\n * controlPrivateHelper();\n * ```\n */\nfunction controlPrivateHelper() {}\n",
      );
      return got.length === 1 && got[0].code.includes("controlPrivateHelper");
    }],
    ["CRLF-delimited fence", () => {
      const src = ["/**", " * @example", " * ```ts", " * const controlCrlf = 1;", " * ```", " */", ""].join("\r\n");
      const got = extractExamples(src);
      return got.length === 1 && got[0].code.includes("controlCrlf");
    }],
    ["fence inside a non-doc block comment must not be collected", () => {
      const got = extractExamples(
        "/* not a doc comment\n * @example\n * ```ts\n * controlNotCollected();\n * ```\n */\n",
      );
      return got.length === 0;
    }],
  ];
  const failed = checks.filter(([, run]) => !run()).map(([name]) => name);
  if (failed.length > 0) {
    console.error(`extraction control mismatch: ${failed.join(", ")}`);
    process.exit(1);
  }
}
runExtractionControls();

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
  const roots = ["src/types", "src/client", "src/modules", "examples"];
  const pkgDirs = workspacePackageDirs(repo);

  const svelteCount = svelteFiles(repo, roots).reduce(
    (sum, f) => sum + extractExamples(readFileSync(f, "utf8")).length,
    0,
  );

  const byPackage = new Map();
  let total = 0;
  for (const file of candidateFiles(repo, roots)) {
    const examples = extractExamples(readFileSync(file, "utf8"));
    if (examples.length === 0) continue;
    total += examples.length;
    const pkgDir = packageForFile(repo, pkgDirs, file);
    if (pkgDir === null) continue; // outside every workspace package: cannot resolve a tsconfig
    if (!byPackage.has(pkgDir)) byPackage.set(pkgDir, new Map());
    byPackage.get(pkgDir).set(file, examples);
  }

  console.log(
    `${svelteCount} @example blocks found in .svelte sources — unchecked (never compiled)`,
  );

  if (total === 0) {
    console.log("no @example ts blocks found — trivially green");
    process.exit(0);
  }

  const failures = [];
  for (const [pkgDir, examplesByFile] of byPackage) {
    for (const result of compilePackageExamples(repo, pkgDir, examplesByFile)) {
      if (result.diagnostics.length > 0) failures.push(result);
    }
  }

  if (failures.length > 0) {
    console.error(`${failures.length} of ${total} examples failed to compile:\n`);
    for (const f of failures) console.error(formatFailure(f));
    console.error(`\n${failures.length} of ${total} TS doc examples FAILED to typecheck`);
    process.exit(1);
  }
  console.log(`${total} TS doc examples typecheck OK`);
}
