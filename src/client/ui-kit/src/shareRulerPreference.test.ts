// @vitest-environment node
// Exercises plain state: no component render and no DOM API use.
import { describe, it, expect } from "vitest";
import { ShareRulerPreference, shareRulerStorageKey } from "./shareRulerPreference.svelte";

/** An in-memory `Storage` stand-in exposing its backing map. */
function memoryStorage(seed: Record<string, string> = {}) {
  const data = new Map(Object.entries(seed));
  return {
    data,
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => void data.set(k, v),
  };
}

const KEY = shareRulerStorageKey("u1");

describe("ShareRulerPreference", () => {
  it("defaults to off, so a user who never opens the toggle keeps measuring privately", () => {
    expect(new ShareRulerPreference("u1").enabled).toBe(false);
    expect(new ShareRulerPreference("u1", memoryStorage()).enabled).toBe(false);
  });

  it("set updates the value in place and persists it under the user's own key", () => {
    const storage = memoryStorage();
    const pref = new ShareRulerPreference("u1", storage);
    pref.set(true);
    expect(pref.enabled).toBe(true);
    expect(storage.data.get(KEY)).toBe("1");
    pref.set(false);
    expect(pref.enabled).toBe(false);
    expect(storage.data.get(KEY)).toBe("0");
  });

  it("restores a persisted value into a fresh instance", () => {
    expect(new ShareRulerPreference("u1", memoryStorage({ [KEY]: "1" })).enabled).toBe(true);
    expect(new ShareRulerPreference("u1", memoryStorage({ [KEY]: "0" })).enabled).toBe(false);
  });

  it("keeps one opt-in per user: two accounts on one browser profile do not share it", () => {
    const storage = memoryStorage();
    new ShareRulerPreference("alice", storage).set(true);
    expect(new ShareRulerPreference("alice", storage).enabled).toBe(true);
    expect(new ShareRulerPreference("bob", storage).enabled).toBe(false);
    expect(shareRulerStorageKey("alice")).not.toBe(shareRulerStorageKey("bob"));
  });

  it("reads any unrecognized stored value as off", () => {
    for (const junk of ["true", "yes", "2", "", "null"]) {
      expect(new ShareRulerPreference("u1", memoryStorage({ [KEY]: junk })).enabled).toBe(false);
    }
  });

  it("survives a storage that throws on read or write", () => {
    const throwing = {
      getItem: (): string | null => {
        throw new Error("denied");
      },
      setItem: (): void => {
        throw new Error("quota");
      },
    };
    const pref = new ShareRulerPreference("u1", throwing);
    expect(pref.enabled).toBe(false);
    pref.set(true);
    expect(pref.enabled).toBe(true);
  });
});
