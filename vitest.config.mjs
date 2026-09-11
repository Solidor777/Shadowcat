import { defineConfig } from "vitest/config";

// `pnpm run test:scripts` (`vitest run scripts/`) is the intended reader of
// this file. Vitest's config resolution DOES walk up to this file for a
// package that owns no vitest config of its own, and the match against
// `include: ["scripts/**/*.test.mjs"]` from that package's root always
// yields zero tests. What that zero yields next depends on the invoking
// flags: a plain `vitest run` exits 1 ("No test files found, exiting with
// code 1"), but `--passWithNoTests` — the shape every first-party module's
// own test script uses — exits 0 having run nothing, a silent green with no
// assertion ever executed. That silent-pass outcome, not merely the louder
// one, is why every package that runs vitest MUST own its own config.
// `src/client/formula/vitest.config.ts` exists precisely because this file
// is what Vite walked up to when that package had none.
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
    // supported CI runner runs ~4x a dev machine" (`DUAL_SESSION_TIMEOUT_MS`'s
    // sizing rule) rounds up to 12000ms.
    testTimeout: 12000,
    hookTimeout: 12000,
  },
});
