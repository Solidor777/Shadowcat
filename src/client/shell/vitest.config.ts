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
    setupFiles: ["../../../test-setup/jsdom-local-storage.ts", "./vitest.setup.ts"],
    // Unit tests are `src/**/*.test.ts` plus the pure helpers the Playwright config imports
    // (`e2e-support/`); the Playwright e2e (`e2e/*.spec.ts`) runs under Playwright, not Vitest.
    include: ["src/**/*.test.ts", "e2e-support/**/*.test.ts"],
  },
});
