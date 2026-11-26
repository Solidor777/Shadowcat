import type { TlsStatus } from "@shadowcat/types";

/** Delays between attempts to read the TLS status after a failure, in milliseconds. After the last
 * the read gives up until the next {@link TlsStatusLoader.ensure} call. */
export const TLS_RETRY_DELAYS_MS: readonly number[] = [1_000, 2_000, 4_000, 8_000, 15_000, 30_000];

/**
 * Reads the server's published TLS status once, retrying a failed read with backoff so a single
 * transient error does not leave every TLS-dependent message silent for the whole session, and
 * reads it again whenever {@link ensure} is called while it is still unread (the shell calls it on
 * every WebSocket welcome and at the WebSocket-failure hint).
 *
 * Reactive: {@link status} is `$state`, so readers re-run when a read lands. Concurrent callers
 * share one request loop.
 */
export class TlsStatusLoader {
  /** The status once read, else `null` (unread, or every attempt so far failed). */
  status = $state<TlsStatus | null>(null);

  /** The in-flight read loop, so concurrent callers share it. */
  #inflight: Promise<TlsStatus | null> | null = null;

  /**
   * Creates a loader; nothing is read until {@link TlsStatusLoader.ensure} is called.
   * @param load Fetches the status (`GET /api/config`'s `tls` block); rejects on failure.
   * @param delays Delays between attempts after a failure, in milliseconds.
   * @param sleep Waits `ms` milliseconds; injectable for tests.
   * @example
   * ```ts
   * const loader = new TlsStatusLoader(async () => { throw new Error("down"); }, [], async () => {});
   * ```
   */
  constructor(
    private readonly load: () => Promise<TlsStatus>,
    private readonly delays: readonly number[] = TLS_RETRY_DELAYS_MS,
    private readonly sleep: (ms: number) => Promise<void> = (ms) => new Promise((r) => setTimeout(r, ms)),
  ) {}

  /**
   * Returns the status, reading it (with retries) when it is not yet known.
   * @returns The status, or `null` when every attempt of this call failed.
   * @example
   * ```ts
   * declare const loader: TlsStatusLoader;
   * const status = await loader.ensure();
   * ```
   */
  ensure(): Promise<TlsStatus | null> {
    if (this.status) return Promise.resolve(this.status);
    this.#inflight ??= this.#run().finally(() => (this.#inflight = null));
    return this.#inflight;
  }

  /**
   * One read loop: the first attempt plus one retry per configured delay.
   * @returns The status, or `null` when every attempt failed.
   * @example
   * ```ts
   * // private; reached through `ensure()`
   * ```
   */
  async #run(): Promise<TlsStatus | null> {
    for (let attempt = 0; attempt <= this.delays.length; attempt++) {
      try {
        this.status = await this.load();
        return this.status;
      } catch {
        const delay = this.delays[attempt];
        if (delay === undefined) break;
        await this.sleep(delay);
      }
    }
    return null;
  }
}
