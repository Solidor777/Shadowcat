import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default [
  js.configs.recommended,
  ...tseslint.config({
    files: ["**/*.ts"],
    extends: [tseslint.configs.recommended],
  }),
  {
    ignores: ["dist/", "node_modules/", "target/", "**/*.svelte", "src/types/generated/"],
  },
  // Allow _-prefixed identifiers to signal intentionally unused parameters/variables.
  ...tseslint.config({
    files: ["**/*.ts"],
    rules: {
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_", varsIgnorePattern: "^_" }],
    },
  }),
];
