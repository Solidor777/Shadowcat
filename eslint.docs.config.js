// Doc-coverage lint: every function — exported or not — carries a doc comment with description,
// params, and an @example.
//
// Every rule is an error. There is no advisory tier and no per-package staging: a warning is a
// violation that ships, and a severity ladder grandfathers whatever existed when the ladder was
// built — indistinguishable, to every later reader, from code that was checked and passed.
//
// A separate ESLint invocation from eslint.props.config.js because both configs set the same rule
// KEYS with different `contexts` lists, and flat config resolves a key to the last block that sets
// it. Merged, one config's context list would silently replace the other's.
import jsdoc from "eslint-plugin-jsdoc";
import tseslint from "typescript-eslint";
import svelteParser from "svelte-eslint-parser";

const RULES = {
  "jsdoc/require-jsdoc": ["error", {
    require: {
      FunctionDeclaration: true,
      MethodDefinition: true,
      ClassDeclaration: true,
      ArrowFunctionExpression: false,
      FunctionExpression: false,
    },
    // Arrow/function expressions only when they are named exports or class fields
    // would over-fire on inline callbacks; declarations and methods are the
    // "every function" surface this config enforces mechanically. Inline callbacks
    // are covered by their enclosing declaration's docs.
  }],
  "jsdoc/require-description": "error",
  "jsdoc/require-param": "error",
  "jsdoc/require-param-description": "error",
  "jsdoc/require-returns": "error",
  "jsdoc/require-example": ["error", { exemptNoArguments: false }],
};

// These rules gate on tag PRESENCE only. They cannot detect a tag whose text is
// vacuous, nor a second doc block appended below an existing one — jsdoc,
// TypeDoc, and editor hover all bind to the NEAREST preceding block, so an
// appended block satisfies the linter while orphaning the richer one above it.
// A clean run here is evidence the tags exist, not that the docs are correct.

export default [
  {
    files: ["src/types/**/*.ts", "src/client/**/*.ts", "src/modules/**/*.ts", "examples/**/*.ts"],
    ignores: [
      // `*.test.ts` (vitest) and `*.spec.ts` (Playwright) name the SAME category —
      // a test file, whose local helpers are described by the test that uses them.
      // Both conventions must appear or the exemption depends on which runner owns
      // the file: `nodeConnect` in core's `capabilities.e2e.test.ts` is exempt while
      // an identical `login` in shell's `world-delete.spec.ts` is not. A test HELPER
      // MODULE that is not itself a test file (core's `e2e/server-process.ts`) stays
      // covered and is documented like any other module.
      "**/node_modules/**", "**/dist/**", "**/*.test.ts", "**/*.spec.ts", "**/vitest.setup.ts",
      // Generated: doc comments originate in the Rust source types (ts-rs).
      "src/types/generated/**",
    ],
    languageOptions: { parser: tseslint.parser },
    // typescript-eslint is registered (no rules enabled) so source files' inline
    // eslint-disable directives naming its rules resolve under this config too.
    plugins: { jsdoc, "@typescript-eslint": tseslint.plugin },
    rules: RULES,
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
  // Separate from the `.ts` block because a `.svelte` file needs svelteParser and one block cannot
  // carry both parsers. The rule set is the same object, and it does reach functions declared in a
  // `<script>` block — an undocumented function added to a component reports, so a green run here
  // is a real zero rather than a parser that silently visits nothing.
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
