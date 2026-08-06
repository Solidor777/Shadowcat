// Property/type/named-arrow doc-coverage lint. A SEPARATE ESLint invocation
// from eslint.docs.config.js, not an extension of it: that file's ratcheted
// `.ts`/`.svelte` blocks carry `files` globs byte-identical to their
// warn-tier siblings, so flat config's later-block-wins-per-rule-key
// semantics make its warn tier fully SHADOWED — adding a new context to its
// shared `rulesAt` would land at `error`, repo-wide, immediately, with no way
// to stage a burn-down. This file exists so that shadowing cannot happen: its
// own `rulesAt`-shaped severity function feeds its own independent
// warn/ratcheted block pair, with no interaction with eslint.docs.config.js's
// rule keys at all.
// Plan: docs/superpowers/plans/2026-08-05-docs-sweep13-property-coverage.md
import jsdoc from "eslint-plugin-jsdoc";
import tseslint from "typescript-eslint";
import svelteParser from "svelte-eslint-parser";

// Properties (4): object/class fields, interface members, enum members.
const PROPERTY_CONTEXTS = [
  "PropertyDefinition",
  "TSPropertySignature",
  "TSMethodSignature",
  "TSEnumMember",
];
// Type declarations (3), plus an index signature (1) — declared interface
// contract shape, not a value-bearing property or a param/return-bearing
// signature, so it joins jsdoc/require-jsdoc's and require-description's
// context lists but not require-param/require-returns's.
const TYPE_CONTEXTS = [
  "TSInterfaceDeclaration",
  "TSTypeAliasDeclaration",
  "TSEnumDeclaration",
  "TSIndexSignature",
];
// Named arrow/function expressions (4): deliberately narrow to exported or
// module-level `const`/`let` bindings, never a bare
// ArrowFunctionExpression/FunctionExpression selector — that would also match
// an inline callback argument, which the function-doc gate's own
// `ArrowFunctionExpression: false` rationale (over-firing on inline
// callbacks) already rejects. These selectors cannot match an inline
// callback because an argument position is never a VariableDeclarator.
const ARROW_CONTEXTS = [
  "ExportNamedDeclaration > VariableDeclaration > VariableDeclarator > ArrowFunctionExpression",
  "ExportNamedDeclaration > VariableDeclaration > VariableDeclarator > FunctionExpression",
  "Program > VariableDeclaration > VariableDeclarator > ArrowFunctionExpression",
  "Program > VariableDeclaration > VariableDeclarator > FunctionExpression",
];
const ALL_CONTEXTS = [...PROPERTY_CONTEXTS, ...TYPE_CONTEXTS, ...ARROW_CONTEXTS];
// TSMethodSignature plus the four arrow/function-expression selectors: the
// only contexts among the twelve that have parameters or a return value to
// document. A plain property, an index signature, or an
// interface/type-alias/enum declaration or member has neither — attaching
// require-param/require-returns there would demand tags that describe
// nothing.
const PARAM_RETURN_CONTEXTS = ["TSMethodSignature", ...ARROW_CONTEXTS];

// Every `require:` entry the function-doc gate covers
// (FunctionDeclaration/ClassDeclaration/MethodDefinition) is explicitly
// `false` here — this config gates only the twelve contexts above, so the two
// configs never both assert requirements about the same construct. Declining
// `ArrowFunctionExpression`/`FunctionExpression` mirrors eslint.docs.config.js's
// own choice to leave them unrequired there too (`eslint.docs.config.js:19-20`)
// rather than duplicating a requirement that file already enforces — this
// config introduces no requirement on the bare selectors either way; its
// ARROW_CONTEXTS entries reach only the four narrow, named-binding paths.
const rulesAt = (sev) => ({
  "jsdoc/require-jsdoc": [sev, {
    require: {
      FunctionDeclaration: false,
      MethodDefinition: false,
      ClassDeclaration: false,
      ArrowFunctionExpression: false,
      FunctionExpression: false,
    },
    contexts: ALL_CONTEXTS,
  }],
  // `contexts` REPLACES the plugin's default list for a rule, not adds to it.
  // Its default (`ArrowFunctionExpression`/`FunctionDeclaration`/
  // `FunctionExpression`/`TSDeclareFunction`) is a function-shaped list that
  // would keep these three rules blind to all twelve contexts above if left
  // implicit — losing that default's function coverage inside THIS config is
  // fine, because eslint.docs.config.js already enforces it at `error`.
  "jsdoc/require-description": [sev, { contexts: ALL_CONTEXTS }],
  "jsdoc/require-param": [sev, { contexts: PARAM_RETURN_CONTEXTS }],
  "jsdoc/require-param-description": [sev, { contexts: PARAM_RETURN_CONTEXTS }],
  "jsdoc/require-returns": [sev, { contexts: PARAM_RETURN_CONTEXTS }],
  // Deliberately NOT extended to any of the twelve contexts: an `@example` on
  // each of ~1222 interface properties is noise, not documentation, and would
  // inflate `docs:check-examples`'s compiled-example count for no reader
  // value. Stays on the plugin's function-shaped default list only.
  "jsdoc/require-example": [sev, { exemptNoArguments: false }],
});

const RULES = rulesAt("warn");

// Same caveat as eslint.docs.config.js: these rules gate on tag PRESENCE
// only. They cannot detect a vacuous tag, an orphaned second block, or a
// property doc that restates the field name. With 1436 property/type/arrow
// warnings measured under this config (`pnpm lint:props`), a restated-name
// doc is the dominant risk, not a footnote — a clean run here is evidence
// the tags exist, never that the docs are good.
const RULES_RATCHETED = rulesAt("error");

export default [
  {
    files: ["src/types/**/*.ts", "src/client/**/*.ts", "src/modules/**/*.ts", "examples/**/*.ts"],
    ignores: [
      // Identical to eslint.docs.config.js's ignores — see that file's
      // comment for why both test-file conventions and the generated-types
      // exemption must all appear.
      "**/node_modules/**", "**/dist/**", "**/*.test.ts", "**/*.spec.ts", "**/vitest.setup.ts",
      "src/types/generated/**",
    ],
    languageOptions: { parser: tseslint.parser },
    plugins: { jsdoc, "@typescript-eslint": tseslint.plugin },
    rules: RULES,
  },
  // Ratcheted `.ts`: starts with a glob matching NO real file (flat config
  // rejects an empty `files` array outright). A later burn-down replaces this
  // placeholder with each package's real glob as it reaches zero under the
  // warn tier above. This block's `files` must stay an enumerated subset,
  // NEVER widened to match the warn block's broad glob — that byte-identical
  // widening is exactly what shadows eslint.docs.config.js's own warn tier
  // (see the top-of-file comment). This file's contexts are meant to end up
  // merged into eslint.docs.config.js and this file deleted, once every
  // package is clean — not to reach that same widened-glob state on its own.
  {
    files: ["__sweep13_ratchet_placeholder__/**/*.ts"],
    ignores: [
      "**/node_modules/**", "**/dist/**", "**/*.test.ts", "**/*.spec.ts", "**/vitest.setup.ts",
      "src/types/generated/**",
    ],
    languageOptions: { parser: tseslint.parser },
    plugins: { jsdoc, "@typescript-eslint": tseslint.plugin },
    rules: RULES_RATCHETED,
  },
  // Mirrors eslint.docs.config.js's hooks.ts exemption: a registered-but-
  // ruleless plugin makes an inline eslint-disable naming its rules dead
  // FROM THIS CONFIG's perspective even though eslint.config.js genuinely
  // needs it. Scoped to the single file that carries such a directive.
  {
    files: ["src/client/core/src/hooks.ts"],
    linterOptions: { reportUnusedDisableDirectives: false },
  },
  {
    files: ["src/client/**/*.svelte", "src/modules/**/*.svelte", "examples/**/*.svelte"],
    ignores: ["**/node_modules/**", "**/dist/**"],
    languageOptions: {
      parser: svelteParser,
      parserOptions: { parser: tseslint.parser },
    },
    plugins: { jsdoc },
    rules: RULES,
  },
  // Ratcheted `.svelte`: same placeholder rationale, and the same
  // never-widen-to-match-the-warn-glob constraint, as the `.ts` ratchet
  // above. A package with components needs BOTH this block and the `.ts`
  // ratchet updated together, or its components stay silently advisory.
  {
    files: ["__sweep13_ratchet_placeholder__/**/*.svelte"],
    ignores: ["**/node_modules/**", "**/dist/**"],
    languageOptions: {
      parser: svelteParser,
      parserOptions: { parser: tseslint.parser },
    },
    plugins: { jsdoc },
    rules: RULES_RATCHETED,
  },
];
