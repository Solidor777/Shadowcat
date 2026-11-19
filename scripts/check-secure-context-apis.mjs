// Fails when client code names an API that a plain-HTTP, non-loopback origin does not expose.
//
// Documented LAN play serves the client over plain HTTP at a LAN address. Browsers treat that
// origin as NOT a secure context (HTML "secure contexts"; MDN "Secure Contexts"), and every
// `[SecureContext]` Web IDL member is absent there. `Crypto.randomUUID` is one: the property reads
// `undefined`, so the call throws `TypeError` on every LAN client while unit tests (Node) and the
// e2e suite's loopback project (a secure context) never see it. `Crypto.getRandomValues` is not
// `[SecureContext]`, so the single seam that builds UUIDs from it, `randomUuid` in
// `@shadowcat/core`, is the only place allowed to name the missing member, together with the
// seam's own test (`uuid.test.ts`) and the HTTPS secure-context e2e test file (`HTTPS_SPEC_FILE`). That
// file is listed for the `randomUUID` rule alone: it asserts that the member IS a function on the
// built-in HTTPS origin, the positive counterpart of the plain-HTTP absence the rest of this gate
// protects, and no other rule or file is widened for it.
//
// The gate bans only members whose ABSENCE BREAKS THE CLIENT: a call or a property read on
// `undefined` throws `TypeError` on a LAN origin. A member the client merely READS as a property
// (`navigator.deviceMemory`, a Chromium-only signal) yields `undefined` there, never throws, and is
// already tolerated on Firefox and Safari, so it has no rule.
//
// One table (`RULES`) drives two detectors:
//   - a raw-text pass over every covered file (comments and strings included), which catches the
//     plain dotted spelling and prose that tells a reader to call a banned member;
//   - a structural pass through the TypeScript parser (`ts.createSourceFile`) over `.ts`/`.js`/
//     `.mjs` files and the `<script>` blocks and template expressions of `.svelte` files, which
//     catches member access, optional chains, string-literal element access, line-broken access,
//     type-asserted owners (`(navigator as X).member`), window/self/globalThis-qualified owners,
//     and destructuring of a banned member (`const { member } = owner`, assignment patterns,
//     including nested `{ navigator: { member } } = window` in both binding and assignment form,
//     and, for a rule that names no owner, members nested inside object and array patterns).
//     Svelte is covered because ESLint ignores `.svelte`; that is why this is a script gate rather
//     than an ESLint rule. A file, `<script>` block or template fragment the parser cannot read
//     fails the gate: an unread region is never a silent pass.
//
// BOUND (what no static pass can decide): an access through an alias (`const nav = navigator;
// nav.mediaDevices`, or a typed parameter `(n: Navigator) => n.mediaDevices`), a non-literal key
// (`navigator[name]`), `Reflect.get(navigator, "member")`, and the `"member" in navigator` test.
// Non-literal keys on the `navigator`/`crypto` owners are counted and listed in the gate's output;
// the other forms are not tracked. An owner-qualified member (`navigator.mediaDevices`) destructured
// out of an array element or a default-value expression (`const [{ mediaDevices }] = xs`) names no
// owner the gate can resolve, so only owner-less rules (`audioWorklet`) see through those.
//
// A `globals` rule (`caches`) matches a bare identifier only when the file declares no VALUE binding
// of that name. The declared set is file-wide (every `<script>` block and the template) and counts
// value declarations only: variables, value parameters, functions, classes and non-type imports. A
// parameter of a function TYPE, a type-only import, and an interface or type alias declare no value
// and do not shadow the global. The runtime cover is the `lan-plain-http` e2e project: it drives
// its flows on a real non-loopback origin where every member below is genuinely absent, fails on
// any page error, and its "measured" test asserts each member is `undefined` there. That covers the
// flows it drives, not the whole client.
//
// Each rule is one banned member (or member set), the owner it hangs off, the file(s) allowed to
// mention it, and the seam that replaces it. `navigator.mediaDevices` and `AudioContext.audioWorklet`
// are the microphone source's members; their seam is `micVad.ts`, whose `isAvailable` feature check
// guards every access. `caches` (CacheStorage) is `[SecureContext]` and unused by the client.
//
// Adding a rule: name the owner and members, name the replacing seam, add a positive control to
// check-secure-context-apis.test.mjs.

import { readFileSync } from "node:fs";
import process from "node:process";
import ts from "typescript";
import { isDirectEntry } from "./lib/is-main.mjs";
import { CLIENT_ROOTS, norm, repoFiles, under } from "./lib/gate-corpus.mjs";

// Direction: a BAN gate. The corpus is `repoFiles` (tracked plus untracked, non-ignored files,
// minus files deleted in the working tree), so a new file is checked before `git add`.

/** The file that defines the seam; the scan must include it or the gate is inert. */
export const SEAM_FILE = "src/client/core/src/uuid.ts";

/** The HTTPS secure-context e2e test file: the only e2e file that names the UUID member, to assert it
 * is a function on the HTTPS origin. Allowed for the `randomUUID` rule only. */
export const HTTPS_SPEC_FILE = "src/client/shell/e2e/https-secure-context.spec.ts";

/** The microphone source: the only file that names `navigator.mediaDevices` and `audioWorklet`. */
export const MIC_SEAM_FILE = "src/modules/ducking/src/micVad.ts";

/** Members whose banned status needs no seam: none of them has any use in the client, and a
 * future use must go through a feature-detected seam added together with its rule change. */
const UNUSED_MEMBER_REPLACEMENT = "a feature-detected seam (the member is undefined on plain-HTTP LAN origins); extend this rule with the seam when a use is needed";

/** The microphone seam's replacement text, shared by its two rules. */
const MIC_REPLACEMENT = "the microphone seam `MicVadDeps` in `micVad.ts` (`isAvailable` feature-detects the member before any use)";

/** Owner sentinel: the global object itself (`window`, `self`, `globalThis`). */
const GLOBAL_OBJECT = "@global";

/** Identifiers that name the global object. */
const GLOBAL_OBJECT_NAMES = new Set(["window", "self", "globalThis"]);

/** The banned members. `owner`: an identifier hanging the members off the global scope
 * (`navigator`, `crypto`), `GLOBAL_OBJECT`, or `null` for any object. `bare`: the members are
 * banned as a bare identifier anywhere. `globals`: bare identifiers that are the member itself
 * when the file does not declare them. `allowedFiles` are the only files that may contain them. */
export const RULES = [
  {
    id: "randomUUID",
    owner: null,
    members: ["randomUUID"],
    bare: true,
    // `uuid.ts` defines the seam, `uuid.test.ts` pins its fallback, and `HTTPS_SPEC_FILE` asserts
    // the member exists on the HTTPS origin.
    allowedFiles: [SEAM_FILE, "src/client/core/src/uuid.test.ts", HTTPS_SPEC_FILE],
    replacement: "`randomUuid()` from `@shadowcat/core` (falls back to `crypto.getRandomValues` where `crypto.randomUUID` is absent)",
  },
  { id: "crypto.subtle", owner: "crypto", members: ["subtle"], allowedFiles: [], replacement: UNUSED_MEMBER_REPLACEMENT },
  { id: "navigator.clipboard", owner: "navigator", members: ["clipboard"], allowedFiles: [], replacement: UNUSED_MEMBER_REPLACEMENT },
  { id: "navigator.serviceWorker", owner: "navigator", members: ["serviceWorker"], allowedFiles: [], replacement: UNUSED_MEMBER_REPLACEMENT },
  { id: "navigator.storage", owner: "navigator", members: ["storage"], allowedFiles: [], replacement: UNUSED_MEMBER_REPLACEMENT },
  { id: "navigator.locks", owner: "navigator", members: ["locks"], allowedFiles: [], replacement: UNUSED_MEMBER_REPLACEMENT },
  { id: "navigator.credentials", owner: "navigator", members: ["credentials"], allowedFiles: [], replacement: UNUSED_MEMBER_REPLACEMENT },
  { id: "navigator.share", owner: "navigator", members: ["share", "canShare"], allowedFiles: [], replacement: UNUSED_MEMBER_REPLACEMENT },
  { id: "navigator.gpu", owner: "navigator", members: ["gpu"], allowedFiles: [], replacement: UNUSED_MEMBER_REPLACEMENT },
  {
    id: "caches",
    owner: GLOBAL_OBJECT,
    members: ["caches"],
    globals: ["caches"],
    allowedFiles: [],
    replacement: UNUSED_MEMBER_REPLACEMENT,
  },
  {
    id: "navigator.mediaDevices",
    owner: "navigator",
    members: ["mediaDevices"],
    // `micVad.ts` is the microphone seam: it reaches the member only behind `MicVadDeps.isAvailable`.
    allowedFiles: [MIC_SEAM_FILE],
    replacement: MIC_REPLACEMENT,
  },
  {
    id: "AudioContext.audioWorklet",
    owner: null,
    members: ["audioWorklet"],
    allowedFiles: [MIC_SEAM_FILE],
    replacement: MIC_REPLACEMENT,
  },
];

const EXTS = [".ts", ".svelte", ".js", ".mjs"];
const ROOTS = CLIENT_ROOTS;

/** Whether `path` is a client source file the gate covers. */
export function isCovered(path) {
  const p = norm(path);
  return ROOTS.some((r) => under(p, r)) && EXTS.some((e) => p.endsWith(e)) && !under(p, "src/types/generated") && !under(p, "src/server");
}

/** The raw-text pattern of a rule: the dotted spelling `owner.member` (any object for a null
 * owner), or the bare word for a `bare` rule. */
function textPattern(rule) {
  const alt = rule.members.join("|");
  if (rule.bare) return new RegExp(`\\b(?:${alt})\\b`);
  if (rule.owner === null) return new RegExp(`\\.(?:${alt})\\b`);
  if (rule.owner === GLOBAL_OBJECT) return new RegExp(`\\b(?:window|self|globalThis)\\.(?:${alt})\\b`);
  return new RegExp(`\\b${rule.owner}\\.(?:${alt})\\b`);
}

/** Strips wrappers that do not change which value an expression denotes. */
function unwrap(expr) {
  let e = expr;
  for (;;) {
    if (ts.isParenthesizedExpression(e) || ts.isNonNullExpression(e) || ts.isAsExpression(e) || ts.isTypeAssertionExpression(e) || ts.isSatisfiesExpression(e)) e = e.expression;
    else return e;
  }
}

/** The string a property name or a literal element-access key spells, else `null`. */
function literalKey(node) {
  if (ts.isIdentifier(node) || ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node) || ts.isPrivateIdentifier(node)) return node.text;
  return null;
}

/** The string an element-access key spells when it is a string literal, else `null`: an
 * identifier key (`navigator[k]`) names a variable, not a member.
 * @param node The `argumentExpression` of an element access.
 */
function elementKey(node) {
  return ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node) ? node.text : null;
}

/** Whether `expr` is the global object: `window`, `self` or `globalThis`. */
function isGlobalObject(expr) {
  const e = unwrap(expr);
  return ts.isIdentifier(e) && GLOBAL_OBJECT_NAMES.has(e.text);
}

/** Whether `expr` denotes the rule's owner: the bare owner identifier, or the owner read off the
 * global object (`window.navigator`, `globalThis["crypto"]`). A null owner matches any expression;
 * `GLOBAL_OBJECT` matches only the global object. */
function isOwner(expr, owner) {
  if (owner === null) return true;
  const e = unwrap(expr);
  if (owner === GLOBAL_OBJECT) return isGlobalObject(e);
  if (ts.isIdentifier(e)) return e.text === owner;
  if (ts.isPropertyAccessExpression(e)) return e.name.text === owner && isGlobalObject(e.expression);
  if (ts.isElementAccessExpression(e)) return elementKey(e.argumentExpression) === owner && isGlobalObject(e.expression);
  return false;
}

/** Whether a parameter belongs to a TYPE (a function type, constructor type or signature) and so
 * binds no value. */
function isTypeLevelParameter(node) {
  const p = node.parent;
  return !!p && (ts.isFunctionTypeNode(p) || ts.isConstructorTypeNode(p) || ts.isCallSignatureDeclaration(p) || ts.isConstructSignatureDeclaration(p) || ts.isMethodSignature(p) || ts.isIndexSignatureDeclaration(p));
}

/** Names the file itself binds as VALUES: variables, value parameters, functions, classes and
 * non-type imports. A `globals` rule matches a bare identifier only when the file does not declare
 * that name, so a local variable called `caches` is not the global; a type-level parameter or a
 * type-only import binds no value and does not shadow it. */
function declaredNames(sf, names = new Set()) {
  const addName = (n) => {
    if (ts.isIdentifier(n)) names.add(n.text);
    else if (ts.isObjectBindingPattern(n) || ts.isArrayBindingPattern(n)) for (const el of n.elements) if (ts.isBindingElement(el)) addName(el.name);
  };
  const visit = (node) => {
    if (ts.isParameter(node) && isTypeLevelParameter(node)) {
      // A parameter of a function type declares no value; its own type annotation is still visited.
    } else if ((ts.isVariableDeclaration(node) || ts.isParameter(node) || ts.isBindingElement(node)) && node.name) addName(node.name);
    else if ((ts.isFunctionDeclaration(node) || ts.isClassDeclaration(node)) && node.name) names.add(node.name.text);
    else if (ts.isImportSpecifier(node)) {
      if (!node.isTypeOnly && !node.parent.parent.isTypeOnly) names.add(node.name.text);
    } else if (ts.isImportClause(node)) {
      if (!node.isTypeOnly && node.name) names.add(node.name.text);
    } else if (ts.isNamespaceImport(node)) {
      if (!node.parent.isTypeOnly) names.add(node.name.text);
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
  return names;
}

/** Names a Svelte template declares for its own scope: `{#each x as item, i}`, `{@const name = ...}`,
 * `{#await p then value}` / `{:catch error}` bindings, and `{#snippet name(params)}`. They shadow a
 * `globals` rule exactly as a script binding does.
 * @param text The whole `.svelte` source.
 * @returns The declared identifiers.
 */
function templateDeclaredNames(text) {
  const names = new Set();
  const words = (tail) => {
    for (const m of tail.matchAll(/[A-Za-z_$][\w$]*/g)) names.add(m[0]);
  };
  for (const m of text.matchAll(/\{#each\s[^}]*?\sas\s+([^}(]*)/g)) words(m[1]);
  for (const m of text.matchAll(/\{@const\s+([A-Za-z_$][\w$]*)/g)) names.add(m[1]);
  for (const m of text.matchAll(/\{#snippet\s+([A-Za-z_$][\w$]*)\s*\(([^)]*)\)/g)) {
    names.add(m[1]);
    words(m[2]);
  }
  for (const m of text.matchAll(/\{(?:#await\s[^}]*?\sthen|:then|:catch)\s+([^}]*)/g)) words(m[1]);
  return names;
}

/** Whether identifier `node` is in a position that reads a binding (not a property name, a
 * declaration name, or a type position). */
function isReadPosition(node) {
  const p = node.parent;
  if (!p) return true;
  if (ts.isPropertyAccessExpression(p) && p.name === node) return false;
  if (ts.isQualifiedName(p)) return false;
  if ((ts.isPropertyAssignment(p) || ts.isPropertySignature(p) || ts.isPropertyDeclaration(p) || ts.isMethodDeclaration(p) || ts.isMethodSignature(p) || ts.isEnumMember(p)) && p.name === node) return false;
  if ((ts.isVariableDeclaration(p) || ts.isParameter(p) || ts.isBindingElement(p) || ts.isFunctionDeclaration(p) || ts.isClassDeclaration(p) || ts.isImportSpecifier(p) || ts.isImportClause(p)) && (p.name === node || (ts.isBindingElement(p) && p.propertyName === node))) return false;
  if (ts.isTypeReferenceNode(p) || ts.isTypeQueryNode(p)) return false;
  return true;
}

/** Adds each banned-member hit under `pattern` (a binding or assignment pattern) whose source is
 * `source`; `nestedOwner` handles `{ navigator: { member } } = window`. */
function scanPattern(pattern, source, rule, hit) {
  if (ts.isArrayBindingPattern(pattern)) {
    // An array element names no owner, so only an owner-less rule can see a member nested in one.
    if (rule.owner === null) for (const el of pattern.elements) if (ts.isBindingElement(el) && (ts.isObjectBindingPattern(el.name) || ts.isArrayBindingPattern(el.name))) scanPattern(el.name, null, rule, hit);
    return;
  }
  const elements = ts.isObjectBindingPattern(pattern) ? pattern.elements : null;
  if (elements) {
    for (const el of elements) {
      const key = el.propertyName ? literalKey(el.propertyName) : ts.isIdentifier(el.name) ? el.name.text : null;
      if (key !== null && rule.members.includes(key) && (source === null ? rule.owner === null : isOwner(source, rule.owner))) hit(el);
      // `{ navigator: { member } } = window`: the key names the owner, the nested pattern holds the members.
      if (key !== null && ts.isObjectBindingPattern(el.name) && source !== null && isGlobalObject(source) && rule.owner === key) {
        scanPattern(el.name, unwrapAsSource(rule.owner), rule, hit);
      } else if ((ts.isObjectBindingPattern(el.name) || ts.isArrayBindingPattern(el.name)) && rule.owner === null) {
        scanPattern(el.name, null, rule, hit);
      }
    }
  }
}

/** A synthetic identifier node standing for the owner, so nested patterns reuse `isOwner`. */
function unwrapAsSource(ownerName) {
  return ts.factory.createIdentifier(ownerName);
}

/** Banned-member hits in an assignment pattern `({ member } = source)`: the object literal's keys
 * are the destructured names. Nested patterns recurse: `({ navigator: { member } } = window)` names
 * the owner in the outer key when `source` is the global object, and a null-owner rule looks
 * through every nesting level.
 * @param pattern The object literal on the left of `=`.
 * @param source The expression on the right of `=`, or `null` for a nested level of a null-owner rule.
 * @param rule The rule being scanned.
 * @param hit Records a hit at a node.
 */
function scanAssignmentPattern(pattern, source, rule, hit) {
  for (const prop of pattern.properties) {
    const key = ts.isShorthandPropertyAssignment(prop) ? prop.name.text : ts.isPropertyAssignment(prop) ? literalKey(prop.name) : null;
    if (key !== null && rule.members.includes(key) && (source === null ? rule.owner === null : isOwner(source, rule.owner))) hit(prop);
    if (ts.isPropertyAssignment(prop) && ts.isObjectLiteralExpression(prop.initializer)) {
      if (rule.owner === null) scanAssignmentPattern(prop.initializer, null, rule, hit);
      else if (key === rule.owner && source !== null && isGlobalObject(source)) scanAssignmentPattern(prop.initializer, unwrapAsSource(rule.owner), rule, hit);
    } else if (rule.owner === null && ts.isPropertyAssignment(prop) && ts.isArrayLiteralExpression(prop.initializer)) {
      scanAssignmentArray(prop.initializer, rule, hit);
    }
  }
}

/** Banned-member hits in an array assignment pattern `[{ member }] = xs`: an array element names
 * no owner, so only an owner-less rule looks through it, at every nesting of arrays and objects.
 * @param pattern The array literal on the left of `=`, or nested in an assignment pattern.
 * @param rule The rule being scanned.
 * @param hit Records a hit at a node.
 */
function scanAssignmentArray(pattern, rule, hit) {
  for (const el of pattern.elements) {
    if (ts.isObjectLiteralExpression(el)) scanAssignmentPattern(el, null, rule, hit);
    else if (ts.isArrayLiteralExpression(el)) scanAssignmentArray(el, rule, hit);
  }
}

/** Every structural hit of `rule` in the parsed source: `{ line }` (1-based, in `sf`'s own text). */
function structuralHits(sf, rule, declared) {
  const out = [];
  const lineOf = (node) => sf.getLineAndCharacterOfPosition(node.getStart(sf)).line + 1;
  const hit = (node) => out.push({ line: lineOf(node) });
  const visit = (node) => {
    if (ts.isPropertyAccessExpression(node)) {
      if (rule.members.includes(node.name.text) && isOwner(node.expression, rule.owner)) hit(node.name);
    } else if (ts.isElementAccessExpression(node)) {
      const key = elementKey(node.argumentExpression);
      if (key !== null && rule.members.includes(key) && isOwner(node.expression, rule.owner)) hit(node.argumentExpression);
    } else if (ts.isVariableDeclaration(node) || ts.isParameter(node)) {
      if (ts.isObjectBindingPattern(node.name) || (rule.owner === null && ts.isArrayBindingPattern(node.name))) {
        // A null-owner rule flags the member in any destructuring; an owned rule needs the owner as
        // the initializer.
        const source = node.initializer ?? null;
        if (rule.owner === null || source !== null) scanPattern(node.name, rule.owner === null ? null : source, rule, hit);
      }
    } else if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.EqualsToken && ts.isObjectLiteralExpression(node.left)) {
      scanAssignmentPattern(node.left, node.right, rule, hit);
    } else if (rule.owner === null && ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.EqualsToken && ts.isArrayLiteralExpression(node.left)) {
      scanAssignmentArray(node.left, rule, hit);
    } else if (rule.globals && ts.isIdentifier(node) && rule.globals.includes(node.text) && !declared.has(node.text) && isReadPosition(node)) {
      hit(node);
    } else if (rule.bare && ts.isIdentifier(node) && rule.members.includes(node.text)) {
      hit(node);
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
  return out;
}

/** Non-literal element-access keys on the `navigator`/`crypto` owners: undecidable statically. */
function dynamicKeyHits(sf) {
  const out = [];
  const visit = (node) => {
    if (ts.isElementAccessExpression(node) && elementKey(node.argumentExpression) === null && (isOwner(node.expression, "navigator") || isOwner(node.expression, "crypto"))) {
      out.push({ line: sf.getLineAndCharacterOfPosition(node.getStart(sf)).line + 1 });
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
  return out;
}

/** Replaces every character except newlines with a space, keeping line numbers stable. */
function blank(text) {
  return text.replace(/[^\n]/g, " ");
}

/** The parseable regions of a `.svelte` file as `{ code, lineOffset }`: each `<script>` block, and
 * each template expression (`{...}` mustaches, blocks and directives) with the script and style
 * blocks blanked out first. `unparsedLines` are the lines of template fragments that are not valid
 * TypeScript after stripping block keywords; the raw-text pass alone covers those, so the gate
 * treats each as an error rather than passing it unverified. */
export function svelteRegions(text) {
  const regions = [];
  let template = text;
  for (const m of text.matchAll(/<(script|style)\b[^>]*>([\s\S]*?)<\/\1>/g)) {
    if (m[1] === "script") {
      const bodyStart = m.index + m[0].indexOf(">") + 1;
      regions.push({ code: m[2], lineOffset: text.slice(0, bodyStart).split("\n").length - 1, kind: "script" });
    }
    template = template.slice(0, m.index) + blank(m[0]) + template.slice(m.index + m[0].length);
  }
  // HTML comments hold prose, not expressions; the raw-text pass still reads them.
  template = template.replace(/<!--[\s\S]*?-->/g, blank);
  const unparsedLines = [];
  let i = 0;
  while (i < template.length) {
    if (template[i] !== "{") {
      i++;
      continue;
    }
    // String-aware brace matching.
    let depth = 0;
    let j = i;
    let quote = null;
    for (; j < template.length; j++) {
      const c = template[j];
      if (quote) {
        if (c === "\\") j++;
        else if (c === quote) quote = null;
      } else if (c === '"' || c === "'" || c === "`") quote = c;
      else if (c === "{") depth++;
      else if (c === "}" && --depth === 0) break;
    }
    if (j >= template.length) break;
    const inner = template.slice(i + 1, j);
    const lineOffset = template.slice(0, i + 1).split("\n").length - 1;
    const expr = templateExpression(inner);
    if (expr !== null) {
      const sf = ts.createSourceFile("t.ts", `(${expr})`, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
      const errors = /** @type {{ parseDiagnostics?: unknown[] }} */ (sf).parseDiagnostics ?? [];
      if (errors.length > 0) unparsedLines.push(lineOffset + 1);
      else regions.push({ code: `(${expr})`, lineOffset, kind: "template" });
    }
    i = j + 1;
  }
  return { regions, unparsedLines };
}

/** The TypeScript expression inside a Svelte mustache, or `null` when it carries none
 * (`{/if}`, `{:else}`, a `{#snippet}` parameter list). Block openers keep their expression and drop
 * the binding tail; only `{#each}` and `{#await}` cut at their own keywords, so a `as` cast inside
 * an ordinary expression survives.
 * @param inner The text between the braces.
 * @returns The expression source, or `null`.
 */
function templateExpression(inner) {
  let e = inner.trim();
  if (e === "" || e.startsWith("/") || e === ":else" || e.startsWith("#snippet") || e.startsWith(":then") || e.startsWith(":catch")) return null;
  const each = /^#each\s+/.exec(e);
  if (each) e = e.slice(each[0].length).replace(/\s+as\s+[\s\S]*$/, "");
  const wait = /^#await\s+/.exec(e);
  if (wait) e = e.slice(wait[0].length).replace(/\s+(?:then|catch)\b[\s\S]*$/, "");
  e = e.replace(/^(?:#if|#key|:else if|@html|@render|@const|@debug|@attach|\.\.\.)\s*/, "");
  return e.length > 0 ? e : null;
}

/** Every rule violation in `text` of file `path`, as `{ line, rule }` (sorted by line, one entry
 * per rule per line), plus `dynamic`: line numbers of non-literal `navigator[...]`/`crypto[...]`
 * accesses and `unparsedLines`: lines of files, `<script>` blocks and template fragments the parser
 * could not read. */
export function scanSecureContextDetailed(path, text) {
  const p = norm(path);
  const active = RULES.filter((r) => !r.allowedFiles.includes(p));
  const found = new Map();
  const add = (line, rule) => found.set(`${rule.id}:${line}`, { line, rule });
  const lines = text.split("\n");
  for (const rule of active) {
    const pattern = textPattern(rule);
    lines.forEach((l, i) => {
      if (pattern.test(l)) add(i + 1, rule);
    });
  }
  const dynamic = [];
  let unparsedLines = [];
  const parseBlocks = [];
  if (p.endsWith(".svelte")) {
    const s = svelteRegions(text);
    unparsedLines = s.unparsedLines;
    parseBlocks.push(...s.regions);
  } else {
    parseBlocks.push({ code: text, lineOffset: 0, kind: "file" });
  }
  const parsed = parseBlocks.map((b) => ({ b, sf: ts.createSourceFile(p.endsWith(".svelte") ? "b.ts" : p, b.code, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS) }));
  // Fail closed: a file or `<script>` block with syntax errors is an unread region. Template
  // fragments were already checked by `svelteRegions`, which drops the unreadable ones.
  for (const { b, sf } of parsed) {
    if (b.kind === "template") continue;
    const diagnostics = /** @type {{ parseDiagnostics?: { start?: number }[] }} */ (sf).parseDiagnostics ?? [];
    if (diagnostics.length > 0) unparsedLines.push((sf.getLineAndCharacterOfPosition(diagnostics[0].start ?? 0).line + 1) + b.lineOffset);
  }
  // Declared names are file-wide: a `let caches` in one `<script>` shadows the global in the
  // template and in the module/instance script pair alike.
  const declared = new Set(p.endsWith(".svelte") ? templateDeclaredNames(text) : []);
  for (const { sf } of parsed) declaredNames(sf, declared);
  for (const { b, sf } of parsed) {
    for (const rule of active) for (const h of structuralHits(sf, rule, declared)) add(h.line + b.lineOffset, rule);
    for (const h of dynamicKeyHits(sf)) dynamic.push(h.line + b.lineOffset);
  }
  const violations = [...found.values()].sort((a, b) => a.line - b.line || a.rule.id.localeCompare(b.rule.id));
  return { violations, dynamic, unparsedLines };
}

/** Every rule violation in `text` of file `path`, as `{ line, rule }`. */
export function scanSecureContext(path, text) {
  return scanSecureContextDetailed(path, text).violations;
}

/** Throws when the scanned file set cannot support a clean verdict: empty, missing the seam file
 * (a scan that does not reach `uuid.ts` cannot be trusted to have reached anything), or holding no
 * `.svelte` file (the Svelte path would then be unexercised).
 * @param files The repo-relative paths the gate is about to scan.
 */
export function assertGateLive(files) {
  if (files.length === 0) throw new Error("lint:secure-context scanned 0 files; the gate is inert");
  const normalized = files.map(norm);
  if (!normalized.includes(SEAM_FILE)) throw new Error(`lint:secure-context did not scan ${SEAM_FILE}; the gate is inert`);
  if (!normalized.some((f) => f.endsWith(".svelte"))) throw new Error("lint:secure-context scanned no .svelte file; the Svelte path is inert");
}

/** The distinct files allowed to mention any banned member. */
export function allowedFileSet() {
  return new Set(RULES.flatMap((r) => r.allowedFiles));
}

/** Lists the corpus files under the gate's roots.
 * @returns Repo-relative paths; throws when git cannot list them.
 */
function listCorpusFiles() {
  return repoFiles({ roots: ROOTS, label: "lint:secure-context" });
}

/** Runs the gate over the files `listFiles` reports and returns the process exit code.
 * @param io Injected effects: `listFiles` returns repo-relative paths (and throws when the tree
 * cannot be listed), `readFile` reads a path, `log`/`error` receive output lines.
 * @returns 0 when clean, 1 on a violation, an unlistable tree, or a parse-blind template fragment.
 */
export function runGate({ listFiles, readFile, log, error }) {
  let listed;
  try {
    listed = listFiles();
  } catch (e) {
    error(`lint:secure-context: ${e.message}`);
    return 1;
  }
  const files = listed.filter((f) => f && isCovered(f));
  assertGateLive(files);
  let errors = 0;
  const unparsed = [];
  const dynamic = [];
  for (const path of files) {
    const r = scanSecureContextDetailed(path, readFile(path));
    for (const line of r.unparsedLines) unparsed.push(`${norm(path)}:${line}`);
    for (const line of r.dynamic) dynamic.push(`${norm(path)}:${line}`);
    for (const v of r.violations) {
      errors++;
      error(`SECURE-CONTEXT-ONLY API: ${norm(path)}:${v.line}: \`${v.rule.id}\` is undefined on plain-HTTP LAN origins — use ${v.rule.replacement}`);
    }
  }
  for (const u of unparsed) {
    errors++;
    error(`lint:secure-context: cannot parse ${u}; the gate cannot verify an unread region — fix the syntax error or rewrite the expression as valid TypeScript`);
  }
  for (const d of dynamic) log(`lint:secure-context: non-literal navigator/crypto key (undecidable statically; covered at runtime by the lan-plain-http e2e project): ${d}`);
  log(`lint:secure-context: ${files.length} files scanned, ${RULES.length} rule(s), ${allowedFileSet().size} distinct allowed file(s), ${dynamic.length} non-literal key access(es), ${unparsed.length} unparsed region(s), ${errors} error(s)`);
  return errors === 0 ? 0 : 1;
}

if (isDirectEntry(import.meta.url)) {
  process.exit(runGate({ listFiles: listCorpusFiles, readFile: (p) => readFileSync(p, "utf8"), log: console.log, error: console.error }));
}
