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

// Every `require:` entry the function-doc gate already covers
// (FunctionDeclaration/MethodDefinition/ClassDeclaration/ArrowFunctionExpression/
// FunctionExpression) is explicitly `false` here — this config gates only the
// eleven contexts below, so the two configs never both assert requirements
// about the same construct.
const rulesAt = (sev) => ({
  "jsdoc/require-jsdoc": [sev, {
    require: {
      FunctionDeclaration: false,
      MethodDefinition: false,
      ClassDeclaration: false,
      ArrowFunctionExpression: false,
      FunctionExpression: false,
    },
    contexts: [
      // Properties (4): object/class fields, interface members, enum members.
      "PropertyDefinition",
      "TSPropertySignature",
      "TSMethodSignature",
      "TSEnumMember",
      // Type declarations (3).
      "TSInterfaceDeclaration",
      "TSTypeAliasDeclaration",
      "TSEnumDeclaration",
      // Named arrow/function expressions (4): deliberately narrow to exported
      // or module-level `const`/`let` bindings, never a bare
      // ArrowFunctionExpression/FunctionExpression selector — that would also
      // match an inline callback argument, which the function-doc gate's own
      // `ArrowFunctionExpression: false` rationale (over-firing on inline
      // callbacks) already rejects. These selectors cannot match an inline
      // callback because an argument position is never a VariableDeclarator.
      "ExportNamedDeclaration > VariableDeclaration > VariableDeclarator > ArrowFunctionExpression",
      "ExportNamedDeclaration > VariableDeclaration > VariableDeclarator > FunctionExpression",
      "Program > VariableDeclaration > VariableDeclarator > ArrowFunctionExpression",
      "Program > VariableDeclaration > VariableDeclarator > FunctionExpression",
    ],
  }],
  "jsdoc/require-description": sev,
  "jsdoc/require-param": sev,
  "jsdoc/require-param-description": sev,
  "jsdoc/require-returns": sev,
  "jsdoc/require-example": [sev, { exemptNoArguments: false }],
});

const RULES = rulesAt("warn");

// Same caveat as eslint.docs.config.js: these rules gate on tag PRESENCE
// only. They cannot detect a vacuous tag, an orphaned second block, or a
// property doc that restates the field name. With ~1329 property/type/arrow
// sites in scope, a restated-name doc is the dominant risk, not a footnote —
// a clean run here is evidence the tags exist, never that the docs are good.
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
  // warn tier above. Unlike eslint.docs.config.js's ratchet, this block's
  // `files` is NOT yet identical to the warn block's — that identity is
  // precisely what causes the shadowing this file exists to avoid, so this
  // list stays an enumerated subset until every package is proven clean, at
  // which point widening it to match the warn block becomes safe.
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
  // Ratcheted `.svelte`: same placeholder rationale as the `.ts` ratchet
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
