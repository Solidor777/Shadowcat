import type { Page } from "@playwright/test";

/** Members recorded per owner object; every entry is `[SecureContext]` in its Web IDL. A dotted
 * entry is a second-level member reached through the first (`mediaDevices.getUserMedia`): on a
 * non-secure origin the first-level member is already absent, so the second level measures as
 * absent through the same lookup. One table drives both the plain-HTTP spec (`lan-plain-http`,
 * every entry absent) and the HTTPS spec (the entries the engine implements are present, and the
 * present set is identical to the secure loopback control's), so the two cannot drift apart. The `Crypto` UUID member is deliberately not listed: the
 * `lint:secure-context` gate reserves its name for the `randomUuid` seam and the HTTPS spec, which
 * asserts it directly beside this table. */
export const SECURE_CONTEXT_MEMBERS = {
  crypto: ["subtle"],
  navigator: [
    "mediaDevices",
    "mediaDevices.getUserMedia",
    "clipboard",
    "serviceWorker",
    "storage",
    "locks",
    "share",
    "credentials",
    "deviceMemory",
    "gpu",
  ],
  audioContext: ["audioWorklet"],
} as const;

/** An `AudioContext` constructor as far as the measurement needs it. */
type AudioContextConstructor = new () => AudioContextHandle;

/** The one `AudioContext` member the measurement calls, to release the context afterwards. */
interface AudioContextHandle {
  /** Releases the context's audio resources.
   * @returns Resolves once the context is closed. */
  close(): Promise<void>;
}

/** The measured `typeof` of each table entry (`"<owner>.<member>"`), plus `isSecureContext`
 * (`"true"`/`"false"`), and the members that must exist on every origin. */
export type MeasuredMembers = Record<string, string>;

/** Members that are NOT `[SecureContext]`, measured as controls: they exist on every origin, so a
 * measurement that reports them absent came from a broken page, not from the origin's security. */
export const ALWAYS_PRESENT: Record<string, string> = {
  "crypto.getRandomValues": "function",
  "navigator.hardwareConcurrency": "number",
};

/**
 * Measures `typeof` of every entry of {@link SECURE_CONTEXT_MEMBERS} on `page`'s current origin,
 * through one dynamic-key lookup path (`owner[first][second]`).
 * @param page A page already navigated to the origin under test.
 * @returns The `typeof` string of each `"<owner>.<member>"` entry, the always-present controls,
 * and `isSecureContext`.
 * @example
 * ```ts
 * declare const page: import("@playwright/test").Page;
 * const measured = await measureSecureContextMembers(page);
 * // measured.isSecureContext === "true" on an https page
 * ```
 */
export async function measureSecureContextMembers(page: Page): Promise<MeasuredMembers> {
  return page.evaluate((members) => {
    // Engines without an unprefixed `AudioContext` measure its members as absent.
    const w = window as unknown as Record<string, AudioContextConstructor | undefined>;
    const Ctx = w["AudioContext"] ?? w["webkitAudioContext"];
    const context = Ctx ? new Ctx() : undefined;
    try {
      const owners: Record<string, unknown> = {
        crypto,
        navigator,
        audioContext: context,
      };
      const out: Record<string, string> = { isSecureContext: String(window.isSecureContext) };
      for (const [owner, names] of Object.entries(members)) {
        for (const name of names) {
          let value: unknown = owners[owner];
          for (const part of name.split(".")) {
            value = value === undefined || value === null ? undefined : (value as Record<string, unknown>)[part];
          }
          out[`${owner}.${name}`] = typeof value;
        }
      }
      out["crypto.getRandomValues"] = typeof crypto.getRandomValues;
      out["navigator.hardwareConcurrency"] = typeof navigator.hardwareConcurrency;
      // Informational: recorded in the attached measurement, asserted by nothing.
      for (const name of ["Notification", "AudioContext", "AudioWorkletNode", "BroadcastChannel", "localStorage", "WebAssembly", "caches", "indexedDB"]) {
        out[name] = typeof (w as Record<string, unknown>)[name];
      }
      return out;
    } finally {
      void context?.close();
    }
  }, SECURE_CONTEXT_MEMBERS);
}

/**
 * The table entries a measurement found present (any `typeof` other than `"undefined"`), sorted.
 * @param measured The result of {@link measureSecureContextMembers}.
 * @returns Sorted `"<owner>.<member>"` names of the present table entries.
 * @example
 * ```ts
 * presentMembers({ isSecureContext: "true", "navigator.deviceMemory": "number" });
 * // ["navigator.deviceMemory"]
 * ```
 */
export function presentMembers(measured: MeasuredMembers): string[] {
  const entries = Object.entries(SECURE_CONTEXT_MEMBERS).flatMap(([owner, names]) =>
    names.map((name) => `${owner}.${name}`),
  );
  return entries.filter((key) => measured[key] !== undefined && measured[key] !== "undefined").sort();
}
