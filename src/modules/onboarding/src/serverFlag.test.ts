import { describe, expect, it } from "vitest";
import { fetchOnboardingEnabled } from "./serverFlag";

/** A complete `GET /api/config` body with the given `onboarding_enabled`.
 * @param onboardingEnabled The value of the switch.
 * @returns A body that satisfies the shared `ServerConfigSchema`.
 * @example
 * ```ts
 * const body = configBody(false);
 * ```
 */
function configBody(onboardingEnabled: boolean): Record<string, unknown> {
  return {
    initialized: true,
    tls: { enabled: false, source: "self-signed", https_origins: [], alpn: ["http/1.1"] },
    onboarding_enabled: onboardingEnabled,
  };
}

/** A fetch that answers with `body` and `status`.
 * @param body The JSON body.
 * @param status The HTTP status.
 * @returns The fetch stand-in.
 * @example
 * ```ts
 * const fetchFn = answering(configBody(false), 200);
 * ```
 */
function answering(body: unknown, status: number): typeof fetch {
  return async () => new Response(JSON.stringify(body), { status });
}

describe("fetchOnboardingEnabled", () => {
  it("is false only when the server reports onboarding_enabled false", async () => {
    expect(await fetchOnboardingEnabled(answering(configBody(false), 200))).toBe(false);
  });

  it("is true when the server reports true", async () => {
    expect(await fetchOnboardingEnabled(answering(configBody(true), 200))).toBe(true);
  });

  it("is true when the body does not satisfy the shared config schema", async () => {
    expect(await fetchOnboardingEnabled(answering({ initialized: true, onboarding_enabled: false }, 200))).toBe(true);
  });

  it("is true when the field is absent", async () => {
    const withoutFlag = configBody(false);
    delete withoutFlag.onboarding_enabled;
    expect(await fetchOnboardingEnabled(answering(withoutFlag, 200))).toBe(true);
  });

  it("is true on a non-2xx status", async () => {
    expect(await fetchOnboardingEnabled(answering(configBody(false), 500))).toBe(true);
  });

  it("is true when the request fails", async () => {
    expect(await fetchOnboardingEnabled(async () => Promise.reject(new Error("offline")))).toBe(true);
  });

  it("is true when the body is not JSON", async () => {
    expect(await fetchOnboardingEnabled(async () => new Response("not json"))).toBe(true);
  });

  it("reads the unauthenticated config route", async () => {
    const seen: string[] = [];
    await fetchOnboardingEnabled(async (input) => {
      seen.push(String(input));
      return new Response("{}");
    });
    expect(seen).toEqual(["/api/config"]);
  });
});
