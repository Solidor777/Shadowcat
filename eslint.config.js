import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default [
  js.configs.recommended,
  ...tseslint.config({
    files: ["**/*.ts"],
    extends: [tseslint.configs.recommended],
  }),
  {
    ignores: ["dist/", "node_modules/", "**/*.svelte", "src/types/generated/"],
  },
];
