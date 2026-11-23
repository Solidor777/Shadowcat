/** Environment variable that moves every port the Playwright suite uses. Two checkouts that run the
 * suite at once set different values so their test servers never share a port. */
export const E2E_PORT_BASE_ENV = "SHADOWCAT_E2E_PORT_BASE";

/** The port base used when `E2E_PORT_BASE_ENV` is unset: it yields the ports the suite has always
 * used (31999 for the loopback server, 32000 to 32003 for the others). */
export const DEFAULT_E2E_PORT_BASE = 32000;

/** The ports of the suite's test servers. */
export interface E2ePorts {
  /** The loopback server every plain project targets. */
  loopback: number;
  /** The non-loopback server the `lan-plain-http` project targets. */
  lan: number;
  /** The built-in-HTTPS server the https projects target. */
  https: number;
  /** The loopback server with the short egress window, for the `media-range-loopback` project. */
  mediaRange: number;
  /** The server the long-audio projects target. */
  longAudio: number;
}

/** Highest port number a TCP listener can use. */
const MAX_PORT = 65535;

/** Number of ports the suite takes above the base (`lan`, `https`, `mediaRange`, `longAudio`). */
const PORTS_ABOVE_BASE = 4;

/**
 * Derives every test-server port from one base value. The loopback server sits one below the base
 * and the other four take the base and the three ports above it, so the five ports are contiguous
 * and move together.
 * @param env The process environment to read `E2E_PORT_BASE_ENV` from.
 * @returns The five ports. An unset or empty variable gives `DEFAULT_E2E_PORT_BASE`.
 * @throws When the value is not a whole number whose five ports all fit in 1 to 65535.
 * @example
 * ```
 * const ports = resolveE2ePorts({ SHADOWCAT_E2E_PORT_BASE: "40000" }); // ports.https === 40001
 * ```
 */
export function resolveE2ePorts(env: Record<string, string | undefined>): E2ePorts {
  const raw = env[E2E_PORT_BASE_ENV];
  const base = raw === undefined || raw === "" ? DEFAULT_E2E_PORT_BASE : Number(raw);
  if (!Number.isInteger(base) || base < 2 || base + PORTS_ABOVE_BASE - 1 > MAX_PORT) {
    throw new Error(
      `${E2E_PORT_BASE_ENV}=${String(raw)} is not a whole number from 2 to ${MAX_PORT - PORTS_ABOVE_BASE + 1}`,
    );
  }
  return { loopback: base - 1, lan: base, https: base + 1, mediaRange: base + 2, longAudio: base + 3 };
}
