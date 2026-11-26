import { describe, it, expect, afterEach } from "vitest";
import { DuckSourcesController } from "./controller";
import type { KeybindingsController } from "@shadowcat/core";
import { DEFAULT_DUCKING_PREFERENCES, type DuckingPreferences } from "./duckingMirror";
import { DEFAULT_KEY } from "./keySource";
import { makeBindings } from "./__fixtures__/bindings";

const logger = { debug() {}, warn() {}, error() {} };
const BOUND_KEY = DEFAULT_KEY;

describe("DuckSourcesController", () => {
  let keybindings: KeybindingsController | undefined;

  /** Builds a controller whose push-to-duck binding is backed by a real keybindings controller. */
  function build(prefs: DuckingPreferences): DuckSourcesController {
    const made = makeBindings();
    keybindings = made.controller;
    return new DuckSourcesController(prefs, logger, made.bindings);
  }

  afterEach(() => {
    keybindings?.dispose();
    keybindings = undefined;
    document.body.innerHTML = "";
  });

  it("starts nothing by default (both sources disabled)", () => {
    const controller = build(DEFAULT_DUCKING_PREFERENCES);
    const calls: number[] = [];
    controller.wireToAudioDuck({ set: (v) => calls.push(v) }, { set: () => {} });
    window.dispatchEvent(new KeyboardEvent("keydown", { code: BOUND_KEY }));
    expect(calls).toEqual([]);
    controller.dispose();
  });

  it("applyPreferences starts the key source when it and master are enabled", () => {
    const controller = build(DEFAULT_DUCKING_PREFERENCES);
    const calls: number[] = [];
    controller.wireToAudioDuck({ set: (v) => calls.push(v) }, { set: () => {} });
    controller.applyPreferences({ ...DEFAULT_DUCKING_PREFERENCES, keyEnabled: true });
    window.dispatchEvent(new KeyboardEvent("keydown", { code: BOUND_KEY }));
    expect(calls).toEqual([1]);
    controller.dispose();
  });

  it("masterEnabled false stops an already-enabled key source", () => {
    const controller = build({ ...DEFAULT_DUCKING_PREFERENCES, keyEnabled: true });
    const calls: number[] = [];
    controller.wireToAudioDuck({ set: (v) => calls.push(v) }, { set: () => {} });
    controller.applyPreferences({ ...DEFAULT_DUCKING_PREFERENCES, keyEnabled: true, masterEnabled: false });
    // `KeySource.stop()` itself resets demand to 0 — that reset, not the keydown below
    // (which never reaches a stopped source's listeners), is what produces this one call.
    expect(calls).toEqual([0]);
    window.dispatchEvent(new KeyboardEvent("keydown", { code: BOUND_KEY }));
    expect(calls).toEqual([0]); // no further calls: the key source is disarmed
    controller.dispose();
  });
});
