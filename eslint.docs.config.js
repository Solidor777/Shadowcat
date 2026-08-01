// Doc-coverage lint (spec §2): every function — exported or not — carries a doc
// comment with description, params, and an @example. Warn-tier during the Phase-1
// ratchet; sweep plans flip per-package severity to error here, and the final
// phase merges these rules into eslint.config.js. Kept separate so `pnpm lint`
// stays warning-free until then.
import jsdoc from "eslint-plugin-jsdoc";
import tseslint from "typescript-eslint";
import svelteParser from "svelte-eslint-parser";

// One rule set, parameterized by severity, so a ratcheted package and a
// still-warning one can never drift in WHICH rules they enforce — only in how
// loudly. Adding a rule here applies it at both tiers by construction.
const rulesAt = (sev) => ({
  "jsdoc/require-jsdoc": [sev, {
    require: {
      FunctionDeclaration: true,
      MethodDefinition: true,
      ClassDeclaration: true,
      ArrowFunctionExpression: false,
      FunctionExpression: false,
    },
    // Arrow/function expressions only when they are named exports or class fields
    // would over-fire on inline callbacks; declarations and methods are the
    // "every function" surface the spec enforces mechanically. Inline callbacks
    // are covered by their enclosing declaration's docs.
  }],
  "jsdoc/require-description": sev,
  "jsdoc/require-param": sev,
  "jsdoc/require-param-description": sev,
  "jsdoc/require-returns": sev,
  "jsdoc/require-example": [sev, { exemptNoArguments: false }],
});

const RULES = rulesAt("warn");

// These rules gate on tag PRESENCE only. They cannot detect a tag whose text is
// vacuous, nor a second doc block appended below an existing one — jsdoc,
// TypeDoc, and editor hover all bind to the NEAREST preceding block, so an
// appended block satisfies the linter while orphaning the richer one above it.
// A clean run here is evidence the tags exist, not that the docs are correct.
const RULES_RATCHETED = rulesAt("error");

export default [
  {
    files: ["src/types/**/*.ts", "src/client/**/*.ts", "src/modules/**/*.ts", "examples/**/*.ts"],
    ignores: [
      "**/node_modules/**", "**/dist/**", "**/*.test.ts", "**/vitest.setup.ts",
      // Generated: doc comments originate in the Rust source types (ts-rs).
      "src/types/generated/**",
    ],
    languageOptions: { parser: tseslint.parser },
    // typescript-eslint is registered (no rules enabled) so source files' inline
    // eslint-disable directives naming its rules resolve under this config too.
    plugins: { jsdoc, "@typescript-eslint": tseslint.plugin },
    rules: RULES,
  },
  // Ratcheted packages: doc coverage is a hard gate, not advisory. A package
  // joins this list only once it is at zero under the warn tier.
  {
    files: ["src/client/core/**/*.ts", "src/client/render/**/*.ts"],
    // Kept identical to the warn block's ignores, including `src/types/generated`
    // (inert against today's `files` glob). The two blocks must stay symmetric:
    // the next package added here inherits whatever asymmetry is left behind.
    ignores: [
      "**/node_modules/**", "**/dist/**", "**/*.test.ts", "**/vitest.setup.ts",
      "src/types/generated/**",
    ],
    languageOptions: { parser: tseslint.parser },
    plugins: { jsdoc, "@typescript-eslint": tseslint.plugin },
    rules: RULES_RATCHETED,
  },
  // A registered-but-ruleless plugin makes any inline disable naming its rules
  // dead FROM THIS CONFIG's perspective, even though eslint.config.js genuinely
  // needs it (tseslint.configs.recommended enables the rule there). Scoped to
  // the single file that carries such a directive rather than the whole tree, so
  // a genuinely dead directive anywhere else still reports.
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
];
