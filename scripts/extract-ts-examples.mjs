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
 * past a fixed count belongs to the example. */
function hostLineMapper(hostLineCount) {
  return (line) => (line < hostLineCount ? `host line ${line + 1}` : "example body");
}

/** A diagnostic-line formatter backed by `buildLineMap` — used by the class-injection
 * path, where the example's method is spliced into the MIDDLE of the host's text, so a
 * single threshold cannot distinguish host lines that follow the injection point from
 * the injected content itself. */
function arrayLineMapper(lineMap) {
  return (line) => {
    const hostLine = lineMap[line];
    return hostLine === null || hostLine === undefined ? "example body" : `host line ${hostLine + 1}`;
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
      where = result.mapLine(line);
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

const repoRootForControls = resolve(fileURLToPath(import.meta.url), "..", "..");
runExtractionControls();
runCompilationControl(repoRootForControls);
runTypeValueClashControl(repoRootForControls);
runHygieneOverrideControl(repoRootForControls);
runClassContextControl();
runClassInjectionControl(repoRootForControls);

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
  const byPackageInjected = new Map();
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

  console.log(`${svelteCount} @example blocks found in .svelte sources — unchecked (never compiled)`);

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

  if (compileTotal === 0) {
    if (unresolvedClashes.length > 0 || noEnclosingClass.length > 0) process.exit(1);
    console.log("no compilable @example ts blocks found — trivially green");
    process.exit(0);
  }

  const failures = [];
  for (const [pkgDir, examplesByFile] of byPackage) {
    for (const result of compilePackageExamples(repo, pkgDir, pkgDirs, examplesByFile)) {
      if (result.diagnostics.length > 0) failures.push(result);
    }
  }
  for (const [pkgDir, examplesByFile] of byPackageInjected) {
    for (const result of compilePackageClassInjectedExamples(repo, pkgDir, pkgDirs, examplesByFile)) {
      if (result.diagnostics.length > 0) failures.push(result);
    }
  }

  if (failures.length > 0 || unresolvedClashes.length > 0 || noEnclosingClass.length > 0) {
    if (failures.length > 0) {
      console.error(`${failures.length} of ${compileTotal} compilable examples failed to compile:\n`);
      for (const f of failures) console.error(formatFailure(f));
      console.error(`\n${failures.length} of ${compileTotal} TS doc examples FAILED to typecheck`);
    }
    process.exit(1);
  }
  console.log(`${compileTotal} TS doc examples typecheck OK`);
}
