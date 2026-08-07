// Staleness gate for doc examples: every @example fence in a workspace .ts source is
// compiled inside a virtual sibling of its host module, so the example sees exactly the
// symbols its host module sees — including non-exported ones. .svelte sources, and
// examples that reference an enclosing class's `this`/private members, carry @example
// blocks too but are never compiled here; each such category's count is reported
// separately so it stays visible instead of silently passing over it.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { basename, dirname, extname, join, relative, resolve, sep } from "node:path";
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

/** Maps every workspace package name to its TS entry (package.json `main`, else
 * exports["."], else src/index.ts), relative to the repo root — pnpm does not hoist a
 * workspace package into its own `node_modules`, so an example inside `@shadowcat/core`
 * that imports `@shadowcat/core` by name has no other way to resolve. Also covers a
 * cross-package example importing a workspace package its host does not depend on. */
export function workspacePaths(repoRoot, pkgDirs) {
  const paths = {};
  for (const dir of pkgDirs) {
    const pkgFile = join(repoRoot, dir, "package.json");
    let pkg;
    try { pkg = JSON.parse(readFileSync(pkgFile, "utf8")); } catch { continue; }
    const entry = pkg.main ?? (typeof pkg.exports?.["."] === "string" ? pkg.exports["."] : "src/index.ts");
    paths[pkg.name] = [toPosix(relative(repoRoot, join(repoRoot, dir, entry)))];
  }
  return paths;
}

/** The npm package name a workspace package dir declares for itself, or null when its
 * package.json is missing or unparsable. */
export function packageOwnName(repoRoot, pkgDir) {
  try {
    return JSON.parse(readFileSync(join(repoRoot, pkgDir, "package.json"), "utf8")).name ?? null;
  } catch {
    return null;
  }
}

/** Maps each workspace package's EXTERNAL dependencies to their on-disk location under
 * that package's own `node_modules`, relative to the repo root. pnpm installs a
 * dependency into the DEPENDENT package's `node_modules`, not the workspace root, so an
 * example whose host package does not itself declare a given dependency (a cross-package
 * example, or a `types`-only dependency used just for the doc) would otherwise fail to
 * resolve it even though some workspace package has it on disk. A name declared by two
 * packages resolves to whichever is visited last; acceptable because this mapping only
 * ever feeds doc-example compilation, never shipped code. */
export function externalDepPaths(repoRoot, pkgDirs) {
  const paths = {};
  for (const dir of pkgDirs) {
    let pkg;
    try { pkg = JSON.parse(readFileSync(join(repoRoot, dir, "package.json"), "utf8")); } catch { continue; }
    for (const dep of Object.keys(pkg.dependencies ?? {})) {
      if (dep.startsWith("@shadowcat/")) continue; // workspace packages: mapped to source by workspacePaths
      const abs = join(repoRoot, dir, "node_modules", dep);
      if (!existsSync(abs)) continue;
      paths[dep] = [toPosix(relative(repoRoot, abs))];
    }
  }
  return paths;
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

function isHoistable(statement) {
  if (
    ts.isImportDeclaration(statement) ||
    ts.isImportEqualsDeclaration(statement) ||
    ts.isExportDeclaration(statement) ||
    ts.isExportAssignment(statement)
  ) {
    return true;
  }
  if (ts.canHaveModifiers(statement)) {
    const modifiers = ts.getModifiers(statement);
    // `export`: the modifier itself is illegal inside a function body.
    // `declare`: an ambient declaration (`declare const store: ReadableDocuments;`, the
    // idiom an example uses to introduce a typed placeholder without a real value) is
    // only legal at the top level of a module, never inside a function body either.
    if (
      modifiers?.some((m) => m.kind === ts.SyntaxKind.ExportKeyword || m.kind === ts.SyntaxKind.DeclareKeyword)
    ) {
      return true;
    }
  }
  return false;
}

function establishesOwnThis(node) {
  return (
    ts.isClassDeclaration(node) ||
    ts.isClassExpression(node) ||
    ts.isFunctionDeclaration(node) ||
    ts.isFunctionExpression(node) ||
    ts.isMethodDeclaration(node) ||
    ts.isConstructorDeclaration(node) ||
    ts.isGetAccessorDeclaration(node) ||
    ts.isSetAccessorDeclaration(node)
  );
}

/** True when a statement references `this` or a `#private` member at a point that
 * would resolve OUTSIDE any class/function the statement itself declares — the shape of
 * an example lifted verbatim from inside a class method, which cannot type-check once
 * wrapped in a free function (arrow functions do not establish their own `this`, so a
 * `this` inside a nested arrow function still counts). */
function referencesEnclosingThisOrPrivate(statement) {
  let found = false;
  const visit = (node, ownThis) => {
    if (found) return;
    if (!ownThis && (node.kind === ts.SyntaxKind.ThisKeyword || ts.isPrivateIdentifier(node))) {
      found = true;
      return;
    }
    ts.forEachChild(node, (child) => visit(child, ownThis || establishesOwnThis(node)));
  };
  visit(statement, false);
  return found;
}

/** Splits an example's own top-level statements into the ones that must be hoisted out
 * of a wrapping function (import/export declarations, and any declaration carrying an
 * `export` modifier — none of these are legal inside a function body) and the rest, and
 * flags whether the example references an enclosing class's `this`/private members (a
 * shape no wrapper, hoisted or not, can type-check without injecting it into the host
 * class body — reported as an unchecked category instead of compiled). */
export function analyzeExample(code) {
  const sourceFile = ts.createSourceFile("example.ts", code, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const hoisted = [];
  const rest = [];
  let classContext = false;
  for (const statement of sourceFile.statements) {
    if (referencesEnclosingThisOrPrivate(statement)) classContext = true;
    const text = statement.getText(sourceFile);
    if (isHoistable(statement)) hoisted.push(text);
    else rest.push(text);
  }
  return { hoisted, rest, classContext };
}

const printer = ts.createPrinter();

/** The specifier a module import names, as the string VALUE it resolves to (not the
 * raw quoted text) — `'x'` and `"x"` name the same module, and comparing text would
 * treat them as different. */
function specifierValue(moduleSpecifier, sourceFile) {
  return ts.isStringLiteral(moduleSpecifier) ? moduleSpecifier.text : moduleSpecifier.getText(sourceFile);
}

const LOCAL_SPECIFIER = Symbol("host-local-declaration");

/** Every name a host module's own source binds at its top level, keyed by that name and
 * valued by `{ specifier, typeOnly }`: `specifier` is the module-specifier VALUE an
 * import binding came from, or the `LOCAL_SPECIFIER` sentinel for a locally declared
 * function/class/interface/type/enum/const (it has no specifier); `typeOnly` is true
 * when the binding exists only in the type space (an `import type`/per-element `type`
 * import, or a local `interface`/`type` alias) and so cannot satisfy an example that
 * needs the name as a VALUE (e.g. to construct it). A hoisted statement shares the
 * host's own top-level scope, so:
 *
 * - An example that re-imports a name the host already imports from the IDENTICAL
 *   specifier is not introducing anything new — a duplicate top-level binding, an
 *   artifact of sharing that scope rather than a defect in the example.
 * - An example importing the symbol it documents FROM THE PACKAGE'S OWN PUBLIC NAME
 *   (`import { resolveTokenActor } from "@shadowcat/core"` written inside
 *   `resolveTokenActor`'s own host module) collides with that symbol's LOCAL
 *   declaration, not another import — the same collision, needing the same treatment
 *   (see `dedupeAgainstHost`'s self-barrel relaxation).
 *
 * A name imported from any other different specifier is left alone: a genuine clash
 * the example would need to resolve by aliasing. */
export function hostTopLevelBindings(hostText) {
  const hostSourceFile = ts.createSourceFile("host.ts", hostText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const bindings = new Map();
  for (const statement of hostSourceFile.statements) {
    if (ts.isImportDeclaration(statement) && statement.importClause) {
      const specifier = specifierValue(statement.moduleSpecifier, hostSourceFile);
      const clause = statement.importClause;
      if (clause.name) bindings.set(clause.name.text, { specifier, typeOnly: clause.isTypeOnly });
      if (clause.namedBindings) {
        if (ts.isNamespaceImport(clause.namedBindings)) {
          bindings.set(clause.namedBindings.name.text, { specifier, typeOnly: clause.isTypeOnly });
        } else if (ts.isNamedImports(clause.namedBindings)) {
          for (const element of clause.namedBindings.elements) {
            bindings.set(element.name.text, { specifier, typeOnly: clause.isTypeOnly || element.isTypeOnly });
          }
        }
      }
    } else if (ts.isInterfaceDeclaration(statement) || ts.isTypeAliasDeclaration(statement)) {
      bindings.set(statement.name.text, { specifier: LOCAL_SPECIFIER, typeOnly: true });
    } else if (
      ts.isFunctionDeclaration(statement) ||
      ts.isClassDeclaration(statement) ||
      ts.isEnumDeclaration(statement)
    ) {
      if (statement.name) bindings.set(statement.name.text, { specifier: LOCAL_SPECIFIER, typeOnly: false });
    } else if (ts.isVariableStatement(statement)) {
      for (const decl of statement.declarationList.declarations) {
        if (ts.isIdentifier(decl.name)) bindings.set(decl.name.text, { specifier: LOCAL_SPECIFIER, typeOnly: false });
      }
    }
  }
  return bindings;
}

/** Narrows or drops a hoisted import statement to the bindings it introduces that the
 * host module does not already provide. Returns null when every binding the statement
 * introduces is already redundant, meaning the whole statement can be dropped.
 * Statements other than import declarations pass through unchanged (there is no
 * host-side dedupe target for them).
 *
 * A binding counts as redundant when the host binds the identical name from the
 * identical specifier (or — when `selfPackageName` is given and the hoisted import's
 * specifier IS that package's own public name — from ANYWHERE, per
 * `hostTopLevelBindings`), AND the host's binding is at least as permissive as what
 * this element needs: a host binding that is type-only can satisfy an example element
 * that is ALSO type-only, but never one that needs the name as a value (e.g. to
 * `new` it) — dropping a value import because the host only has the TYPE of the same
 * name would leave the example with no value binding at all. Such an element instead
 * classifies as `"clash"` (see `hasTypeValueClash`): the host and the example need two
 * incompatible bindings under one name, which no import statement can provide at once
 * (see `hasTypeValueClash`'s own doc for why this is left uncompiled rather than
 * reported as a compile failure). */
function classifyBinding(name, elementTypeOnly, hostBindings, specifier, isSelfBarrel) {
  const hostBinding = hostBindings.get(name);
  if (!hostBinding) return "keep";
  if (!isSelfBarrel && hostBinding.specifier !== specifier) return "keep";
  if (hostBinding.typeOnly && !elementTypeOnly) return "clash";
  return "redundant";
}

/** True when `specifier`, written inside `hostFile`, names `hostFile` itself — a
 * relative self-import (`import { MockServer } from "./mock-server"` written inside
 * `mock-server.ts`), the file-scoped analogue of importing a package by its own public
 * name: the host's local declarations ARE what such a specifier resolves to. */
function isSelfFileReference(hostFile, specifier) {
  if (hostFile === null || !specifier.startsWith(".")) return false;
  const withoutExt = (p) => p.slice(0, p.length - extname(p).length);
  return withoutExt(resolve(dirname(hostFile), specifier)) === withoutExt(resolve(hostFile));
}

export function dedupeAgainstHost(hoistedText, hostBindings, selfPackageName = null, hostFile = null) {
  const sourceFile = ts.createSourceFile("hoisted.ts", hoistedText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const statement = sourceFile.statements[0];
  if (!statement || !ts.isImportDeclaration(statement) || !statement.importClause) return hoistedText;
  const specifier = specifierValue(statement.moduleSpecifier, sourceFile);
  const isSelfBarrel =
    (selfPackageName !== null && specifier === selfPackageName) || isSelfFileReference(hostFile, specifier);
  const isRedundant = (name, elementTypeOnly) =>
    classifyBinding(name, elementTypeOnly, hostBindings, specifier, isSelfBarrel) === "redundant";
  const clause = statement.importClause;
  let name = clause.name;
  if (name && isRedundant(name.text, clause.isTypeOnly)) name = undefined;
  let namedBindings = clause.namedBindings;
  if (namedBindings && ts.isNamespaceImport(namedBindings)) {
    if (isRedundant(namedBindings.name.text, clause.isTypeOnly)) namedBindings = undefined;
  } else if (namedBindings && ts.isNamedImports(namedBindings)) {
    const kept = namedBindings.elements.filter(
      (el) => !isRedundant(el.name.text, clause.isTypeOnly || el.isTypeOnly),
    );
    namedBindings = kept.length > 0 ? ts.factory.updateNamedImports(namedBindings, kept) : undefined;
  }
  if (!name && !namedBindings) return null;
  if (name === clause.name && namedBindings === clause.namedBindings) return hoistedText;
  const newClause = ts.factory.updateImportClause(clause, clause.isTypeOnly, name, namedBindings);
  const newStatement = ts.factory.updateImportDeclaration(
    statement,
    statement.modifiers,
    newClause,
    statement.moduleSpecifier,
    statement.attributes,
  );
  return printer.printNode(ts.EmitHint.Unspecified, newStatement, sourceFile);
}

/** True when hoisting would leave an import needing a name as a VALUE (e.g. to `new`
 * it) alongside the host's own pre-existing TYPE-ONLY import of the identical name —
 * the shape of a doc example that imports the class it documents from the package's
 * public surface while the host module itself only ever needs that class's TYPE (a
 * `verbatimModuleSyntax` codebase writes exactly this: `import type { X }` where a
 * value is never touched). No import statement can bind the same top-level name twice
 * in one module regardless of type-only-ness — TypeScript rejects `import type { X }`
 * and `import { X }` (or an aliased `import { X as Y }`) from the same specifier
 * coexisting as a duplicate identifier — so this is not a compile FAILURE the example
 * or the host did anything wrong to cause; it is the point at which "compile inside
 * the host's own scope" and "an example may exercise the value half of a name the host
 * only uses as a type" become mutually exclusive. Reported as its own unchecked
 * category instead of a cryptic duplicate-identifier diagnostic. */
export function hasTypeValueClash(hoisted, hostBindings, selfPackageName = null, hostFile = null) {
  return hoisted.some((hoistedText) => {
    const sourceFile = ts.createSourceFile("hoisted.ts", hoistedText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
    const statement = sourceFile.statements[0];
    if (!statement || !ts.isImportDeclaration(statement) || !statement.importClause) return false;
    const specifier = specifierValue(statement.moduleSpecifier, sourceFile);
    const isSelfBarrel =
      (selfPackageName !== null && specifier === selfPackageName) || isSelfFileReference(hostFile, specifier);
    const clashes = (name, elementTypeOnly) =>
      classifyBinding(name, elementTypeOnly, hostBindings, specifier, isSelfBarrel) === "clash";
    const clause = statement.importClause;
    if (clause.name && clashes(clause.name.text, clause.isTypeOnly)) return true;
    if (clause.namedBindings) {
      if (ts.isNamespaceImport(clause.namedBindings)) return clashes(clause.namedBindings.name.text, clause.isTypeOnly);
      if (ts.isNamedImports(clause.namedBindings)) {
        return clause.namedBindings.elements.some((el) => clashes(el.name.text, clause.isTypeOnly || el.isTypeOnly));
      }
    }
    return false;
  });
}

const WRAPPER_OPEN = "async function __docExample(): Promise<void> {\n";
const WRAPPER_CLOSE = "\n}\nvoid __docExample();\n";

/** The full virtual-file text for one example: the host module's full text, unchanged
 * (so its own relative imports keep resolving exactly as they do for the real host
 * file, and a diagnostic landing inside that portion carries the host's own line
 * numbers), followed by the example's own hoisted import/export/declare statements —
 * each import first deduped against the host's own top-level imports — at the virtual
 * file's top level, followed by its remaining statements wrapped in an async function
 * referenced once (so `noUnusedLocals` stays satisfied) and never executed — this is
 * compile-checked, the TS analogue of a `no_run` doctest. */
export function buildVirtualText(
  hostText,
  analyzed,
  hostBindings = hostTopLevelBindings(hostText),
  selfPackageName = null,
  hostFile = null,
) {
  const hoisted = analyzed.hoisted
    .map((h) => dedupeAgainstHost(h, hostBindings, selfPackageName, hostFile))
    .filter((h) => h !== null);
  const hoistedBlock = hoisted.length > 0 ? `${hoisted.join("\n")}\n` : "";
  return `${hostText}\n${hoistedBlock}${WRAPPER_OPEN}${analyzed.rest.join("\n")}${WRAPPER_CLOSE}`;
}

/** A package's real compiler options, resolved via `ts.getParsedCommandLineOfConfigFile`
 * against that package's own tsconfig.json (which extends tsconfig.base.json) — so
 * examples are checked under the same `strict`, `noUnusedLocals` and
 * `verbatimModuleSyntax` settings, and the same `types`, as the code they document.
 * `baseUrl`/`paths` are added on top so a workspace-package-name import resolves even
 * when the host package does not itself declare that package as a dependency
 * (including a package importing itself by name). */
function packageCompilerOptions(repoRoot, pkgDir, pkgDirs) {
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
  return {
    ...parsed.options,
    noEmit: true,
    baseUrl: toPosix(repoRoot),
    paths: {
      ...(parsed.options.paths ?? {}),
      ...externalDepPaths(repoRoot, pkgDirs),
      // Workspace source wins over an installed copy of the same name.
      ...workspacePaths(repoRoot, pkgDirs),
    },
  };
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

/** Compiles a set of virtual files (already built by `buildVirtualText`) against real
 * compiler options and a real (overlaid) filesystem, and returns each root's own
 * diagnostics — never the whole program's, so one example's failure never appears
 * charged against a sibling. */
function compileOverlay(options, overlay) {
  const host = createOverlayHost(options, overlay);
  const roots = [...overlay.keys()];
  const program = ts.createProgram(roots, options, host);
  return new Map(
    roots.map((virtualPath) => {
      const sourceFile = program.getSourceFile(virtualPath);
      return [virtualPath, ts.getPreEmitDiagnostics(program, sourceFile)];
    }),
  );
}

/** Compiles every compilable extracted example for one package (an example flagged
 * `classContext` by `analyzeExample` is never passed in here — see the unchecked-count
 * reporting in `main`). Returns per-example pass/fail with host symbol, ordinal and
 * mapped diagnostics. */
function compilePackageExamples(repoRoot, pkgDir, pkgDirs, examplesByFile) {
  const options = packageCompilerOptions(repoRoot, pkgDir, pkgDirs);
  const selfPackageName = packageOwnName(repoRoot, pkgDir);
  const overlay = new Map();
  const meta = new Map();
  for (const [hostFile, examples] of examplesByFile) {
    const hostText = readFileSync(hostFile, "utf8");
    const hostLineCount = hostText.split("\n").length;
    const dir = dirname(hostFile);
    const base = basename(hostFile, extname(hostFile));
    const hostBindings = hostTopLevelBindings(hostText);
    examples.forEach((ex, i) => {
      const virtualPath = toPosix(join(dir, `${base}.doctest${i}${extname(hostFile)}`));
      overlay.set(virtualPath, buildVirtualText(hostText, ex, hostBindings, selfPackageName, hostFile));
      meta.set(virtualPath, { hostFile, hostLineCount, ...ex });
    });
  }
  const diagnosticsByPath = compileOverlay(options, overlay);
  return [...diagnosticsByPath.entries()].map(([virtualPath, diagnostics]) => ({
    ...meta.get(virtualPath),
    diagnostics,
  }));
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

// --- Controls ------------------------------------------------------------------------
// Prove the extraction predicate detects what it claims, AND that the compilation
// pipeline itself accepts a genuinely self-contained example and correctly routes a
// class-context example away from compilation — a control that only re-tests
// extraction cannot catch a defect in hoisting, path resolution, or classification.
// Falsified once during development by deliberately breaking each behavior in turn and
// confirming the control failed before restoring it — an unfalsified control is
// decoration.
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

/** A self-contained example with a top-level import — the shape a hoist defect
 * regresses across the whole tagged-fence surface — must compile GREEN through the
 * real hoist + overlay + compile pipeline. Uses two virtual files placed inside a real
 * directory (mirroring production: a virtual file always lives beside a real host
 * file), never written to disk. */
function runCompilationControl(repoRoot) {
  const dir = join(repoRoot, "scripts");
  const helperPath = toPosix(join(dir, "__doctest_control_helper__.ts"));
  const hostPath = toPosix(join(dir, "__doctest_control_host__.doctest0.ts"));
  const analyzed = analyzeExample(
    'import { controlHelperValue } from "./__doctest_control_helper__";\n' +
      "const v: number = controlHelperValue();\nconsole.log(v);",
  );
  const options = {
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    strict: true,
    noUnusedLocals: true,
    noEmit: true,
    skipLibCheck: true,
  };
  const overlay = new Map([
    [helperPath, "export function controlHelperValue(): number {\n  return 1;\n}\n"],
    [hostPath, buildVirtualText("export {};\n", analyzed)],
  ]);
  const diagnostics = compileOverlay(options, overlay).get(hostPath);
  if (diagnostics.length !== 0) {
    console.error(
      `compilation control mismatch: self-contained top-level-import example did not compile green:\n` +
        diagnostics.map((d) => `  ${ts.flattenDiagnosticMessageText(d.messageText, "\n")}`).join("\n"),
    );
    process.exit(1);
  }
}

/** A `this.`-style example (documenting a class method) must be classified as needing
 * class context, never compiled as a free function and never silently dropped. */
function runClassContextControl() {
  const { classContext } = analyzeExample("this.doThing();\nreturn this.value;");
  if (!classContext) {
    console.error("compilation control mismatch: a this-referencing example was not routed to the class-context category");
    process.exit(1);
  }
  const selfContained = analyzeExample("class Local {\n  m() { return this.x; }\n}\nnew Local().m();");
  if (selfContained.classContext) {
    console.error("compilation control mismatch: a self-contained class example was misclassified as needing class context");
    process.exit(1);
  }
}

const repoRootForControls = resolve(fileURLToPath(import.meta.url), "..", "..");
runExtractionControls();
runCompilationControl(repoRootForControls);
runClassContextControl();

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const repo = repoRootForControls;
  const roots = ["src/types", "src/client", "src/modules", "examples"];
  const pkgDirs = workspacePackageDirs(repo);

  const svelteCount = svelteFiles(repo, roots).reduce(
    (sum, f) => sum + extractExamples(readFileSync(f, "utf8")).length,
    0,
  );

  const byPackage = new Map();
  let classContextCount = 0;
  let typeValueClashCount = 0;
  let compileTotal = 0;
  for (const file of candidateFiles(repo, roots)) {
    const hostText = readFileSync(file, "utf8");
    const examples = extractExamples(hostText);
    if (examples.length === 0) continue;
    const pkgDir = packageForFile(repo, pkgDirs, file);
    const selfPackageName = pkgDir === null ? null : packageOwnName(repo, pkgDir);
    const hostBindings = hostTopLevelBindings(hostText);
    for (const ex of examples) {
      const analyzed = analyzeExample(ex.code);
      if (analyzed.classContext) {
        classContextCount += 1;
        continue;
      }
      if (pkgDir === null) continue; // outside every workspace package: cannot resolve a tsconfig
      if (hasTypeValueClash(analyzed.hoisted, hostBindings, selfPackageName, file)) {
        typeValueClashCount += 1;
        continue;
      }
      compileTotal += 1;
      if (!byPackage.has(pkgDir)) byPackage.set(pkgDir, new Map());
      const forFile = byPackage.get(pkgDir);
      if (!forFile.has(file)) forFile.set(file, []);
      forFile.get(file).push({ ...ex, ...analyzed });
    }
  }

  console.log(`${svelteCount} @example blocks found in .svelte sources — unchecked (never compiled)`);
  console.log(
    `${classContextCount} @example blocks reference an enclosing class's this/private members — ` +
      `unchecked (not compiled; would need injection into the host class body)`,
  );
  console.log(
    `${typeValueClashCount} @example blocks need a name as a VALUE that the host module only ever imports ` +
      `as a TYPE — unchecked (no import statement can bind the same name twice in one module)`,
  );

  if (compileTotal === 0) {
    console.log("no compilable @example ts blocks found — trivially green");
    process.exit(0);
  }

  const failures = [];
  for (const [pkgDir, examplesByFile] of byPackage) {
    for (const result of compilePackageExamples(repo, pkgDir, pkgDirs, examplesByFile)) {
      if (result.diagnostics.length > 0) failures.push(result);
    }
  }

  if (failures.length > 0) {
    console.error(`${failures.length} of ${compileTotal} compilable examples failed to compile:\n`);
    for (const f of failures) console.error(formatFailure(f));
    console.error(`\n${failures.length} of ${compileTotal} TS doc examples FAILED to typecheck`);
    process.exit(1);
  }
  console.log(`${compileTotal} TS doc examples typecheck OK`);
}
