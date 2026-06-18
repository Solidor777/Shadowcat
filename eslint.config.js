import js from "@eslint/js";

export default [
  js.configs.recommended,
  {
    ignores: ["dist/", "node_modules/", "**/*.svelte", "src/types/generated/"],
  },
];
