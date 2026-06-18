import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// outDir resolves from src/client/ui to the repo-root dist/.
export default defineConfig({
  plugins: [svelte()],
  build: { outDir: "../../../dist", emptyOutDir: true },
});
