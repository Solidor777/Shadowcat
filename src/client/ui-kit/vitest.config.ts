import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { svelteTesting } from "@testing-library/svelte/vite";

// Mirrors the shell's vitest config so the seam suites behave identically after
// the move: jsdom env + @testing-library/svelte auto-cleanup + browser-condition
// resolution. Unit tests are `src/**/*.test.ts`.
export default defineConfig({
  plugins: [svelte(), svelteTesting()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./vitest.setup.ts"],
    include: ["src/**/*.test.ts"],
  },
});
