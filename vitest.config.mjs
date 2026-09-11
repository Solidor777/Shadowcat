import { defineConfig } from "vitest/config";

// `pnpm run test:scripts` (`vitest run scripts/`) is the intended reader of
// this file. A workspace package that owns NO vitest config of its own
// resolves upward and matches this one instead — it then finds zero tests
// under `scripts/**/*.test.mjs` from that package's root and fails loudly
// ("No test files found"), rather than silently running against the wrong
// config. `src/client/formula/vitest.config.ts` exists precisely because
// Vite DID walk up to this file when that package had none: every package
// that runs vitest MUST own its own config.
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
