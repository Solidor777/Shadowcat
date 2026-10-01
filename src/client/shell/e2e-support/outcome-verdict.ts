/** Operating systems the CI matrix runs (`process.platform` values). */
export type Platform = "win32" | "linux" | "darwin";

/** Expected outcomes keyed by Playwright project name, then by the platform they were measured on. */
export type OutcomeTable<T extends string> = Record<string, Partial<Record<Platform, T>>>;

/** The cell is measured and the observation equals it. */
export interface OutcomeMatch {
  /** Discriminant. */
  kind: "match";
}

/** The cell is measured and the observation differs: a real regression or a wrong table entry. */
export interface OutcomeMismatch {
  /** Discriminant. */
  kind: "mismatch";
  /** The measured outcome. */
  expected: string;
  /** Names the project, platform, observation and measurement. */
  message: string;
}

/** The cell has no measurement and the run is local: annotate and carry on. */
export interface OutcomeUnmeasuredAnnotate {
  /** Discriminant. */
  kind: "unmeasured-annotate";
  /** Names the project and platform and carries the observed value. */
  message: string;
}

/** The cell has no measurement and the run is on CI: fail so the run reports the observed value. */
export interface OutcomeUnmeasuredFail {
  /** Discriminant. */
  kind: "unmeasured-fail";
  /** Names the project and platform and carries the observed value. */
  message: string;
}

/** What to do with an observed outcome. */
export type OutcomeVerdict =
  | OutcomeMatch
  | OutcomeMismatch
  | OutcomeUnmeasuredAnnotate
  | OutcomeUnmeasuredFail;

/**
 * Judges an observation against the outcome measured for `project` on `platform`. Engines differ
 * per OS, so a value measured on one OS proves nothing about another: an unmeasured cell never
 * passes silently. Locally it is an annotation; on CI it FAILS carrying the observed value, so the
 * first CI run on each OS produces the measurement to record in the table.
 * @param table The measured outcomes.
 * @param project The Playwright project name.
 * @param platform The platform the run is on.
 * @param observed What this run observed.
 * @param ci Whether the run is on CI.
 * @returns The verdict.
 * @example
 * ```ts
 * const v = judgeOutcome({ p: { win32: "a" } }, "p", "linux", "b", true);
 * // v.kind === "unmeasured-fail"
 * ```
 */
export function judgeOutcome<T extends string>(
  table: OutcomeTable<T>,
  project: string,
  platform: Platform,
  observed: string,
  ci: boolean,
): OutcomeVerdict {
  const measured = table[project]?.[platform];
  if (measured === undefined) {
    const message = `${project} on ${platform} has no measured outcome; observed ${JSON.stringify(observed)}. Record it in the table for this platform.`;
    return ci ? { kind: "unmeasured-fail", message } : { kind: "unmeasured-annotate", message };
  }
  if (observed === measured) return { kind: "match" };
  return {
    kind: "mismatch",
    expected: measured,
    message: `${project} on ${platform}: observed ${JSON.stringify(observed)}, measured ${JSON.stringify(measured)}`,
  };
}
