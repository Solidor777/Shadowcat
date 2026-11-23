import { describe, expect, it } from "vitest";
import { fetchOnboardingEnabled } from "./serverFlag";

/** A fetch that answers with `body` and `status`.
 * @param body The JSON body.
 * @param status The HTTP status.
 * @returns The fetch stand-in.
 * @example
 * ```ts
 * const fetchFn = answering({ onboarding_enabled: false }, 200);
 * ```
 */
function answering(body: unknown, status: number): typeof fetch {
  return async () => new Response(JSON.stringify(body), { status });
}

describe("fetchOnboardingEnabled", () => {
  it("is false only when the server reports onboarding_enabled false", async () => {
    expect(await fetchOnboardingEnabled(answering({ onboarding_enabled: false }, 200))).toBe(false);
  });

  it("is true when the server reports true", async () => {
    expect(await fetchOnboardingEnabled(answering({ onboarding_enabled: true }, 200))).toBe(true);
  });

  it("is true when the field is absent", async () => {
    expect(await fetchOnboardingEnabled(answering({ initialized: true }, 200))).toBe(true);
  });

  it("is true on a non-2xx status", async () => {
    expect(await fetchOnboardingEnabled(answering({ onboarding_enabled: false }, 500))).toBe(true);
  });

  it("is true when the request fails", async () => {
    expect(await fetchOnboardingEnabled(async () => Promise.reject(new Error("offline")))).toBe(true);
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
