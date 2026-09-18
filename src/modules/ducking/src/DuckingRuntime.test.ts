import { describe, it, expect, afterEach, vi } from "vitest";
import { render } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DuckSourcesController } from "./controller";
import { DEFAULT_DUCKING_PREFERENCES } from "./duckingMirror";
import DuckingRuntime from "./DuckingRuntime.svelte";

const logger = { debug() {}, warn() {}, error() {} };

/**
 * Builds the `audio` fixture with spied `duck.addSource`/`duck.removeSource` — the same
 * spy-a-fixture pattern `setAppContextForTest`'s `combat`/`chat` fixtures use elsewhere in this
 * codebase.
 * @returns The audio fixture plus its spies.
 * @example
 * ```
 * const { audio, addSource, removeSource } = audioFixture();
 * ```
 */
function audioFixture() {
  const addSource = vi.fn(() => ({ set: () => {} }));
  const removeSource = vi.fn();
  const audio = {
    channels: {
      master: { gain: 1, muted: false },
      music: { gain: 1, muted: false },
      ambience: { gain: 1, muted: false },
      sfx: { gain: 1, muted: false },
      ui: { gain: 1, muted: false },
    },
    setChannel: () => {},
    unlock: async () => {},
    context: () => null,
    duck: { addSource, removeSource, gain: 1, depth: 0.7, setDepth: () => {} },
    playOneShot: () => {},
    serverNow: () => 0,
    transport: () => {},
    listenAs: () => {},
  };
  return { audio, addSource, removeSource };
}

describe("DuckingRuntime", () => {
  afterEach(() => {
    localStorage.clear();
  });

  it("adds exactly the key and os-monitor duck sources on mount and wires them into the controller", () => {
    const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
    const { audio, addSource } = audioFixture();
    const context = setAppContextForTest({ audio });
    render(DuckingRuntime, { props: { controller }, context });
    expect(addSource).toHaveBeenCalledWith("ducking:key");
    expect(addSource).toHaveBeenCalledWith("ducking:os-monitor");
    expect(addSource).toHaveBeenCalledTimes(2);
    expect(controller.micToggle).not.toBeNull();
    controller.dispose();
  });

  it("removes exactly the key and os-monitor sources on unmount, never the mic source before it was ever added", () => {
    const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
    const { audio, removeSource } = audioFixture();
    const context = setAppContextForTest({ audio });
    const { unmount } = render(DuckingRuntime, { props: { controller }, context });
    unmount();
    expect(removeSource).toHaveBeenCalledWith("ducking:key");
    expect(removeSource).toHaveBeenCalledWith("ducking:os-monitor");
    expect(removeSource).not.toHaveBeenCalledWith("ducking:mic");
    expect(controller.micToggle).toBeNull();
    controller.dispose();
  });

  it("stops the key and OS-monitor sources on unmount — the only reliably-firing teardown hook in production, since WorldSession.leave() never calls ModuleRegistry.unload()", () => {
    const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
    const keyStop = vi.spyOn(controller.key, "stop");
    const osStop = vi.spyOn(controller.osMonitor, "stop");
    const { audio } = audioFixture();
    const context = setAppContextForTest({ audio });
    const { unmount } = render(DuckingRuntime, { props: { controller }, context });
    unmount();
    expect(keyStop).toHaveBeenCalled();
    expect(osStop).toHaveBeenCalled();
  });

  it("micToggle('unknown') denies enabling before the engine's AudioContext is unlocked", async () => {
    const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
    const { audio } = audioFixture();
    const context = setAppContextForTest({ audio });
    render(DuckingRuntime, { props: { controller }, context });
    const denial = await controller.micToggle!(true);
    expect(denial).toBe("unknown");
    controller.dispose();
  });
});
