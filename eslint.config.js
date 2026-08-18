import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default [
  js.configs.recommended,
  ...tseslint.config({
    files: ["**/*.ts"],
    extends: [tseslint.configs.recommended],
  }),
  {
    // .claude/worktrees holds harness-created git worktrees whose own dist/
    // builds are not at the repo-root "dist/" path this list matches.
    // A pattern with no leading `**/` is anchored at the config's own directory, so `target/`
    // matches only a repo-root Cargo output directory and never `src/server/target/`, which is
    // where the Rust build actually writes — including the rustdoc `.js` the doc gate generates
    // under `--target-dir target/nightly-doc`. `**/target/` is what covers a Cargo output
    // directory at any depth.
    ignores: ["dist/", "node_modules/", "**/target/", "**/*.svelte", "src/types/generated/", ".claude/worktrees/",
      // Git-ignored plan workspace: briefs, reports, diffs and throwaway probe scripts, none of
      // which ship or are tracked. Linting it turns any scratch script into a repo-wide gate
      // failure that CI cannot reproduce, because CI never checks the directory out — a local-only
      // false failure trains readers to discount a real one.
      ".superpowers/",
      // Docs pipeline output: generated sites/scratch, never hand-written code.
      ".docs-tmp/", "dist-docs/", "docs/site/.vitepress/cache/", "docs/site/.vitepress/dist/",
      // Worked-example lib builds (the root "dist/" ignore is top-level only).
      "examples/*/dist/"],
  },
  // Import boundary: dockview-core is an implementation detail of the panels
  // engine adapter (EngineAdapter seam) — only engine/dockview.ts and its test
  // may import it. Svelte files are outside this net (unlinted above); the
  // boundary holds there by the seam's design (components consume EngineAdapter).
  {
    files: ["**/*.ts"],
    ignores: ["src/modules/panels/src/engine/dockview.ts", "src/modules/panels/src/engine/dockview.test.ts"],
    rules: {
      "no-restricted-imports": ["error", {
        paths: [{ name: "dockview-core", message: "dockview-core may only be imported by src/modules/panels/src/engine/dockview.ts (EngineAdapter boundary)." }],
      }],
    },
  },
  // Node-executed build scripts (not bundled, not typechecked) run outside the
  // browser global set js.configs.recommended assumes.
  {
    files: ["scripts/**/*.mjs"],
    languageOptions: {
      globals: { process: "readonly", console: "readonly" },
    },
  },
  // Allow _-prefixed identifiers to signal intentionally unused parameters/variables.
  ...tseslint.config({
    files: ["**/*.ts"],
    rules: {
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_", varsIgnorePattern: "^_" }],
    },
  }),
];
