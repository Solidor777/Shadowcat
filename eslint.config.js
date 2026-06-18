import js from "@eslint/js";

export default [
  js.configs.recommended,
  {
    ignores: ["dist/", "node_modules/", "**/*.svelte", "src/types/generated/"],
  },
  {
    // Transitional browser-side auth pages served by the server bundle.
    files: ["src/server/static/**/*.js"],
    languageOptions: {
      globals: {
        fetch: "readonly",
        FormData: "readonly",
        document: "readonly",
        window: "readonly",
      },
    },
  },
];
