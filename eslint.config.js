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
    ignores: ["dist/", "node_modules/", "target/", "**/*.svelte", "src/types/generated/", ".claude/worktrees/"],
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
  // Allow _-prefixed identifiers to signal intentionally unused parameters/variables.
  ...tseslint.config({
    files: ["**/*.ts"],
    rules: {
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_", varsIgnorePattern: "^_" }],
    },
  }),
];
