import { defineConfig, configDefaults } from "vitest/config";

// Default unit run: everything except the Node<->Rust e2e suite, which needs the
// Rust toolchain and is run separately (test:e2e) in its own CI job.
export default defineConfig({
  test: {
    exclude: [...configDefaults.exclude, "**/*.e2e.test.ts"],
  },
});
