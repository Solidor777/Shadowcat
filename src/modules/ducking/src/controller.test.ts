import { describe, it, expect, afterEach } from "vitest";
import { DuckSourcesController } from "./controller";
import { DEFAULT_DUCKING_PREFERENCES } from "./duckingMirror";

const logger = { debug() {}, warn() {}, error() {} };

describe("DuckSourcesController", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("starts nothing by default (both sources disabled)", () => {
    const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
    const calls: number[] = [];
    controller.wireToAudioDuck({ set: (v) => calls.push(v) }, { set: () => {} });
    window.dispatchEvent(new KeyboardEvent("keydown", { code: DEFAULT_DUCKING_PREFERENCES.keyBinding }));
    expect(calls).toEqual([]);
    controller.dispose();
  });

  it("applyPreferences starts the key source when it and master are enabled", () => {
    const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
    const calls: number[] = [];
    controller.wireToAudioDuck({ set: (v) => calls.push(v) }, { set: () => {} });
    controller.applyPreferences({ ...DEFAULT_DUCKING_PREFERENCES, keyEnabled: true });
    window.dispatchEvent(new KeyboardEvent("keydown", { code: DEFAULT_DUCKING_PREFERENCES.keyBinding }));
    expect(calls).toEqual([1]);
    controller.dispose();
  });

  it("masterEnabled false stops an already-enabled key source", () => {
    const controller = new DuckSourcesController({ ...DEFAULT_DUCKING_PREFERENCES, keyEnabled: true }, logger);
    const calls: number[] = [];
    controller.wireToAudioDuck({ set: (v) => calls.push(v) }, { set: () => {} });
    controller.applyPreferences({ ...DEFAULT_DUCKING_PREFERENCES, keyEnabled: true, masterEnabled: false });
    // `KeySource.stop()` itself resets demand to 0 — that reset, not the keydown below
    // (which never reaches a stopped source's listeners), is what produces this one call.
    expect(calls).toEqual([0]);
    window.dispatchEvent(new KeyboardEvent("keydown", { code: DEFAULT_DUCKING_PREFERENCES.keyBinding }));
    expect(calls).toEqual([0]); // no further calls: the key source's listeners are detached
    controller.dispose();
  });
});
