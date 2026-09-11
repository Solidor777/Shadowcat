import { defineConfig } from "vitest/config";

// `pnpm run test:scripts` (`vitest run scripts/`) is the only vitest
// invocation that reads this file: every workspace package under
// src/modules and src/client owns its own vitest.config.ts in its own
// directory, and Vite's config resolution searches only the invocation's
// own root, never a parent directory, so this file never reaches them.
export default defineConfig({
  test: {
    include: ["scripts/**/*.test.mjs"],
    // scripts/git-sequencer-state.test.mjs spawns real `git` subprocesses
    // (ten-plus per remaining-work test) rather than mocking the process
    // boundary, so its wall-clock cost scales with host git/process-spawn
    // speed, not with vitest's own per-test overhead. The default 5000ms
    // testTimeout/hookTimeout is a floor sized for in-process assertions
    // and is not enough headroom for a loaded or slower host: measured
    // slowest test at 2887ms on an unloaded dev machine, x4 for "slowest
    // supported CI runner runs ~4x a dev machine" (the sizing rule stated
    // in src/client/shell/e2e/fixtures.ts) rounds up to 12000ms.
    testTimeout: 12000,
    hookTimeout: 12000,
  },
});
