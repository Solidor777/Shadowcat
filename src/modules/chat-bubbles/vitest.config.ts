import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { svelteTesting } from "@testing-library/svelte/vite";

export default defineConfig({
  plugins: [svelte(), svelteTesting()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["../../../test-setup/jsdom-local-storage.ts"],
    include: ["src/**/*.test.ts"],
  },
});
