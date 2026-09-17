// @vitest-environment node
import { describe, it, expect } from "vitest";
import {
  readDuckingMirror,
  writeDuckingMirror,
  DEFAULT_DUCKING_PREFERENCES,
  DUCKING_MIRROR_STORAGE_KEY,
} from "./duckingMirror";

function fakeStorage(initial: Record<string, string> = {}) {
  const store = { ...initial };
  return {
    getItem: (k: string) => store[k] ?? null,
    setItem: (k: string, v: string) => {
      store[k] = v;
    },
    raw: store,
  };
}

describe("ducking mirror", () => {
  it("returns the defaults when absent", () => {
    expect(readDuckingMirror(fakeStorage())).toEqual(DEFAULT_DUCKING_PREFERENCES);
  });

  it("returns the defaults on malformed JSON", () => {
    expect(readDuckingMirror(fakeStorage({ [DUCKING_MIRROR_STORAGE_KEY]: "{not json" }))).toEqual(
      DEFAULT_DUCKING_PREFERENCES,
    );
  });

  it("fills a partial stored object from the defaults", () => {
    const storage = fakeStorage({ [DUCKING_MIRROR_STORAGE_KEY]: JSON.stringify({ osPort: 40000 }) });
    expect(readDuckingMirror(storage)).toEqual({ ...DEFAULT_DUCKING_PREFERENCES, osPort: 40000 });
  });

  it("round-trips a full write/read", () => {
    const storage = fakeStorage();
    const value = { ...DEFAULT_DUCKING_PREFERENCES, keyEnabled: true, watchList: ["discord", "teams"] };
    writeDuckingMirror(storage, value);
    expect(readDuckingMirror(storage)).toEqual(value);
  });

  it("a throwing storage write is swallowed", () => {
    const storage = {
      getItem: () => null,
      setItem: () => {
        throw new Error("quota");
      },
    };
    expect(() => writeDuckingMirror(storage, DEFAULT_DUCKING_PREFERENCES)).not.toThrow();
  });
});
