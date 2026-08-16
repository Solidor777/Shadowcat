// Staleness gate for doc examples: every @example fence in a workspace .ts source is
// compiled inside a virtual sibling of its host module, so the example sees exactly the
// symbols its host module sees — including non-exported ones. An example referencing an
// enclosing class's `this`/private members is instead injected as a synthetic method
// into the class actually enclosing the documented symbol. .svelte sources carry
// @example blocks too but are never compiled here; that count, and any example this
// gate genuinely cannot resolve (no import to upgrade, no enclosing class), is reported
// separately and individually so nothing passes over in silence.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { basename, dirname, extname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import ts from "typescript";
import { isDirectEntry } from "./lib/is-main.mjs";

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
 * that symbol, so a compile failure can be reported without a file:line citation, plus
 * `commentEnd` — the character offset right after the doc comment, the same anchor
 * `symbolAfter` reads from, reused by `findEnclosingClassTarget` to locate the actual
 * AST declaration (and its enclosing class, if any) the comment documents. */
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
      out.push({ code, line: fenceLine, symbol, ordinal, commentEnd: block.index + body.length });
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
 * shape no plain top-level wrapper can type-check — see `findEnclosingClassTarget` and
 * `buildClassInjectionText` for how such an example is instead compiled as a synthetic
 * method injected into the host class body). */
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

/** Every name across `hoisted` that classifies as `"clash"` against `hostBindings` —
 * needed as a VALUE (e.g. to `new` it) while the host only provides it as a TYPE. See
 * `resolveTypeValueClashes` for why this is resolved by upgrading the host's own
 * import rather than left unresolved. */
function clashingNames(hoisted, hostBindings, selfPackageName, hostFile) {
  const names = new Set();
  for (const hoistedText of hoisted) {
    const sourceFile = ts.createSourceFile("hoisted.ts", hoistedText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
    const statement = sourceFile.statements[0];
    if (!statement || !ts.isImportDeclaration(statement) || !statement.importClause) continue;
    const specifier = specifierValue(statement.moduleSpecifier, sourceFile);
    const isSelfBarrel =
      (selfPackageName !== null && specifier === selfPackageName) || isSelfFileReference(hostFile, specifier);
    const check = (name, elementTypeOnly) => {
      if (classifyBinding(name, elementTypeOnly, hostBindings, specifier, isSelfBarrel) === "clash") names.add(name);
    };
    const clause = statement.importClause;
    if (clause.name) check(clause.name.text, clause.isTypeOnly);
    if (clause.namedBindings) {
      if (ts.isNamespaceImport(clause.namedBindings)) {
        check(clause.namedBindings.name.text, clause.isTypeOnly);
      } else if (ts.isNamedImports(clause.namedBindings)) {
        for (const el of clause.namedBindings.elements) check(el.name.text, clause.isTypeOnly || el.isTypeOnly);
      }
    }
  }
  return names;
}

/** True when hoisting would leave an import needing a name as a VALUE (e.g. to `new`
 * it) alongside the host's own pre-existing TYPE-ONLY import of the identical name —
 * the shape of a doc example that imports the class it documents from the package's
 * public surface while the host module itself only ever needs that class's TYPE (a
 * `verbatimModuleSyntax` codebase writes exactly this: `import type { X }` where a
 * value is never touched). See `resolveTypeValueClashes` for how this is resolved. */
export function hasTypeValueClash(hoisted, hostBindings, selfPackageName = null, hostFile = null) {
  return clashingNames(hoisted, hostBindings, selfPackageName, hostFile).size > 0;
}

/** Rewrites the SPECIFIC import statements in `hostText` that bind a name in
 * `namesToUpgrade` from type-only to a value import — narrowly, leaving every other
 * name in the same (or any other) import declaration untouched. A whole-clause
 * `import type { A, B }` where only `A` clashes is split into `import { A } from …`
 * plus `import type { B } from …`, so `B` stays exactly as type-only as it was. Safe
 * ONLY because the merged virtual file is typechecked, never emitted: the
 * `verbatimModuleSyntax` rule that makes `import type` meaningful for real output
 * governs emitted JS, which this file never produces. The host's real file on disk is
 * never touched — this operates on an in-memory copy of its text.
 *
 * Returns the rewritten text and the subset of `namesToUpgrade` actually resolved.
 * A name is left out of `upgraded` when it names something other than an import (a
 * local `interface`/`type` alias has no import statement to upgrade — its type-only
 * status is intrinsic, not a `verbatimModuleSyntax` restriction) or when it appears in
 * an import clause shape this function does not recognize; the caller reports any such
 * name individually rather than silently treating it as resolved. */
export function upgradeTypeOnlyImports(hostText, namesToUpgrade) {
  const upgraded = new Set();
  if (namesToUpgrade.size === 0) return { text: hostText, upgraded };
  const sourceFile = ts.createSourceFile("host.ts", hostText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const replacements = [];
  for (const statement of sourceFile.statements) {
    if (!ts.isImportDeclaration(statement) || !statement.importClause) continue;
    const clause = statement.importClause;
    const specifier = statement.moduleSpecifier;
    const rewritten = [];

    if (clause.name && !clause.namedBindings && clause.isTypeOnly && namesToUpgrade.has(clause.name.text)) {
      upgraded.add(clause.name.text);
      const newClause = ts.factory.updateImportClause(clause, false, clause.name, undefined);
      rewritten.push(ts.factory.updateImportDeclaration(statement, statement.modifiers, newClause, specifier, statement.attributes));
    } else if (clause.namedBindings && ts.isNamespaceImport(clause.namedBindings)) {
      const ns = clause.namedBindings;
      if (clause.isTypeOnly && namesToUpgrade.has(ns.name.text)) {
        upgraded.add(ns.name.text);
        const newClause = ts.factory.updateImportClause(clause, false, clause.name, ns);
        rewritten.push(ts.factory.updateImportDeclaration(statement, statement.modifiers, newClause, specifier, statement.attributes));
      }
    } else if (clause.namedBindings && ts.isNamedImports(clause.namedBindings)) {
      const elements = clause.namedBindings.elements;
      if (clause.isTypeOnly) {
        const toUpgrade = elements.filter((el) => namesToUpgrade.has(el.name.text));
        const toKeep = elements.filter((el) => !namesToUpgrade.has(el.name.text));
        if (toUpgrade.length > 0) {
          for (const el of toUpgrade) upgraded.add(el.name.text);
          rewritten.push(
            ts.factory.createImportDeclaration(
              statement.modifiers,
              ts.factory.createImportClause(false, undefined, ts.factory.createNamedImports(toUpgrade)),
              specifier,
              statement.attributes,
            ),
          );
          if (toKeep.length > 0) {
            rewritten.push(
              ts.factory.createImportDeclaration(
                statement.modifiers,
                ts.factory.createImportClause(true, undefined, ts.factory.createNamedImports(toKeep)),
                specifier,
                statement.attributes,
              ),
            );
          }
        }
      } else {
        let anyElementChanged = false;
        const newElements = elements.map((el) => {
          if (el.isTypeOnly && namesToUpgrade.has(el.name.text)) {
            anyElementChanged = true;
            upgraded.add(el.name.text);
            return ts.factory.updateImportSpecifier(el, false, el.propertyName, el.name);
          }
          return el;
        });
        if (anyElementChanged) {
          const newNamedBindings = ts.factory.updateNamedImports(clause.namedBindings, newElements);
          const newClause = ts.factory.updateImportClause(clause, clause.isTypeOnly, clause.name, newNamedBindings);
          rewritten.push(
            ts.factory.updateImportDeclaration(statement, statement.modifiers, newClause, specifier, statement.attributes),
          );
        }
      }
    }

    if (rewritten.length === 0) continue;
    const printed = rewritten.map((s) => printer.printNode(ts.EmitHint.Unspecified, s, sourceFile)).join("\n");
    replacements.push({ start: statement.getStart(sourceFile), end: statement.getEnd(), text: printed });
  }
  if (replacements.length === 0) return { text: hostText, upgraded };
  replacements.sort((a, b) => b.start - a.start);
  let text = hostText;
  for (const r of replacements) text = text.slice(0, r.start) + r.text + text.slice(r.end);
  return { text, upgraded };
}

/** Resolves every type/value clash between `hoisted` and `hostBindings` by upgrading
 * the host's own clashing import(s) to a value import, narrowly, via
 * `upgradeTypeOnlyImports`, and reflecting the upgrade in a COPY of `hostBindings` (the
 * upgraded names now read `typeOnly: false`) so a subsequent `dedupeAgainstHost` pass
 * correctly treats the example's own now-redundant import as droppable. A name that
 * resists the upgrade (see `upgradeTypeOnlyImports`) is returned in `unresolved` for
 * the caller to report individually — never silently compiled as if nothing were
 * wrong, and never silently dropped into an uninspected bucket. Returns the ORIGINAL
 * `hostText`/`hostBindings` unchanged when there is nothing to resolve. */
export function resolveTypeValueClashes(hostText, hostBindings, hoisted, selfPackageName = null, hostFile = null) {
  const clashes = clashingNames(hoisted, hostBindings, selfPackageName, hostFile);
  if (clashes.size === 0) return { hostText, hostBindings, unresolved: new Set() };
  const { text, upgraded } = upgradeTypeOnlyImports(hostText, clashes);
  const unresolved = new Set([...clashes].filter((name) => !upgraded.has(name)));
  if (upgraded.size === 0) return { hostText, hostBindings, unresolved };
  const newBindings = new Map(hostBindings);
  for (const name of upgraded) {
    const existing = newBindings.get(name);
    newBindings.set(name, { specifier: existing.specifier, typeOnly: false });
  }
  return { hostText: text, hostBindings: newBindings, unresolved };
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

// --- Class-context injection ---------------------------------------------------------
// An example that references an enclosing class's `this`/private members cannot
// type-check as a free function: `this` has no type outside a method, and a
// `#private` reference is a syntax error outside a class body. Such an example is
// instead injected as a synthetic method inside the class that actually encloses the
// documented symbol.

/** Marker lines wrapping every stretch of text that came from the EXAMPLE (the
 * injected method body, and the hoisted import block) rather than from the host's own
 * source, so `buildLineMap` can attribute a diagnostic's line to "host line N" or
 * "example body" by scanning the FINAL text for these markers — correct regardless of
 * how many lines an earlier transformation (splitting a type-only import, upgrading
 * one to a value import) added or removed before the marked region, which a
 * fixed-offset line count could not track. */
const EXAMPLE_BODY_START = "// __doc_example_body_start__";
const EXAMPLE_BODY_END = "// __doc_example_body_end__";

/** Maps each 0-based line index of `text` to the 0-based line number it corresponds to
 * in the ORIGINAL host file, or `null` when the line is inside an
 * `EXAMPLE_BODY_START`/`EXAMPLE_BODY_END` marked region (including the marker lines
 * themselves). Lines outside any marked region appear in `text` in the same relative
 * order as in the original host file (splicing only ever inserts, never reorders or
 * deletes), so a running counter over just those lines reconstructs real 1-based host
 * line numbers. */
export function buildLineMap(text) {
  const lines = text.split("\n");
  const map = new Array(lines.length).fill(null);
  let hostLine = 0;
  let inBody = false;
  for (let i = 0; i < lines.length; i++) {
    const trimmed = lines[i].trim();
    if (trimmed === EXAMPLE_BODY_START) {
      inBody = true;
      continue;
    }
    if (trimmed === EXAMPLE_BODY_END) {
      inBody = false;
      continue;
    }
    if (inBody) continue;
    map[i] = hostLine;
    hostLine += 1;
  }
  return map;
}

/** A diagnostic-line formatter backed by a simple threshold — used by the free-function
 * wrapper path, where the host's own text is never edited mid-file, so every line at or
 * past a fixed count belongs to the example. `translate` converts a 0-based line index
 * WITHIN `hostText` to the 0-based line it corresponds to in the file the citation
 * should name — identity for a `.ts` host (`hostText` IS that file verbatim), or
 * `extractSvelteHost`'s `toSvelteLine` for a `.svelte` host, where `hostText` is only
 * the extracted script content and a further indirection is needed to cite the real
 * SFC line. A `translate` returning `null` (a line `extractSvelteHost` cannot relate
 * back, e.g. a synthetic block separator) falls back to "example body" rather than
 * mis-citing a line. */
function hostLineMapper(hostLineCount, translate = (n) => n) {
  return (line) => {
    if (line >= hostLineCount) return "example body";
    const real = translate(line);
    return real === null ? "example body" : `host line ${real + 1}`;
  };
}

/** A diagnostic-line formatter backed by `buildLineMap` — used by the class-injection
 * path, where the example's method is spliced into the MIDDLE of the host's text, so a
 * single threshold cannot distinguish host lines that follow the injection point from
 * the injected content itself. See `hostLineMapper` for `translate`. */
function arrayLineMapper(lineMap, translate = (n) => n) {
  return (line) => {
    const hostLine = lineMap[line];
    if (hostLine === null || hostLine === undefined) return "example body";
    const real = translate(hostLine);
    return real === null ? "example body" : `host line ${real + 1}`;
  };
}

function establishesOwnStatic(node) {
  return (
    ts.isMethodDeclaration(node) ||
    ts.isPropertyDeclaration(node) ||
    ts.isGetAccessorDeclaration(node) ||
    ts.isSetAccessorDeclaration(node) ||
    ts.isConstructorDeclaration(node)
  );
}

/** The class enclosing the declaration a doc comment documents, found by AST position
 * rather than by name — the same "next real token after the comment" anchor
 * `symbolAfter` approximates with a regex, but exact: the node anywhere in the file
 * with the SMALLEST start position at or after `commentEnd` is the declaration the
 * comment leads (a class body member if the comment sits inside a class, the class
 * declaration itself if the comment documents the class as a whole). Returns null when
 * that declaration is not inside any class — a plain top-level function whose example
 * happens to reference `this` has no injection target, and none is guessed. `isStatic`
 * is true when the documented member itself carries the `static` modifier, so the
 * caller injects a static synthetic member and `this` resolves against the correct
 * (static vs instance) side. */
export function findEnclosingClassTarget(sourceFile, commentEnd) {
  let best = null;
  const visit = (node) => {
    const start = node.getStart(sourceFile);
    if (start >= commentEnd && (best === null || start < best.getStart(sourceFile))) {
      best = node;
    }
    ts.forEachChild(node, visit);
  };
  ts.forEachChild(sourceFile, visit);
  if (best === null) return null;
  let node = best;
  let isStatic = false;
  if (establishesOwnStatic(node)) {
    if (ts.canHaveModifiers(node)) {
      isStatic = (ts.getModifiers(node) ?? []).some((m) => m.kind === ts.SyntaxKind.StaticKeyword);
    }
    node = node.parent;
  }
  if (ts.isClassDeclaration(node) && node.name) return { classNode: node, isStatic };
  return null;
}

/** A synthetic method name guaranteed not to collide with any existing member
 * (instance or static, public or private) of `classNode` — the example's own local
 * bindings live inside that method's own function scope, where a same-named class
 * member is never ambiguous with a local variable (a class member is only ever
 * reachable via `this.name`/`ClassName.name`, never as a bare identifier), so only the
 * synthetic method's OWN name needs a collision check. */
export function pickSyntheticMethodName(classNode) {
  const existingNames = new Set();
  for (const member of classNode.members) {
    if (member.name && (ts.isIdentifier(member.name) || ts.isPrivateIdentifier(member.name))) {
      existingNames.add(member.name.text);
    }
  }
  let i = 0;
  while (existingNames.has(`__docExample${i}`)) i += 1;
  return `__docExample${i}`;
}

/** The full virtual-file text for a class-context example: the host module's full
 * text, spliced to insert a synthetic method — `static` exactly when the documented
 * member is `static`, so `this` resolves against the correct side — as the last member
 * of `target.classNode`, body-only from `ex.rest` (its own hoisted import/export/
 * declare statements cannot live inside a class body; they are appended at module top
 * level afterward, exactly as `buildVirtualText` does). Splicing happens BEFORE
 * resolving type/value clashes and deduping hoisted imports so their character offsets
 * are computed against the ALREADY-spliced text, never invalidated by a length change
 * introduced earlier in the file. Every stretch of example-authored text is wrapped in
 * `EXAMPLE_BODY_START`/`EXAMPLE_BODY_END` markers so the returned `lineMap` (see
 * `buildLineMap`) can attribute each diagnostic correctly. */
export function buildClassInjectionText(hostText, ex, target, hostBindings, selfPackageName = null, hostFile = null) {
  const { classNode, isStatic } = target;
  const insertPos = classNode.end - 1;
  const methodName = pickSyntheticMethodName(classNode);
  const methodText =
    `\n  ${EXAMPLE_BODY_START}\n` +
    `  ${isStatic ? "static " : ""}async ${methodName}(): Promise<void> {\n${ex.rest.join("\n")}\n  }\n` +
    `  ${EXAMPLE_BODY_END}\n`;
  const splicedHostText = hostText.slice(0, insertPos) + methodText + hostText.slice(insertPos);

  const resolved = resolveTypeValueClashes(splicedHostText, hostBindings, ex.hoisted, selfPackageName, hostFile);
  const hoisted = ex.hoisted
    .map((h) => dedupeAgainstHost(h, resolved.hostBindings, selfPackageName, hostFile))
    .filter((h) => h !== null);
  const hoistedBlock =
    hoisted.length > 0 ? `\n${EXAMPLE_BODY_START}\n${hoisted.join("\n")}\n${EXAMPLE_BODY_END}\n` : "";

  const text = `${resolved.hostText}${hoistedBlock}`;
  return { text, unresolved: resolved.unresolved, lineMap: buildLineMap(text) };
}

/** The example-compilation option contract, applied on top of whatever a package's own
 * `tsconfig.json` resolves to. **Type correctness is enforced; production-hygiene
 * lints are not.** An example is documentation, not shipped code: a binding kept only
 * to name what a call returns (`const asset = await uploadAsset(...);`) is the correct
 * shape for a doc example, not a defect, and `noUnusedLocals`/`noUnusedParameters`
 * would report it as one purely because the host package happens to enable that lint
 * for its own production hygiene. Inheriting a package's options WHOLESALE silently
 * turns that production-hygiene policy into an example-correctness verdict with
 * nothing marking the transition — this block is the marker: any future option that
 * governs style/hygiene rather than whether an example actually works belongs here,
 * named, rather than being rediscovered the next time it inflates a failure count. */
export const EXAMPLE_HYGIENE_OVERRIDES = {
  noUnusedLocals: false,
  noUnusedParameters: false,
};

/** A package's real compiler options, resolved via `ts.getParsedCommandLineOfConfigFile`
 * against that package's own tsconfig.json (which extends tsconfig.base.json), so
 * examples are checked under the same `strict` and `verbatimModuleSyntax` settings,
 * and the same `types`, as the code they document — with `EXAMPLE_HYGIENE_OVERRIDES`
 * layered on top to keep production-hygiene lints from becoming example-correctness
 * verdicts. `baseUrl`/`paths` are added on top so a workspace-package-name import
 * resolves even when the host package does not itself declare that package as a
 * dependency (including a package importing itself by name). */
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
    ...EXAMPLE_HYGIENE_OVERRIDES,
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

/** Compiles every compilable free-function-wrapper example for one package (a
 * `classContext`-flagged example is never passed in here — see
 * `compilePackageClassInjectedExamples`). Returns per-example pass/fail with host
 * symbol, ordinal and mapped diagnostics. */
function compilePackageExamples(repoRoot, pkgDir, pkgDirs, examplesByFile) {
  const options = packageCompilerOptions(repoRoot, pkgDir, pkgDirs);
  const selfPackageName = packageOwnName(repoRoot, pkgDir);
  const overlay = new Map();
  const meta = new Map();
  for (const [hostFile, examples] of examplesByFile) {
    const hostText = readFileSync(hostFile, "utf8");
    const dir = dirname(hostFile);
    const base = basename(hostFile, extname(hostFile));
    const hostBindings = hostTopLevelBindings(hostText);
    examples.forEach((ex, i) => {
      const virtualPath = toPosix(join(dir, `${base}.doctest${i}${extname(hostFile)}`));
      // Each example gets its OWN resolution: a clash-driven upgrade is scoped to the
      // names one example's hoisted imports actually need, never shared across the
      // other examples of the same host file.
      const resolved = resolveTypeValueClashes(hostText, hostBindings, ex.hoisted, selfPackageName, hostFile);
      const effectiveHostLineCount = resolved.hostText.split("\n").length;
      overlay.set(virtualPath, buildVirtualText(resolved.hostText, ex, resolved.hostBindings, selfPackageName, hostFile));
      meta.set(virtualPath, { hostFile, mapLine: hostLineMapper(effectiveHostLineCount), ...ex });
    });
  }
  const diagnosticsByPath = compileOverlay(options, overlay);
  return [...diagnosticsByPath.entries()].map(([virtualPath, diagnostics]) => ({
    ...meta.get(virtualPath),
    diagnostics,
  }));
}

/** Compiles every class-context example for one package by injecting each as a
 * synthetic method via `buildClassInjectionText`. The enclosing-class target is
 * re-derived here from a fresh parse of the CURRENT host text, never carried over from
 * an earlier parse in `main`'s classification pass, so the AST node driving the splice
 * is always consistent with the text it is spliced into. */
function compilePackageClassInjectedExamples(repoRoot, pkgDir, pkgDirs, examplesByFile) {
  const options = packageCompilerOptions(repoRoot, pkgDir, pkgDirs);
  const selfPackageName = packageOwnName(repoRoot, pkgDir);
  const overlay = new Map();
  const meta = new Map();
  for (const [hostFile, examples] of examplesByFile) {
    const hostText = readFileSync(hostFile, "utf8");
    const dir = dirname(hostFile);
    const base = basename(hostFile, extname(hostFile));
    const hostBindings = hostTopLevelBindings(hostText);
    const hostSourceFile = ts.createSourceFile(hostFile, hostText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
    examples.forEach((ex, i) => {
      const target = findEnclosingClassTarget(hostSourceFile, ex.commentEnd);
      if (target === null) return; // pre-filtered by `main`; defensive only
      const virtualPath = toPosix(join(dir, `${base}.doctest${i}.inject${extname(hostFile)}`));
      const built = buildClassInjectionText(hostText, ex, target, hostBindings, selfPackageName, hostFile);
      overlay.set(virtualPath, built.text);
      meta.set(virtualPath, { hostFile, mapLine: arrayLineMapper(built.lineMap), ...ex });
    });
  }
  const diagnosticsByPath = compileOverlay(options, overlay);
  return [...diagnosticsByPath.entries()].map(([virtualPath, diagnostics]) => ({
    ...meta.get(virtualPath),
    diagnostics,
  }));
}

// --- Svelte SFC hosts ----------------------------------------------------------------
// A `.svelte` file's `@example` blocks are documented inside its `<script lang="ts">`
// content, which is not itself a standalone TS module: `svelte2tsx` is what normally
// gives that content a real component type and is bundled inside `svelte-check`, not
// resolvable as a package from this workspace. What IS available is the extracted
// script text itself (valid TS on its own — Svelte's runes are declared as ambient
// globals by `node_modules/svelte/types/index.d.ts`, pulled in automatically once a
// package's own tsconfig sets `"types": ["svelte"]`, exactly as it already does for
// every package that owns a `.svelte` example-bearing file — no extra file-set wiring
// needed on top of `packageCompilerOptions`, reused unchanged. The same ambient file
// also declares `declare module '*.svelte'` itself, so a host's own
// `import Foo from "./Foo.svelte"` resolves for free too — no shim of this script's
// own is needed to keep that import from producing a host-attributable "cannot find
// module" diagnostic (verified: compiling a host with such an import, with and
// without a hand-written shim, produces the identical zero diagnostics). Everything downstream
// of `hostText` (`hostTopLevelBindings`, `dedupeAgainstHost`, `resolveTypeValueClashes`,
// `buildVirtualText`, `buildClassInjectionText`, `compileOverlay`) is reused unchanged —
// this section only supplies a different HOST, not a second compiler.

const SCRIPT_TAG_RE = /<script([^>]*)>([\s\S]*?)<\/script>/g;

const BIND_THIS_RE = /bind:this\s*=\s*\{([^}]*)\}/g;

/** Every `bind:this={expr}` TEMPLATE target in `svelteText` (a match inside a
 * `<script>` block is not a real template binding and is excluded) that names a bare
 * identifier — the one shape `markBindThisAssigned` can act on. A member or
 * element-access target (`bind:this={refs.foo}`, `bind:this={itemEls[i]}`) is excluded
 * by design, not merely unhandled: Svelte can only assign INTO an already-existing
 * object/array through such a target, so the script must already initialize the base
 * identifier (`refs`, `itemEls`) with a value wherever it is declared — that
 * declaration already carries an initializer, and TypeScript's definite-assignment
 * analysis was never confused about it. Only a bare identifier can be the sole,
 * uninitialized declaration a template-only assignment leaves TypeScript unable to
 * see. `bind:this` on a component instance (not a DOM element) reduces to the same
 * bare-identifier shape and needs no separate handling. */
export function extractBindThisSimpleIdentifiers(svelteText) {
  const scriptRanges = [];
  for (const m of svelteText.matchAll(SCRIPT_TAG_RE)) scriptRanges.push([m.index, m.index + m[0].length]);
  const names = new Set();
  for (const m of svelteText.matchAll(BIND_THIS_RE)) {
    if (scriptRanges.some(([s, e]) => m.index >= s && m.index < e)) continue;
    const expr = m[1].trim();
    if (/^[A-Za-z_$][\w$]*$/.test(expr)) names.add(expr);
  }
  return names;
}

/** Rewrites every top-level-visible `let`/`var` declaration in `hostText` whose name
 * is in `names` and which has neither an initializer nor an existing definite-
 * assignment assertion, adding TypeScript's `!` definite-assignment assertion — the
 * same feature used for a class field a FRAMEWORK assigns through a mechanism the
 * compiler cannot see (e.g. Angular `@ViewChild`), applied here because Svelte's
 * template (`bind:this`) assigns these variables at runtime through a mechanism
 * script-only extraction never observes. The assertion preserves the declared TYPE
 * exactly — it is not a widening to `any`, so a genuine type error against that
 * variable is still caught. A name with an initializer (already definitely assigned)
 * is left untouched; a name with no type annotation (`!` requires one) is left
 * untouched and reported in `unmarked` rather than silently ignored. Returns the
 * ordered list of raw text `replacements` actually applied (ascending by `start`, in
 * the ORIGINAL `hostText`'s coordinates) so a caller with offsets into that same
 * original text (`extractSvelteHost`'s `toHostOffset`) can keep them valid against the
 * edited text via `remapOffsetAfterEdits`. */
export function markBindThisAssigned(hostText, names) {
  const marked = new Set();
  const unmarked = new Set();
  if (names.size === 0) return { text: hostText, marked, unmarked, replacements: [] };
  const sourceFile = ts.createSourceFile("host.ts", hostText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const found = [];
  const visit = (node) => {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && names.has(node.name.text)) {
      found.push(node);
    }
    ts.forEachChild(node, visit);
  };
  ts.forEachChild(sourceFile, visit);
  const replacements = [];
  for (const node of found) {
    if (node.initializer !== undefined || node.exclamationToken !== undefined) continue;
    if (node.type === undefined) {
      unmarked.add(node.name.text);
      continue;
    }
    const updated = ts.factory.updateVariableDeclaration(
      node,
      node.name,
      ts.factory.createToken(ts.SyntaxKind.ExclamationToken),
      node.type,
      node.initializer,
    );
    const printed = printer.printNode(ts.EmitHint.Unspecified, updated, sourceFile);
    replacements.push({ start: node.getStart(sourceFile), end: node.getEnd(), text: printed });
    marked.add(node.name.text);
  }
  replacements.sort((a, b) => a.start - b.start);
  let text = hostText;
  for (const r of [...replacements].reverse()) text = text.slice(0, r.start) + r.text + text.slice(r.end);
  return { text, marked, unmarked, replacements };
}

/** Maps a character offset in the text BEFORE `replacements` to the corresponding
 * offset in the text AFTER `replacements` were applied — `replacements` is the
 * ascending-by-`start`, non-overlapping list `markBindThisAssigned` returns, each an
 * `[start, end)` span in the ORIGINAL text replaced by `text`. Keeps
 * `extractSvelteHost`'s `toHostOffset` correct once its `hostText` has been edited in
 * place for a `bind:this` fix, without needing to redo the segment-based
 * svelte-offset-to-host-offset computation those edits never touch. An offset landing
 * INSIDE a replaced range has no exact post-edit counterpart (the text there changed);
 * it is clamped to the start of that edit's replacement, a case no caller actually
 * exercises (a doc comment's `commentEnd` never falls inside a `let` declaration
 * statement). */
export function remapOffsetAfterEdits(offset, replacements) {
  let delta = 0;
  for (const r of replacements) {
    if (offset < r.start) break;
    if (offset < r.end) return r.start + delta;
    delta += r.text.length - (r.end - r.start);
  }
  return offset + delta;
}

/** Extracts a Svelte SFC's TypeScript-bearing script content and the data needed to
 * relate a position inside it back to the real `.svelte` file. A `<script module>`
 * block's body (if present) is placed before the instance `<script>` block's body —
 * mirroring the visibility Svelte itself grants a module block's top-level bindings to
 * the instance script, with no import needed. Returns `{ skip: reason }` when there is
 * no instance `<script>` block, or when a present block is not `lang="ts"` — both
 * legitimate, individually-reported dispositions (see `main`'s `.svelte` handling).
 * Otherwise returns `hostText` — with every `bind:this={name}` target's declaration
 * marked definitely-assigned via `markBindThisAssigned` (see that function's own doc
 * for why this is correct rather than a suppression) — plus:
 * - `toHostOffset(svelteOffset)`: translates a character offset in the ORIGINAL SFC
 *   text (e.g. `commentEnd`, from `extractExamples`) to the equivalent offset in
 *   `hostText`, for `findEnclosingClassTarget`. Returns null when the offset falls
 *   outside every script block (a doc comment in the template, never observed but
 *   reported rather than assumed impossible).
 * - `toSvelteLine(hostLineIndex)`: translates a 0-based line index WITHIN `hostText`
 *   back to the 0-based line it occupies in the real SFC, for diagnostic citation.
 *   Returns null for a line with no real correspondent (a synthetic separator inserted
 *   only when a block's body does not itself end in a newline). Unaffected by the
 *   `bind:this` rewrite: that rewrite only ever inserts characters WITHIN an existing
 *   line, never adding or removing a line, so a host line index means the same real
 *   SFC line before and after it runs. */
export function extractSvelteHost(svelteText) {
  const blocks = [];
  for (const m of svelteText.matchAll(SCRIPT_TAG_RE)) {
    const attrs = m[1];
    const body = m[2];
    const openTagLen = m[0].length - body.length - "</script>".length;
    const bodyStart = m.index + openTagLen;
    const isModule = /(?:^|\s)module(?:\s|=|$)/.test(attrs) || /context\s*=\s*["']module["']/.test(attrs);
    const isTs = /lang\s*=\s*["']ts["']/.test(attrs);
    blocks.push({ isModule, isTs, body, bodyStart });
  }
  const moduleBlock = blocks.find((b) => b.isModule) ?? null;
  const instanceBlock = blocks.find((b) => !b.isModule) ?? null;
  if (moduleBlock === null && instanceBlock === null) return { skip: "no <script> block" };
  if (instanceBlock === null) return { skip: "no instance <script> block (module-only SFC)" };
  if (!instanceBlock.isTs) return { skip: 'the <script> instance block is not lang="ts"' };
  if (moduleBlock !== null && !moduleBlock.isTs) return { skip: 'the <script module> block is not lang="ts"' };

  const segments = [];
  let hostText = "";
  for (const block of [moduleBlock, instanceBlock]) {
    if (!block) continue;
    segments.push({ hostOffset: hostText.length, svelteOffset: block.bodyStart, length: block.body.length });
    hostText += block.body;
    if (!hostText.endsWith("\n")) hostText += "\n";
  }

  const rawToHostOffset = (svelteOffset) => {
    for (const seg of segments) {
      if (svelteOffset >= seg.svelteOffset && svelteOffset <= seg.svelteOffset + seg.length) {
        return seg.hostOffset + (svelteOffset - seg.svelteOffset);
      }
    }
    return null;
  };
  const toSvelteOffset = (hostOffset) => {
    for (const seg of segments) {
      if (hostOffset >= seg.hostOffset && hostOffset <= seg.hostOffset + seg.length) {
        return seg.svelteOffset + (hostOffset - seg.hostOffset);
      }
    }
    return null;
  };
  const hostLineStarts = [0];
  for (let i = 0; i < hostText.length; i++) if (hostText[i] === "\n") hostLineStarts.push(i + 1);
  const toSvelteLine = (hostLineIndex) => {
    const hostOffset = hostLineStarts[hostLineIndex];
    if (hostOffset === undefined) return null;
    const svelteOffset = toSvelteOffset(hostOffset);
    return svelteOffset === null ? null : svelteText.slice(0, svelteOffset).split("\n").length - 1;
  };

  const bindThisNames = extractBindThisSimpleIdentifiers(svelteText);
  const { text: markedHostText, marked: bindThisMarked, replacements } = markBindThisAssigned(hostText, bindThisNames);
  const toHostOffset = (svelteOffset) => {
    const raw = rawToHostOffset(svelteOffset);
    return raw === null ? null : remapOffsetAfterEdits(raw, replacements);
  };

  return { hostText: markedHostText, toHostOffset, toSvelteLine, bindThisMarked };
}

/** Compiles every compilable free-function-wrapper example extracted from `.svelte`
 * hosts for one package, mirroring `compilePackageExamples` exactly except for where
 * `hostText` comes from (`extractSvelteHost` instead of the raw file). `examplesByFile`
 * maps each `.svelte` absolute path to its already-extracted `{ hostText,
 * toSvelteLine }` plus the examples destined for this (non-class-context) path. */
function compilePackageSvelteExamples(repoRoot, pkgDir, pkgDirs, examplesByFile) {
  const options = packageCompilerOptions(repoRoot, pkgDir, pkgDirs);
  const selfPackageName = packageOwnName(repoRoot, pkgDir);
  const overlay = new Map();
  const meta = new Map();
  for (const [hostFile, { hostText, toSvelteLine, examples }] of examplesByFile) {
    const dir = dirname(hostFile);
    const base = basename(hostFile, extname(hostFile));
    const hostBindings = hostTopLevelBindings(hostText);
    examples.forEach((ex, i) => {
      const virtualPath = toPosix(join(dir, `${base}.doctest${i}.ts`));
      const resolved = resolveTypeValueClashes(hostText, hostBindings, ex.hoisted, selfPackageName, hostFile);
      const effectiveHostLineCount = resolved.hostText.split("\n").length;
      overlay.set(virtualPath, buildVirtualText(resolved.hostText, ex, resolved.hostBindings, selfPackageName, hostFile));
      meta.set(virtualPath, { hostFile, mapLine: hostLineMapper(effectiveHostLineCount, toSvelteLine), ...ex });
    });
  }
  const diagnosticsByPath = compileOverlay(options, overlay);
  return [...diagnosticsByPath.entries()].map(([virtualPath, diagnostics]) => ({ ...meta.get(virtualPath), diagnostics }));
}

/** Compiles every class-context example extracted from `.svelte` hosts for one
 * package by injecting each as a synthetic method, mirroring
 * `compilePackageClassInjectedExamples` except for the `hostText` source. A
 * class-context example with no enclosing class (the overwhelming majority — a
 * component's script is not itself a TS class) is filtered out by `main` before
 * reaching this function, exactly as for `.ts` hosts. */
function compilePackageSvelteClassInjectedExamples(repoRoot, pkgDir, pkgDirs, examplesByFile) {
  const options = packageCompilerOptions(repoRoot, pkgDir, pkgDirs);
  const selfPackageName = packageOwnName(repoRoot, pkgDir);
  const overlay = new Map();
  const meta = new Map();
  for (const [hostFile, { hostText, toSvelteLine, examples }] of examplesByFile) {
    const dir = dirname(hostFile);
    const base = basename(hostFile, extname(hostFile));
    const hostBindings = hostTopLevelBindings(hostText);
    const hostSourceFile = ts.createSourceFile(hostFile, hostText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
    examples.forEach((ex, i) => {
      const target = findEnclosingClassTarget(hostSourceFile, ex.hostCommentEnd);
      if (target === null) return; // pre-filtered by `main`; defensive only
      const virtualPath = toPosix(join(dir, `${base}.doctest${i}.inject.ts`));
      const built = buildClassInjectionText(hostText, ex, target, hostBindings, selfPackageName, hostFile);
      overlay.set(virtualPath, built.text);
      meta.set(virtualPath, { hostFile, mapLine: arrayLineMapper(built.lineMap, toSvelteLine), ...ex });
    });
  }
  const diagnosticsByPath = compileOverlay(options, overlay);
  return [...diagnosticsByPath.entries()].map(([virtualPath, diagnostics]) => ({ ...meta.get(virtualPath), diagnostics }));
}

/** Where one diagnostic against a compiled example belongs: `"example body"` when
 * `result.mapLine` cannot attribute the line to the host source, else the
 * `"host line N"` string it returns. The single source of truth both `formatFailure`
 * (citation) and `classifyCompiledResult` (pass/fail attribution, see `main`) read, so
 * the two can never disagree about where a diagnostic lands. */
function diagnosticLocation(result, diagnostic) {
  if (diagnostic.file && diagnostic.start !== undefined) {
    const { line } = ts.getLineAndCharacterOfPosition(diagnostic.file, diagnostic.start);
    return result.mapLine(line);
  }
  return "example body";
}

/** Formats a diagnostic against the host module symbol and the example's ordinal
 * within that symbol, distinguishing a diagnostic inside the host's own copied text
 * (real host line — actionable against the actual source) from one inside the example
 * body itself. Never cites the virtual path alone. */
function formatFailure(result) {
  const lines = result.diagnostics.map((d) => `    [${diagnosticLocation(result, d)}] ${ts.flattenDiagnosticMessageText(d.messageText, "\n")}`);
  return `  ${result.hostFile} :: ${result.symbol} :: example #${result.ordinal}\n${lines.join("\n")}`;
}

/** Classifies one compiled example's diagnostics as a genuine example-body failure
 * (at least one diagnostic lands in the example body itself — a content defect on its
 * own merits) versus HOST-only (every diagnostic is host-attributable — real
 * information about a broken host script, but never the example's own fault, per the
 * attribution requirement this checker must honor: a host-side problem is surfaced
 * distinctly from an example-body failure, never folded into it, and never silently
 * discarded either). Returns `"pass"` when `result.diagnostics` is empty. */
function classifyCompiledResult(result) {
  if (result.diagnostics.length === 0) return "pass";
  const hasBodyDiagnostic = result.diagnostics.some((d) => diagnosticLocation(result, d) === "example body");
  return hasBodyDiagnostic ? "body" : "host";
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
    noEmit: true,
    skipLibCheck: true,
    ...EXAMPLE_HYGIENE_OVERRIDES,
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

/** A host module that only ever imports a name TYPE-ONLY, paired with an example
 * needing that same name as a VALUE (to `new` it) — the shape `resolveTypeValueClashes`
 * exists for — must compile GREEN end to end: resolve the clash, build the virtual
 * text, and run it through the real TS compiler. Never left as an unchecked bucket. */
function runTypeValueClashControl(repoRoot) {
  const dir = join(repoRoot, "scripts");
  const helperPath = toPosix(join(dir, "__doctest_control_helper2__.ts"));
  const hostPath = toPosix(join(dir, "__doctest_control_typevalue__.doctest0.ts"));
  const hostText = 'import type { ControlAssetResolver } from "./__doctest_control_helper2__";\nexport {};\n';
  const analyzed = analyzeExample(
    'import { ControlAssetResolver } from "./__doctest_control_helper2__";\n' +
      "const v = new ControlAssetResolver();\nconsole.log(v);",
  );
  const hostBindings = hostTopLevelBindings(hostText);
  const resolved = resolveTypeValueClashes(hostText, hostBindings, analyzed.hoisted, null, null);
  if (resolved.unresolved.size > 0) {
    console.error(
      `compilation control mismatch: type/value clash left unresolved for ${[...resolved.unresolved].join(", ")}`,
    );
    process.exit(1);
  }
  const options = {
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    strict: true,
    noEmit: true,
    skipLibCheck: true,
    ...EXAMPLE_HYGIENE_OVERRIDES,
  };
  const overlay = new Map([
    [helperPath, "export class ControlAssetResolver {}\n"],
    [hostPath, buildVirtualText(resolved.hostText, analyzed, resolved.hostBindings)],
  ]);
  const diagnostics = compileOverlay(options, overlay).get(hostPath);
  if (diagnostics.length !== 0) {
    console.error(
      `compilation control mismatch: type-only-host/value-example example did not compile green:\n` +
        diagnostics.map((d) => `  ${ts.flattenDiagnosticMessageText(d.messageText, "\n")}`).join("\n"),
    );
    process.exit(1);
  }
}

/** `EXAMPLE_HYGIENE_OVERRIDES` must draw the line exactly where it claims to: an
 * example whose only issue is an unused binding compiles GREEN (hygiene is off), and
 * an example with a genuine type error still FAILS (correctness stays on). Proving
 * only the first half would leave the override free to silently swallow real errors
 * alongside the hygiene noise it targets. */
function runHygieneOverrideControl(repoRoot) {
  const dir = join(repoRoot, "scripts");
  const options = {
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    strict: true,
    noEmit: true,
    skipLibCheck: true,
    ...EXAMPLE_HYGIENE_OVERRIDES,
  };

  const unusedPath = toPosix(join(dir, "__doctest_control_hygiene_unused__.doctest0.ts"));
  const unusedAnalyzed = analyzeExample("const controlHygieneUnused = 1;\n");
  const unusedDiagnostics = compileOverlay(
    options,
    new Map([[unusedPath, buildVirtualText("export {};\n", unusedAnalyzed)]]),
  ).get(unusedPath);
  if (unusedDiagnostics.length !== 0) {
    console.error(
      `hygiene control mismatch: an example whose only issue is an unused binding did not compile green:\n` +
        unusedDiagnostics.map((d) => `  ${ts.flattenDiagnosticMessageText(d.messageText, "\n")}`).join("\n"),
    );
    process.exit(1);
  }

  const typeErrorPath = toPosix(join(dir, "__doctest_control_hygiene_typeerror__.doctest0.ts"));
  const typeErrorAnalyzed = analyzeExample('const controlHygieneTypeError: number = "not a number";\n');
  const typeErrorDiagnostics = compileOverlay(
    options,
    new Map([[typeErrorPath, buildVirtualText("export {};\n", typeErrorAnalyzed)]]),
  ).get(typeErrorPath);
  if (typeErrorDiagnostics.length === 0) {
    console.error(
      "hygiene control mismatch: an example with a genuine type error compiled green — " +
        "EXAMPLE_HYGIENE_OVERRIDES is masking correctness errors, not just hygiene lints",
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

/** Class-injection controls, exercised through the real extract → analyze → target →
 * inject → compile pipeline against one synthetic host class: a `this.`-style example
 * calling a REAL private method compiles GREEN; one calling a private member that does
 * NOT EXIST fails; and an example on a STATIC method resolves `this` against the
 * static side specifically. The static/instance case gives both sides a private member
 * of the SAME NAME but different parameter arity — a static/instance mixup is a type
 * that still checks (a name that resolves, just on the wrong side), not a syntax
 * error, so a naive same-signature test would pass even with the wrong side wired in;
 * the arity mismatch forces a genuine compile failure if the wrong side is used. */
function runClassInjectionControl(repoRoot) {
  const dir = join(repoRoot, "scripts");
  const hostFile = toPosix(join(dir, "__doctest_control_injection_host__.ts"));
  const hostText = [
    "export class ControlInjectionHost {",
    "  /**",
    "   * @example",
    "   * ```ts",
    '   * this.sharedName("hi");',
    "   * ```",
    "   */",
    "  instanceMethod(): void {",
    '    this.sharedName("hi");',
    "  }",
    "",
    "  private sharedName(arg: string): void {",
    "    void arg;",
    "  }",
    "",
    "  /**",
    "   * @example",
    "   * ```ts",
    "   * this.sharedName();",
    "   * ```",
    "   */",
    "  static staticMethod(): void {}",
    "",
    "  private static sharedName(): void {}",
    "",
    "  /**",
    "   * @example",
    "   * ```ts",
    "   * this.doesNotExist();",
    "   * ```",
    "   */",
    "  instanceMethod2(): void {}",
    "}",
    "",
  ].join("\n");

  const examples = extractExamples(hostText);
  if (examples.length !== 3) {
    console.error(`class-injection control setup mismatch: expected 3 examples, found ${examples.length}`);
    process.exit(1);
  }
  const [instanceEx, staticEx, missingEx] = examples;
  const hostBindings = hostTopLevelBindings(hostText);
  const options = {
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    strict: true,
    noEmit: true,
    skipLibCheck: true,
    ...EXAMPLE_HYGIENE_OVERRIDES,
  };

  const compileOne = (ex) => {
    const analyzed = analyzeExample(ex.code);
    const sourceFile = ts.createSourceFile(hostFile, hostText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
    const target = findEnclosingClassTarget(sourceFile, ex.commentEnd);
    if (target === null) {
      console.error(`class-injection control setup mismatch: no enclosing class found for ${ex.symbol}`);
      process.exit(1);
    }
    const built = buildClassInjectionText(hostText, { ...ex, ...analyzed }, target, hostBindings);
    const virtualPath = toPosix(join(dir, `__doctest_control_injection__${ex.symbol}.doctest0.ts`));
    return compileOverlay(options, new Map([[virtualPath, built.text]])).get(virtualPath);
  };

  const instanceDiagnostics = compileOne(instanceEx);
  if (instanceDiagnostics.length !== 0) {
    console.error(
      `class-injection control mismatch: a real-private-method example did not compile green:\n` +
        instanceDiagnostics.map((d) => `  ${ts.flattenDiagnosticMessageText(d.messageText, "\n")}`).join("\n"),
    );
    process.exit(1);
  }

  const staticDiagnostics = compileOne(staticEx);
  if (staticDiagnostics.length !== 0) {
    console.error(
      `class-injection control mismatch: a static-method example did not resolve against the static side:\n` +
        staticDiagnostics.map((d) => `  ${ts.flattenDiagnosticMessageText(d.messageText, "\n")}`).join("\n"),
    );
    process.exit(1);
  }

  const missingDiagnostics = compileOne(missingEx);
  if (missingDiagnostics.length === 0) {
    console.error("class-injection control mismatch: an example calling a nonexistent private member compiled green");
    process.exit(1);
  }
}

/** `.svelte` host controls, exercised through the real `extractExamples` →
 * `extractSvelteHost` → `analyzeExample` → `buildVirtualText`/`buildClassInjectionText`
 * → `compileOverlay` pipeline against synthetic SFC text (never a file on disk):
 * - **Rune typing is real, not `any`.** A host binding `let n = $state(0)` must make
 *   an example doing `n.toUpperCase()` FAIL, and one doing arithmetic on it (`n * 2`)
 *   compile GREEN — proving both that the ambient rune declarations resolved with
 *   their real generic signature AND that the FAIL half isn't a false positive from
 *   something else entirely.
 * - **Host context resolves.** An example calling a real host-declared function
 *   compiles GREEN; one calling a name the host never declares FAILS.
 * - **`<script module>` bindings resolve.** An example reading a module-block export,
 *   with no import, compiles GREEN — the same visibility Svelte itself grants an
 *   instance script over its own module block.
 * - **An unrelated host-side `.svelte` import never masks a real example error.** A
 *   host importing a nonexistent-on-disk `./Child.svelte` component — resolved by
 *   svelte's own `declare module '*.svelte'` ambient declaration, pulled in the same
 *   way as the rune globals, needing no shim of this script's own — compiles GREEN
 *   for a correct example and still FAILS for one with a genuine type error, proving
 *   neither diagnostic direction is accidentally swallowed. */
function runSvelteHostControl(repoRoot) {
  const dir = join(repoRoot, "scripts");
  const options = {
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    strict: true,
    noEmit: true,
    skipLibCheck: true,
    types: ["svelte"],
    ...EXAMPLE_HYGIENE_OVERRIDES,
  };

  const compileSvelteExample = (svelteText, exampleIndex, label) => {
    const examples = extractExamples(svelteText);
    const host = extractSvelteHost(svelteText);
    if (host.skip) {
      console.error(`svelte control setup mismatch (${label}): ${host.skip}`);
      process.exit(1);
    }
    const ex = examples[exampleIndex];
    if (!ex) {
      console.error(`svelte control setup mismatch (${label}): expected an example at index ${exampleIndex}`);
      process.exit(1);
    }
    const analyzed = analyzeExample(ex.code);
    const hostBindings = hostTopLevelBindings(host.hostText);
    const virtualPath = toPosix(join(dir, `__doctest_control_svelte_${label}__.doctest0.ts`));
    const overlay = new Map([
      [virtualPath, buildVirtualText(host.hostText, { ...ex, ...analyzed }, hostBindings)],
    ]);
    return compileOverlay(options, overlay).get(virtualPath);
  };

  const requireGreen = (label, diagnostics) => {
    if (diagnostics.length !== 0) {
      console.error(
        `svelte control mismatch (${label}): expected GREEN but got:\n` +
          diagnostics.map((d) => `  ${ts.flattenDiagnosticMessageText(d.messageText, "\n")}`).join("\n"),
      );
      process.exit(1);
    }
  };
  const requireFail = (label, diagnostics) => {
    if (diagnostics.length === 0) {
      console.error(`svelte control mismatch (${label}): expected a compile failure but got GREEN`);
      process.exit(1);
    }
  };

  const runeHost = [
    '<script lang="ts">',
    "  let n = $state(0);",
    "  /**",
    "   * @example",
    "   * ```ts",
    "   * n.toUpperCase();",
    "   * ```",
    "   */",
    "  function controlRuneFail(): void {}",
    "  /**",
    "   * @example",
    "   * ```ts",
    "   * const doubled = n * 2;",
    "   * console.log(doubled);",
    "   * ```",
    "   */",
    "  function controlRunePass(): void {}",
    "</script>",
    "",
  ].join("\n");
  requireFail("rune-typed-not-any", compileSvelteExample(runeHost, 0, "rune_fail"));
  requireGreen("rune-typed-arithmetic-still-works", compileSvelteExample(runeHost, 1, "rune_pass"));

  const hostContextHost = [
    '<script lang="ts">',
    "  function controlHostHelper(): number {",
    "    return 1;",
    "  }",
    "  /**",
    "   * @example",
    "   * ```ts",
    "   * const v: number = controlHostHelper();",
    "   * console.log(v);",
    "   * ```",
    "   */",
    "  function controlHostGreen(): void {}",
    "  /**",
    "   * @example",
    "   * ```ts",
    "   * controlHostMissing();",
    "   * ```",
    "   */",
    "  function controlHostFail(): void {}",
    "</script>",
    "",
  ].join("\n");
  requireGreen("host-context-resolves", compileSvelteExample(hostContextHost, 0, "host_green"));
  requireFail("host-context-missing-member-fails", compileSvelteExample(hostContextHost, 1, "host_fail"));

  const moduleBlockHost = [
    '<script module lang="ts">',
    "  export const CONTROL_MODULE_VALUE = 42;",
    "</script>",
    "",
    '<script lang="ts">',
    "  /**",
    "   * @example",
    "   * ```ts",
    "   * const doubled: number = CONTROL_MODULE_VALUE * 2;",
    "   * console.log(doubled);",
    "   * ```",
    "   */",
    "  function controlModuleBinding(): void {}",
    "</script>",
    "",
  ].join("\n");
  requireGreen("module-block-binding-visible", compileSvelteExample(moduleBlockHost, 0, "module_block"));

  const componentImportHost = [
    '<script lang="ts">',
    '  import ControlChild from "./__doctest_control_svelte_child__.svelte";',
    "  void ControlChild;",
    "  /**",
    "   * @example",
    "   * ```ts",
    "   * const good: number = 1;",
    "   * console.log(good);",
    "   * ```",
    "   */",
    "  function controlComponentImportNeverReported(): void {}",
    "  /**",
    "   * @example",
    "   * ```ts",
    "   * const bad: number = \"not a number\";",
    "   * console.log(bad);",
    "   * ```",
    "   */",
    "  function controlComponentImportStillCatchesRealErrors(): void {}",
    "</script>",
    "",
  ].join("\n");
  requireGreen("unrelated-component-import-not-reported", compileSvelteExample(componentImportHost, 0, "component_import_green"));
  requireFail("unrelated-component-import-does-not-mask-real-errors", compileSvelteExample(componentImportHost, 1, "component_import_fail"));

  // A synthetic-`this`-referencing example inside a class DECLARED in a .svelte
  // instance script exercises the full class-injection path for a `.svelte` host —
  // target resolution AND the `arrayLineMapper(built.lineMap, toSvelteLine)`
  // composition that maps a diagnostic back to the real SFC line through TWO
  // indirections (the class-injection splice, then the script-extraction offset),
  // never proven by the free-function-only checks above.
  const classInjectionSrc = [
    '<div>template</div>',
    '<script lang="ts">',
    "  class ControlSvelteClass {",
    "    /**",
    "     * @example",
    "     * ```ts",
    "     * this.controlSharedName();",
    "     * ```",
    "     */",
    "    instanceMethod(): void {",
    "      this.controlSharedName();",
    "    }",
    "    private controlSharedName(): void {}",
    "    /**",
    "     * @example",
    "     * ```ts",
    "     * this.doesNotExist();",
    "     * ```",
    "     */",
    "    instanceMethod2(): void {}",
    "  }",
    "</script>",
    "",
  ].join("\n");
  const classHost = extractSvelteHost(classInjectionSrc);
  if (classHost.skip) {
    console.error(`svelte control setup mismatch (class-injection): ${classHost.skip}`);
    process.exit(1);
  }
  const classExamples = extractExamples(classInjectionSrc);
  if (classExamples.length !== 2) {
    console.error(`svelte control setup mismatch (class-injection): expected 2 examples, found ${classExamples.length}`);
    process.exit(1);
  }
  const classHostBindings = hostTopLevelBindings(classHost.hostText);
  const classHostSourceFile = ts.createSourceFile("host.ts", classHost.hostText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const compileClassInjected = (ex, label) => {
    const hostCommentEnd = classHost.toHostOffset(ex.commentEnd);
    const target = findEnclosingClassTarget(classHostSourceFile, hostCommentEnd);
    if (target === null) {
      console.error(`svelte control setup mismatch (class-injection ${label}): no enclosing class found`);
      process.exit(1);
    }
    const built = buildClassInjectionText(classHost.hostText, { ...ex, ...analyzeExample(ex.code) }, target, classHostBindings);
    const virtualPath = toPosix(join(dir, `__doctest_control_svelte_class_${label}__.doctest0.ts`));
    const diagnostics = compileOverlay(options, new Map([[virtualPath, built.text]])).get(virtualPath);
    return { diagnostics, mapLine: arrayLineMapper(built.lineMap, classHost.toSvelteLine) };
  };

  const realMethodResult = compileClassInjected(classExamples[0], "real_method");
  requireGreen("class-injection-real-private-method", realMethodResult.diagnostics);

  const missingResult = compileClassInjected(classExamples[1], "missing_method");
  requireFail("class-injection-missing-private-member", missingResult.diagnostics);
  // The failing call lives inside the INJECTED synthetic method (example-authored
  // text, marked by `buildClassInjectionText`'s EXAMPLE_BODY markers) — the composed
  // `arrayLineMapper(built.lineMap, classHost.toSvelteLine)` must still report
  // "example body" for it, never a fabricated host line, proving the extra
  // svelte-offset indirection didn't corrupt the null/non-null marker distinction.
  const missingDiag = missingResult.diagnostics[0];
  const { line } = ts.getLineAndCharacterOfPosition(missingDiag.file, missingDiag.start);
  const citedLine = missingResult.mapLine(line);
  if (citedLine !== "example body") {
    console.error(`svelte control mismatch (class-injection-line-citation): expected "example body", got "${citedLine}"`);
    process.exit(1);
  }

  // A diagnostic on the class's OWN (real host) member declaration — never inside an
  // injected region — must map to the REAL .svelte line through the same composition.
  const badMemberSrc = [
    "<div>template line one</div>",
    "<div>template line two</div>",
    '<script lang="ts">',
    "  class ControlSvelteBadMember {",
    '    private controlBadHostMember: number = "not a number";',
    "    /**",
    "     * @example",
    "     * ```ts",
    "     * this.controlBadHostMember;",
    "     * ```",
    "     */",
    "    instanceMethod(): void {",
    "      void this.controlBadHostMember;",
    "    }",
    "  }",
    "</script>",
    "",
  ].join("\n");
  const badMemberHost = extractSvelteHost(badMemberSrc);
  const badMemberEx = extractExamples(badMemberSrc)[0];
  const badMemberBindings = hostTopLevelBindings(badMemberHost.hostText);
  const badMemberSourceFile = ts.createSourceFile(
    "host.ts",
    badMemberHost.hostText,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TS,
  );
  const badMemberTarget = findEnclosingClassTarget(badMemberSourceFile, badMemberHost.toHostOffset(badMemberEx.commentEnd));
  const badMemberBuilt = buildClassInjectionText(
    badMemberHost.hostText,
    { ...badMemberEx, ...analyzeExample(badMemberEx.code) },
    badMemberTarget,
    badMemberBindings,
  );
  const badMemberVirtualPath = toPosix(join(dir, "__doctest_control_svelte_class_bad_member__.doctest0.ts"));
  const badMemberDiagnostics = compileOverlay(options, new Map([[badMemberVirtualPath, badMemberBuilt.text]])).get(
    badMemberVirtualPath,
  );
  requireFail("class-injection-bad-host-member-declaration-fails", badMemberDiagnostics);
  const badMemberDiag = badMemberDiagnostics[0];
  const badMemberLine = ts.getLineAndCharacterOfPosition(badMemberDiag.file, badMemberDiag.start).line;
  const badMemberCited = arrayLineMapper(badMemberBuilt.lineMap, badMemberHost.toSvelteLine)(badMemberLine);
  const expectedRealLine = badMemberSrc.split("\n").findIndex((l) => l.includes("controlBadHostMember: number")) + 1;
  if (badMemberCited !== `host line ${expectedRealLine}`) {
    console.error(
      `svelte control mismatch (class-injection-host-line-citation): expected "host line ${expectedRealLine}", got "${badMemberCited}"`,
    );
    process.exit(1);
  }
}

/** `bind:this` definite-assignment controls, exercised through the real
 * `extractBindThisSimpleIdentifiers` → `markBindThisAssigned` → `extractSvelteHost` →
 * `buildVirtualText` → `compileOverlay` pipeline:
 * - **The definite-assignment marking does not blind the checker.** A host variable read
 *   before assignment that is NOT named by any `bind:this` in the template must still FAIL —
 *   the analysis stays real rather than silenced (a global `strictNullChecks` disable, or a
 *   filter on the diagnostic's message text, would pass this control's twin covering
 *   the actual bind:this target below but fail to distinguish it from this one).
 * - **A real `bind:this` target still FAILS on genuine type errors.** Marking a
 *   declaration definitely-assigned must not widen its type to `any`: a
 *   `bind:this={el}` bound to `HTMLDivElement` must still fail an example calling a
 *   nonexistent method on `el`, and still PASS ordinary use of its real members. */
function runBindThisAssignmentControl(repoRoot) {
  const dir = join(repoRoot, "scripts");
  const options = {
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    strict: true,
    noEmit: true,
    skipLibCheck: true,
    types: ["svelte"],
    ...EXAMPLE_HYGIENE_OVERRIDES,
  };
  const compile = (svelteText, exampleIndex, label) => {
    const examples = extractExamples(svelteText);
    const host = extractSvelteHost(svelteText);
    if (host.skip) {
      console.error(`bind:this control setup mismatch (${label}): ${host.skip}`);
      process.exit(1);
    }
    const ex = examples[exampleIndex];
    const analyzed = analyzeExample(ex.code);
    const hostBindings = hostTopLevelBindings(host.hostText);
    const virtualPath = toPosix(join(dir, `__doctest_control_bindthis_${label}__.doctest0.ts`));
    const overlay = new Map([[virtualPath, buildVirtualText(host.hostText, { ...ex, ...analyzed }, hostBindings)]]);
    return { host, diagnostics: compileOverlay(options, overlay).get(virtualPath) };
  };
  const requireGreen = (label, diagnostics) => {
    if (diagnostics.length !== 0) {
      console.error(
        `bind:this control mismatch (${label}): expected GREEN but got:\n` +
          diagnostics.map((d) => `  ${ts.flattenDiagnosticMessageText(d.messageText, "\n")}`).join("\n"),
      );
      process.exit(1);
    }
  };
  const requireFail = (label, diagnostics, needle) => {
    const hit = diagnostics.some((d) => ts.flattenDiagnosticMessageText(d.messageText, "\n").includes(needle));
    if (!hit) {
      console.error(
        `bind:this control mismatch (${label}): expected a diagnostic containing "${needle}" but got:\n` +
          diagnostics.map((d) => `  ${ts.flattenDiagnosticMessageText(d.messageText, "\n")}`).join("\n"),
      );
      process.exit(1);
    }
  };

  // A `let` never named by any `bind:this` in the template, read before assignment
  // from inside a nested function — the exact shape the real defect takes, minus the
  // template binding — must still be reported: the definite-assignment marking must
  // not blind the checker to genuine "used before assigned" bugs.
  const notBoundHost = [
    "<div>no bind:this here</div>",
    '<script lang="ts">',
    "  export const controlModuleMarker = true;", // real component-shaped module scope (mirrors imports every real host has), not the special non-module script scope TS analyzes differently
    "  let controlNotBoundThis: number;",
    "  function controlReadsEarly(): void {",
    "    console.log(controlNotBoundThis);",
    "  }",
    "  /**",
    "   * @example",
    "   * ```ts",
    "   * controlReadsEarly();",
    "   * ```",
    "   */",
    "  function controlNotBoundExample(): void {}",
    "</script>",
    "",
  ].join("\n");
  if (extractBindThisSimpleIdentifiers(notBoundHost).has("controlNotBoundThis")) {
    console.error("bind:this control setup mismatch (not-bound): fixture unexpectedly contains a bind:this target");
    process.exit(1);
  }
  requireFail(
    "unrelated-used-before-assigned-still-fails",
    compile(notBoundHost, 0, "not_bound").diagnostics,
    "used before being assigned",
  );

  // The real defect shape: a `bind:this` target read before assignment from inside a
  // nested function must compile GREEN (the definite-assignment assertion resolved
  // it), while its TYPE stays real: a call to a nonexistent member still FAILS, and
  // ordinary use of a real member still PASSES.
  const boundHost = [
    '<div bind:this={controlBoundEl}></div>',
    '<script lang="ts">',
    "  export const controlModuleMarker2 = true;", // real component-shaped module scope, per the note on `notBoundHost` above
    "  let controlBoundEl: HTMLDivElement;",
    "  function controlReadsEl(): void {",
    "    console.log(controlBoundEl.tagName);",
    "  }",
    "  /**",
    "   * @example",
    "   * ```ts",
    "   * controlReadsEl();",
    "   * controlBoundEl.click();",
    "   * ```",
    "   */",
    "  function controlBoundGreen(): void {}",
    "  /**",
    "   * @example",
    "   * ```ts",
    "   * controlBoundEl.controlNonexistentMethod();",
    "   * ```",
    "   */",
    "  function controlBoundTypeStillReal(): void {}",
    "</script>",
    "",
  ].join("\n");
  if (!extractBindThisSimpleIdentifiers(boundHost).has("controlBoundEl")) {
    console.error("bind:this control setup mismatch (bound): fixture's bind:this target was not extracted");
    process.exit(1);
  }
  const { host: boundHostResult } = compile(boundHost, 0, "bound_setup");
  if (!boundHostResult.bindThisMarked.has("controlBoundEl")) {
    console.error("bind:this control mismatch (declaration-marked): controlBoundEl was not marked definitely-assigned");
    process.exit(1);
  }
  requireGreen("bound-target-resolves-and-real-member-passes", compile(boundHost, 0, "bound_green").diagnostics);
  requireFail(
    "bound-target-stays-typed-not-any",
    compile(boundHost, 1, "bound_fail").diagnostics,
    "controlNonexistentMethod",
  );
}

const repoRootForControls = resolve(fileURLToPath(import.meta.url), "..", "..");
runExtractionControls();
runCompilationControl(repoRootForControls);
runTypeValueClashControl(repoRootForControls);
runHygieneOverrideControl(repoRootForControls);
runClassContextControl();
runClassInjectionControl(repoRootForControls);
runSvelteHostControl(repoRootForControls);
runBindThisAssignmentControl(repoRootForControls);

if (isDirectEntry(import.meta.url)) {
  const repo = repoRootForControls;
  const roots = ["src/types", "src/client", "src/modules", "examples"];
  const pkgDirs = workspacePackageDirs(repo);

  const byPackage = new Map();
  const byPackageInjected = new Map();
  const byPackageSvelte = new Map();
  const byPackageSvelteInjected = new Map();
  let compileTotal = 0;
  const unresolvedClashes = [];
  const noEnclosingClass = [];
  for (const file of candidateFiles(repo, roots)) {
    const hostText = readFileSync(file, "utf8");
    const examples = extractExamples(hostText);
    if (examples.length === 0) continue;
    const pkgDir = packageForFile(repo, pkgDirs, file);
    const selfPackageName = pkgDir === null ? null : packageOwnName(repo, pkgDir);
    const hostBindings = hostTopLevelBindings(hostText);
    const hostSourceFile = ts.createSourceFile(file, hostText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
    for (const ex of examples) {
      const analyzed = analyzeExample(ex.code);
      if (analyzed.classContext) {
        // The injection target is the class enclosing the DOCUMENTED SYMBOL, found by
        // AST position from `ex.commentEnd` — never merely the first or nearest class
        // in the file. A top-level example with no enclosing class is never guessed
        // at; it is reported individually below instead.
        const target = findEnclosingClassTarget(hostSourceFile, ex.commentEnd);
        if (target === null) {
          noEnclosingClass.push({ hostFile: file, symbol: ex.symbol, ordinal: ex.ordinal });
          continue;
        }
        if (pkgDir === null) continue; // outside every workspace package: cannot resolve a tsconfig
        compileTotal += 1;
        if (!byPackageInjected.has(pkgDir)) byPackageInjected.set(pkgDir, new Map());
        const forFileInjected = byPackageInjected.get(pkgDir);
        if (!forFileInjected.has(file)) forFileInjected.set(file, []);
        forFileInjected.get(file).push({ ...ex, ...analyzed });
        continue;
      }
      if (pkgDir === null) continue; // outside every workspace package: cannot resolve a tsconfig
      // A type/value clash is resolved by upgrading the host's own clashing import(s)
      // — never left as a silent unchecked bucket. A name that resists the upgrade
      // (see `resolveTypeValueClashes`) is reported individually below, with its own
      // reason, and still fails the gate.
      const { unresolved } = resolveTypeValueClashes(hostText, hostBindings, analyzed.hoisted, selfPackageName, file);
      if (unresolved.size > 0) {
        unresolvedClashes.push({ hostFile: file, symbol: ex.symbol, ordinal: ex.ordinal, names: [...unresolved] });
        continue;
      }
      compileTotal += 1;
      if (!byPackage.has(pkgDir)) byPackage.set(pkgDir, new Map());
      const forFile = byPackage.get(pkgDir);
      if (!forFile.has(file)) forFile.set(file, []);
      forFile.get(file).push({ ...ex, ...analyzed });
    }
  }

  // Svelte SFC hosts: same three dispositions as a `.ts` host (compile, unresolved
  // clash, no enclosing class) plus one host-structural bucket `.ts` never needs — no
  // usable `<script lang="ts">` content to compile against at all.
  const svelteNoScript = [];
  let svelteTotal = 0;
  let svelteCompiled = 0;
  for (const file of svelteFiles(repo, roots)) {
    const svelteText = readFileSync(file, "utf8");
    const examples = extractExamples(svelteText);
    if (examples.length === 0) continue;
    svelteTotal += examples.length;
    const host = extractSvelteHost(svelteText);
    if (host.skip) {
      for (const ex of examples) svelteNoScript.push({ hostFile: file, symbol: ex.symbol, ordinal: ex.ordinal, reason: host.skip });
      continue;
    }
    const { hostText, toHostOffset, toSvelteLine } = host;
    const pkgDir = packageForFile(repo, pkgDirs, file);
    if (pkgDir === null) {
      for (const ex of examples) {
        svelteNoScript.push({
          hostFile: file,
          symbol: ex.symbol,
          ordinal: ex.ordinal,
          reason: "outside every workspace package: cannot resolve a tsconfig",
        });
      }
      continue;
    }
    const selfPackageName = packageOwnName(repo, pkgDir);
    const hostBindings = hostTopLevelBindings(hostText);
    const hostSourceFile = ts.createSourceFile(file, hostText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
    for (const ex of examples) {
      const analyzed = analyzeExample(ex.code);
      const hostCommentEnd = toHostOffset(ex.commentEnd);
      if (hostCommentEnd === null) {
        svelteNoScript.push({
          hostFile: file,
          symbol: ex.symbol,
          ordinal: ex.ordinal,
          reason: "the doc comment lies outside any TS script block",
        });
        continue;
      }
      if (analyzed.classContext) {
        const target = findEnclosingClassTarget(hostSourceFile, hostCommentEnd);
        if (target === null) {
          noEnclosingClass.push({ hostFile: file, symbol: ex.symbol, ordinal: ex.ordinal });
          continue;
        }
        compileTotal += 1;
        svelteCompiled += 1;
        if (!byPackageSvelteInjected.has(pkgDir)) byPackageSvelteInjected.set(pkgDir, new Map());
        const forFileInjected = byPackageSvelteInjected.get(pkgDir);
        if (!forFileInjected.has(file)) forFileInjected.set(file, { hostText, toSvelteLine, examples: [] });
        forFileInjected.get(file).examples.push({ ...ex, ...analyzed, hostCommentEnd });
        continue;
      }
      const { unresolved } = resolveTypeValueClashes(hostText, hostBindings, analyzed.hoisted, selfPackageName, file);
      if (unresolved.size > 0) {
        unresolvedClashes.push({ hostFile: file, symbol: ex.symbol, ordinal: ex.ordinal, names: [...unresolved] });
        continue;
      }
      compileTotal += 1;
      svelteCompiled += 1;
      if (!byPackageSvelte.has(pkgDir)) byPackageSvelte.set(pkgDir, new Map());
      const forFile = byPackageSvelte.get(pkgDir);
      if (!forFile.has(file)) forFile.set(file, { hostText, toSvelteLine, examples: [] });
      forFile.get(file).examples.push({ ...ex, ...analyzed });
    }
  }

  const svelteNoEnclosingClass = noEnclosingClass.filter((u) => extname(u.hostFile) === ".svelte").length;
  const svelteUnresolvedClashes = unresolvedClashes.filter((u) => extname(u.hostFile) === ".svelte").length;
  const svelteSkippedTotal = svelteNoScript.length + svelteNoEnclosingClass + svelteUnresolvedClashes;
  if (svelteCompiled + svelteSkippedTotal !== svelteTotal) {
    console.error(
      `svelte example census mismatch: ${svelteCompiled} compiled + ${svelteSkippedTotal} reported-skipped ` +
        `= ${svelteCompiled + svelteSkippedTotal}, expected ${svelteTotal} total @example blocks found in .svelte sources`,
    );
    process.exit(1);
  }

  if (svelteNoScript.length > 0) {
    console.error(`${svelteNoScript.length} @example blocks in .svelte sources have no compilable script content:`);
    for (const u of svelteNoScript) {
      console.error(`  ${u.hostFile} :: ${u.symbol} :: example #${u.ordinal} :: ${u.reason}`);
    }
  }

  if (unresolvedClashes.length > 0) {
    console.error(
      `${unresolvedClashes.length} @example blocks need a name as a value the host cannot provide even after ` +
        `upgrading its type-only import:`,
    );
    for (const u of unresolvedClashes) {
      console.error(
        `  ${u.hostFile} :: ${u.symbol} :: example #${u.ordinal} :: cannot upgrade ${u.names.join(", ")} — ` +
          `the host declares it directly (not via an import), so no import statement can be rewritten`,
      );
    }
  }

  if (noEnclosingClass.length > 0) {
    console.error(
      `${noEnclosingClass.length} @example blocks reference an enclosing class's this/private members but have ` +
        `no enclosing class:`,
    );
    for (const u of noEnclosingClass) {
      console.error(
        `  ${u.hostFile} :: ${u.symbol} :: example #${u.ordinal} :: the documented declaration is not a class ` +
          `member — no injection target exists, and none is guessed`,
      );
    }
  }

  const anyUnresolved = unresolvedClashes.length > 0 || noEnclosingClass.length > 0 || svelteNoScript.length > 0;

  if (compileTotal === 0) {
    if (anyUnresolved) process.exit(1);
    console.log("no compilable @example ts blocks found — trivially green");
    process.exit(0);
  }

  // `failures`: at least one diagnostic lands in the EXAMPLE BODY — a genuine content
  // defect on this example's own merits. `hostOnlyFailures`: every diagnostic is
  // HOST-attributable — real information about a broken host script, surfaced below
  // distinctly (never discarded), but never counted as this example's own failure; see
  // `classifyCompiledResult`.
  const failures = [];
  const hostOnlyFailures = [];
  const collect = (result) => {
    const verdict = classifyCompiledResult(result);
    if (verdict === "body") failures.push(result);
    else if (verdict === "host") hostOnlyFailures.push(result);
  };
  for (const [pkgDir, examplesByFile] of byPackage) {
    for (const result of compilePackageExamples(repo, pkgDir, pkgDirs, examplesByFile)) collect(result);
  }
  for (const [pkgDir, examplesByFile] of byPackageInjected) {
    for (const result of compilePackageClassInjectedExamples(repo, pkgDir, pkgDirs, examplesByFile)) collect(result);
  }
  for (const [pkgDir, examplesByFile] of byPackageSvelte) {
    for (const result of compilePackageSvelteExamples(repo, pkgDir, pkgDirs, examplesByFile)) collect(result);
  }
  for (const [pkgDir, examplesByFile] of byPackageSvelteInjected) {
    for (const result of compilePackageSvelteClassInjectedExamples(repo, pkgDir, pkgDirs, examplesByFile)) collect(result);
  }

  if (hostOnlyFailures.length > 0) {
    console.error(
      `${hostOnlyFailures.length} examples compiled with diagnostics attributable ONLY to their HOST script ` +
        `(none in the example body itself) — real host defects, not counted as example content failures:\n`,
    );
    for (const f of hostOnlyFailures) console.error(formatFailure(f));
  }

  if (failures.length > 0 || hostOnlyFailures.length > 0 || anyUnresolved) {
    if (failures.length > 0) {
      console.error(`${failures.length} of ${compileTotal} compilable examples failed to compile:\n`);
      for (const f of failures) console.error(formatFailure(f));
      console.error(`\n${failures.length} of ${compileTotal} TS doc examples FAILED to typecheck`);
    }
    process.exit(1);
  }
  console.log(`${compileTotal} TS doc examples typecheck OK`);
}
