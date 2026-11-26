import type { TestInfo } from "@playwright/test";
import { judgeOutcome, type OutcomeTable, type Platform } from "./outcome-verdict";

export type { OutcomeTable, Platform } from "./outcome-verdict";

/** The run being judged: its operating system and whether it is on CI. */
export interface RunEnvironment {
  /** The operating system the run is on. */
  platform: Platform;
  /** Whether the run is on CI, where an unmeasured cell fails. */
  ci: boolean;
}

/**
 * The environment of the running process.
 * @returns The process platform and whether `CI` is set.
 * @example
 * ```ts
 * const { platform, ci } = currentRun();
 * ```
 */
export function currentRun(): RunEnvironment {
  return { platform: process.platform as Platform, ci: process.env.CI !== undefined };
}

/**
 * Asserts `observed` against the outcome measured for this project on this platform
 * ({@link judgeOutcome} decides). A cell with no measurement for the running platform is recorded
 * as an `UNMEASURED` annotation locally and FAILS on CI with the observed value in the message.
 * @param testInfo The running test.
 * @param table The measured outcomes.
 * @param observed What this run observed.
 * @param run The environment judged against; the running process by default.
 * @returns Whether the cell was measured (and therefore asserted).
 * @throws When the observation differs from the measured outcome, or the cell is unmeasured on CI.
 * @example
 * ```ts
 * declare const testInfo: import("@playwright/test").TestInfo;
 * checkMeasuredOutcome(testInfo, { "https-webkit": { win32: "blocked" } }, "blocked");
 * ```
 */
export function checkMeasuredOutcome<T extends string>(
  testInfo: TestInfo,
  table: OutcomeTable<T>,
  observed: string,
  run: RunEnvironment = currentRun(),
): boolean {
  const { platform, ci } = run;
  testInfo.annotations.push({ type: "observed-outcome", description: `${platform}: ${observed}` });
  const verdict = judgeOutcome(table, testInfo.project.name, platform, observed, ci);
  switch (verdict.kind) {
    case "match":
      return true;
    case "unmeasured-annotate":
      testInfo.annotations.push({ type: "UNMEASURED", description: verdict.message });
      return false;
    case "unmeasured-fail":
    case "mismatch":
      throw new Error(verdict.message);
  }
}
