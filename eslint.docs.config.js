// Doc-coverage lint (spec §2): every function — exported or not — carries a doc
// comment with description, params, and an @example. Warn-tier during the Phase-1
// ratchet; sweep plans flip per-package severity to error here, and the final
// phase merges these rules into eslint.config.js. Kept separate so `pnpm lint`
// stays warning-free until then.
import jsdoc from "eslint-plugin-jsdoc";
import tseslint from "typescript-eslint";
import svelteParser from "svelte-eslint-parser";

const RULES = {
  "jsdoc/require-jsdoc": ["warn", {
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
  "jsdoc/require-description": "warn",
  "jsdoc/require-param": "warn",
  "jsdoc/require-param-description": "warn",
  "jsdoc/require-returns": "warn",
  "jsdoc/require-example": ["warn", { exemptNoArguments: false }],
};

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
