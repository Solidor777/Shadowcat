import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { svelteTesting } from "@testing-library/svelte/vite";

// Separate from vite.config.ts (the production build): adds the jsdom env and
// @testing-library/svelte's auto-cleanup + browser-condition resolution.
export default defineConfig({
  plugins: [svelte(), svelteTesting()],
  test: {
    environment: "jsdom",
    globals: true,
  },
});
