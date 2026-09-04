import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { svelteTesting } from "@testing-library/svelte/vite";

export default defineConfig({
  // `emitCss: false` compiles each component's `<style>` into a runtime-injected `<style>`
  // element instead of a separate CSS asset (which vitest never attaches to the document), so
  // `Layout.test`'s growth-cap assertions can read the declarations back through jsdom's
  // cascade.
  plugins: [svelte({ emitCss: false }), svelteTesting()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./vitest.setup.ts"],
    include: ["src/**/*.test.ts"],
  },
});
