import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// External-module build: ES library with every engine package left external —
// the host (Shadowcat's shell) supplies exactly one instance of each at runtime.
export default defineConfig({
  plugins: [svelte()],
  build: {
    lib: {
      entry: "src/index.ts",
      formats: ["es"],
      fileName: () => "index.js",
      // Pin the stylesheet name the manifest declares (`"style": "style.css"`),
      // instead of Vite's package-name default.
      cssFileName: "style",
    },
    rollupOptions: {
      external: [
        "svelte",
        /^svelte\//,
        "@shadowcat/core",
        "@shadowcat/ui-kit",
        "@shadowcat/formula",
        "@shadowcat/types",
      ],
    },
  },
});
