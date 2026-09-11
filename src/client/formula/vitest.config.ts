import { defineConfig } from "vitest/config";

// An explicit (if empty) config file is required here so this package's own
// `vitest run` resolves to ITS root rather than walking up and picking up
// the repo-root vitest.config.mjs (scoped to scripts/**/*.test.mjs, which
// this package has none of).
export default defineConfig({});
