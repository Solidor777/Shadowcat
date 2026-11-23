/** The part of the `GET /api/config` body this module reads. */
interface ConfigBody {
  /** The server's tour switch; absent on a server that predates it. */
  onboarding_enabled?: unknown;
}

/** Reads the server's `onboarding_enabled` switch from the public `GET /api/config` body.
 *
 * The flag defaults to `true` server-side, so a response that omits it, a non-2xx status or a
 * failed request all mean "tours allowed": only an explicit `false` suppresses them. The route
 * is unauthenticated, so this works before and after sign-in.
 * @param fetchFn The fetch implementation (injectable for tests).
 * @returns `false` only when the server reports `onboarding_enabled: false`.
 * @example
 * ```ts
 * import { fetchOnboardingEnabled } from "./serverFlag";
 *
 * const allowed = await fetchOnboardingEnabled(async () => new Response(JSON.stringify({ onboarding_enabled: false })));
 * // allowed === false
 * ```
 */
export async function fetchOnboardingEnabled(fetchFn: typeof fetch = fetch): Promise<boolean> {
  try {
    const res = await fetchFn("/api/config", { headers: { accept: "application/json" } });
    if (!res.ok) return true;
    const body: unknown = await res.json();
    if (typeof body !== "object" || body === null) return true;
    return (body as ConfigBody).onboarding_enabled !== false;
  } catch {
    return true;
  }
}
