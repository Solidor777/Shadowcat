import { describe, it, expect } from "vitest";
import { ChatBubblesPreference, CHAT_BUBBLES_STORAGE_KEY } from "./preference.svelte";

/** An in-memory `Storage` stand-in. */
function memory(initial: Record<string, string> = {}): Pick<Storage, "getItem" | "setItem"> & { data: Record<string, string> } {
  const data = { ...initial };
  return { data, getItem: (k) => data[k] ?? null, setItem: (k, v) => { data[k] = v; } };
}

describe("ChatBubblesPreference", () => {
  it("defaults to on with an empty store", () => {
    expect(new ChatBubblesPreference(memory()).enabled).toBe(true);
  });

  it("set(false) turns it off and persists the choice", () => {
    const store = memory();
    const pref = new ChatBubblesPreference(store);
    pref.set(false);
    expect(pref.enabled).toBe(false);
    expect(store.data[CHAT_BUBBLES_STORAGE_KEY]).toBe("0");
  });

  it("restores the persisted off state in a new instance", () => {
    expect(new ChatBubblesPreference(memory({ [CHAT_BUBBLES_STORAGE_KEY]: "0" })).enabled).toBe(false);
  });

  it("restores the persisted on state", () => {
    const store = memory();
    new ChatBubblesPreference(store).set(true);
    expect(new ChatBubblesPreference(store).enabled).toBe(true);
  });

  it("a throwing store keeps the default and the in-session choice", () => {
    const broken = { getItem: () => { throw new Error("denied"); }, setItem: () => { throw new Error("denied"); } };
    const pref = new ChatBubblesPreference(broken);
    expect(pref.enabled).toBe(true);
    pref.set(false);
    expect(pref.enabled).toBe(false);
  });

  it("works with no store at all", () => {
    const pref = new ChatBubblesPreference(null);
    pref.set(false);
    expect(pref.enabled).toBe(false);
  });
});
