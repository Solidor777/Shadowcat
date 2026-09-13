// @vitest-environment node
import { describe, it, expect, vi, afterEach } from "vitest";
import { readDeviceSignals } from "./deviceSignals";

describe("readDeviceSignals", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("returns {} when every global probe is absent", () => {
    // Node >= 21.5 ships a global `navigator` with `hardwareConcurrency`; stub it away so this
    // pins the "every probe absent" path it names.
    vi.stubGlobal("navigator", undefined);
    expect(readDeviceSignals()).toEqual({});
  });

  it("reads hardwareConcurrency when the global exposes it", () => {
    vi.stubGlobal("navigator", { hardwareConcurrency: 8 });
    expect(readDeviceSignals()).toEqual({ hardwareConcurrency: 8 });
  });
});
