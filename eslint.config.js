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
  // Allow _-prefixed identifiers to signal intentionally unused parameters/variables.
  ...tseslint.config({
    files: ["**/*.ts"],
    rules: {
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_", varsIgnorePattern: "^_" }],
    },
  }),
];
