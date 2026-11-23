import { apiPath, ServerConfigSchema } from "@shadowcat/core";

/** Reads the server's `onboarding_enabled` switch from the public `GET /api/config` body.
 *
 * The body is parsed with the shared `ServerConfigSchema`, so the config body has one parser.
 * The flag defaults to `true` server-side, so a body that does not parse, a non-2xx status or a
 * failed request all mean "tours allowed": only an explicit `false` suppresses them. The route
 * is unauthenticated, so this works before and after sign-in.
 * @param fetchFn The fetch implementation (injectable for tests).
 * @returns `false` only when the server reports `onboarding_enabled: false`.
 * @example
 * ```ts
 * import { fetchOnboardingEnabled } from "./serverFlag";
 *
 * const allowed = await fetchOnboardingEnabled(async () => new Response("{}"));
 * // allowed === true
 * ```
 */
export async function fetchOnboardingEnabled(fetchFn: typeof fetch = fetch): Promise<boolean> {
  try {
    const res = await fetchFn(apiPath("/api/config"), { headers: { accept: "application/json" } });
    if (!res.ok) return true;
    const parsed = ServerConfigSchema.safeParse(await res.json());
    return parsed.success ? parsed.data.onboarding_enabled : true;
  } catch {
    return true;
  }
}
