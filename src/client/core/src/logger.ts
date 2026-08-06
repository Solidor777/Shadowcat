// The core's diagnostic seam. Production hosts inject their own Logger; the
// core never calls console.* directly (raw console output is banned). Hook /
// module errors are isolated and reported here rather than thrown.
/** The diagnostic sink every core function accepts instead of calling `console.*`
 * directly — see the module note above. */
export interface Logger {
  /** Logs a low-severity diagnostic, not surfaced to the user.
   * @param msg The message text.
   * @param meta Optional structured context.
   */
  debug(msg: string, meta?: unknown): void;
  /** Logs a recoverable-condition diagnostic.
   * @param msg The message text.
   * @param meta Optional structured context.
   */
  warn(msg: string, meta?: unknown): void;
  /** Logs a failure diagnostic.
   * @param msg The message text.
   * @param meta Optional structured context.
   */
  error(msg: string, meta?: unknown): void;
}

export const silentLogger: Logger = {
  debug() {},
  warn() {},
  error() {},
};

/** The production console logger; prefixes the project tag. Shell entry points (worldSession, sessionState, Table) construct it per consumer.
 * @returns A fresh `Logger` that writes to `console.debug`/`warn`/`error`.
 * @example
 * ```ts
 * import { consoleLogger } from "@shadowcat/core";
 *
 * const logger = consoleLogger();
 * logger.warn("world session degraded", { reason: "welcome timeout" });
 * ```
 */
export function consoleLogger(): Logger {
  return {
    debug: (m, meta) => console.debug(`[shadowcat] ${m}`, meta),
    warn: (m, meta) => console.warn(`[shadowcat] ${m}`, meta),
    error: (m, meta) => console.error(`[shadowcat] ${m}`, meta),
  };
}
