// The core's diagnostic seam. Production hosts inject their own Logger; the
// core never calls console.* directly (raw console output is banned). Hook /
// module errors are isolated and reported here rather than thrown.
export interface Logger {
  debug(msg: string, meta?: unknown): void;
  warn(msg: string, meta?: unknown): void;
  error(msg: string, meta?: unknown): void;
}

export const silentLogger: Logger = {
  debug() {},
  warn() {},
  error() {},
};

/** A development logger that prefixes the project tag; not used in the bundle. */
export function consoleLogger(): Logger {
  return {
    debug: (m, meta) => console.debug(`[shadowcat] ${m}`, meta),
    warn: (m, meta) => console.warn(`[shadowcat] ${m}`, meta),
    error: (m, meta) => console.error(`[shadowcat] ${m}`, meta),
  };
}
