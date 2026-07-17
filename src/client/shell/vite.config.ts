import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Dev: the SPA is served by Vite; /api and /ws proxy to the Rust server so
// `vite dev` runs against a real backend. SHADOWCAT_SERVER overrides the target.
const target = process.env.SHADOWCAT_SERVER ?? "http://127.0.0.1:30000";

// Shared-runtime ESM entry chunks (M13-1 T1/T3): each bare specifier below is
// ALSO a genuine Rollup entry point in this same multi-entry build, so Rollup's
// standard entry-sharing dedup makes the app's own first-party bundle AND any
// future external module import the SAME runtime instance — never a second
// copy. Chunk filenames are forced stable (`runtime/<name>.js`, no content
// hash) so `index.html`'s import map below can reference them at build time.
// This set is the empirically-used surface in THIS codebase (grep for
// `from "svelte` across src/): `svelte` (user imports), `svelte/reactivity`
// (SvelteMap, widely used), plus the two internal subpaths every compiled
// Svelte 5 component imports regardless of author code
// (`svelte/internal/client`, `svelte/internal/disclose-version`). A module
// author introducing a NEW svelte/* subpath (e.g. `svelte/store`,
// `svelte/transition`) needs this list extended — see
// docs/design/module-authoring.md.
const RUNTIME_ENTRIES: Record<string, string> = {
  svelte: "svelte",
  "svelte-internal-client": "svelte/internal/client",
  "svelte-internal-disclose-version": "svelte/internal/disclose-version",
  "svelte-reactivity": "svelte/reactivity",
  "shadowcat-core": "@shadowcat/core",
  "shadowcat-ui-kit": "@shadowcat/ui-kit",
  "shadowcat-formula": "@shadowcat/formula",
  "shadowcat-types": "@shadowcat/types",
};

export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: "../../../dist",
    emptyOutDir: true,
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL("./index.html", import.meta.url)),
        ...RUNTIME_ENTRIES,
      },
      output: {
        entryFileNames: (chunk) =>
          chunk.name && chunk.name in RUNTIME_ENTRIES
            ? `runtime/${chunk.name}.js`
            : "assets/[name]-[hash].js",
      },
    },
  },
  server: {
    proxy: {
      "/api": { target, changeOrigin: true },
      "/ws": { target, ws: true, changeOrigin: true },
    },
  },
});
