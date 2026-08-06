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
  // Ratcheted, repo-wide (sweep 12): every package under these globs reached
  // zero under the warn tier, so the ratcheted block now takes the SAME globs
  // as the warn block above rather than an enumerated package list. Because
  // flat config gives the LATER block precedence per rule key, and this block
  // now has `files` byte-identical to the warn block above, this block fully
  // SHADOWS it: every rule key in `rulesAt` resolves to this block's `error`
  // severity here, for every matched file, unconditionally. The warn block is
  // not "empty" — it is superseded. It also cannot stage a future rule on its
  // own: `rulesAt` is one function feeding both tiers (`:10-12`), so adding a
  // context there sets both tiers' severity at once, and with the globs now
  // identical the ratcheted block's `error` is what every file sees. Sweep 13
  // stages new (property/type) contexts through a SEPARATE config file
  // instead — see `docs/superpowers/plans/2026-08-06-docs-sweep13-property-
  // coverage.md` — precisely because this shadowing makes in-place staging
  // impossible once a block's globs match its warn-tier sibling's.
  {
    files: ["src/types/**/*.ts", "src/client/**/*.ts", "src/modules/**/*.ts", "examples/**/*.ts"],
    // Kept identical to the warn block's ignores. `src/types/generated/**` is
    // load-bearing here (unlike when this block held an enumerated package
    // list): this block's `files` now includes `src/types/**/*.ts`, which DOES
    // match `src/types/generated/**` — removing this ignore would gate every
    // ts-rs-generated file at `error`. The two blocks must stay symmetric: the
    // next edit here inherits whatever asymmetry is left behind.
    ignores: [
      "**/node_modules/**", "**/dist/**", "**/*.test.ts", "**/*.spec.ts", "**/vitest.setup.ts",
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
  // Ratcheted .svelte, repo-wide (sweep 12). Separate from the ratcheted .ts
  // block because a `.svelte` file needs svelteParser — one block cannot carry
  // both parsers — so gating a package means adding it to BOTH blocks; here it
  // means both blocks take the same broad globs. The rules themselves are the
  // same `rulesAt` set, and they DO reach functions declared in a `<script>`
  // block (mutation-checked again this sweep: an undocumented function added to
  // a ratcheted component reports, so a green run here is a real zero rather
  // than a parser that silently visits nothing).
  {
    files: ["src/client/**/*.svelte", "src/modules/**/*.svelte", "examples/**/*.svelte"],
    ignores: ["**/node_modules/**", "**/dist/**"],
    languageOptions: {
      parser: svelteParser,
      parserOptions: { parser: tseslint.parser },
    },
    plugins: { jsdoc },
    rules: RULES_RATCHETED,
  },
];
