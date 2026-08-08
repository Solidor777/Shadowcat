import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Dev: the SPA is served by Vite; /api and /ws proxy to the Rust server so
// `vite dev` runs against a real backend. SHADOWCAT_SERVER overrides the target.
const target = process.env.SHADOWCAT_SERVER ?? "http://127.0.0.1:30000";

// Shared-runtime ESM entry chunks: each bare specifier below is
// ALSO a genuine Rollup entry point in this same multi-entry build, so Rollup's
// standard entry-sharing dedup makes the app's own first-party bundle AND any
// future external module import the SAME runtime instance — but ONLY because
// `build.rollupOptions.preserveEntrySignatures = "strict"` below forces every
// entry chunk to keep its full real export surface. Without that flag Vite's
// default (`preserveEntrySignatures: false`) lets Rollup drop or mangle an
// entry's exports down to whatever THIS build's own module graph consumes by
// name, so an external module importing e.g. `mount` from `svelte` would
// resolve to a chunk that no longer exports it under that name — a hard
// runtime `SyntaxError`, not a second-instance violation, but equally fatal
// to external-module sharing. Chunk filenames are forced stable
// (`runtime/<name>.js`, no content hash) so `index.html`'s import map below
// can reference them at build time. This set is the empirically-used surface
// in THIS codebase (grep for `from "svelte` across src/): `svelte` (user
// imports), `svelte/reactivity` (SvelteMap, widely used), plus the two
// internal subpaths every compiled Svelte 5 component imports regardless of
// author code (`svelte/internal/client`, `svelte/internal/disclose-version`).
// A module author introducing a NEW svelte/* subpath (e.g. `svelte/store`,
// `svelte/transition`) needs this list extended — see the module-authoring
// guide. `shadowcat-types` is deliberately a zero-export chunk: `@shadowcat/types`
// is type-only and fully erased by tsc, so an empty runtime module is correct,
// not a bug.
export const RUNTIME_ENTRIES: Record<string, string> = {
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
      // Preserves each RUNTIME_ENTRIES chunk's full real export binding
      // surface — see the comment above. Required for external-module
      // consumers to resolve named imports against these chunks.
      preserveEntrySignatures: "strict",
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
