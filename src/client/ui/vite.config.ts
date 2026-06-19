import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Dev: the SPA is served by Vite; /api and /ws proxy to the Rust server so
// `vite dev` runs against a real backend. SHADOWCAT_SERVER overrides the target.
const target = process.env.SHADOWCAT_SERVER ?? "http://127.0.0.1:30000";

export default defineConfig({
  plugins: [svelte()],
  build: { outDir: "../../../dist", emptyOutDir: true },
  server: {
    proxy: {
      "/api": { target, changeOrigin: true },
      "/ws": { target, ws: true, changeOrigin: true },
    },
  },
});
