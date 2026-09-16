import { describe, it, expect, afterEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DuckSourcesController } from "./controller";
import { DEFAULT_DUCKING_PREFERENCES, DUCKING_MIRROR_STORAGE_KEY } from "./duckingMirror";
import DuckingSettings from "./DuckingSettings.svelte";

const logger = { debug() {}, warn() {}, error() {} };

/**
 * Builds the `audio` fixture every test needs — the component reads `audio.duck.depth` at
 * mount, so every `setAppContextForTest` call in this file supplies one.
 * @param depth The fixture's starting duck depth; default `0.7`.
 * @param setDepth Spy for `duck.setDepth`; default a no-op `vi.fn()`.
 * @returns The `audio` fixture slice.
 * @example
 * ```
 * const context = setAppContextForTest({ audio: audioFixture() });
 * ```
 */
function audioFixture(depth = 0.7, setDepth = vi.fn()) {
  return {
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
    duck: {
      depth,
      setDepth,
      addSource: vi.fn(),
      removeSource: vi.fn(),
      gain: 1,
    },
    playOneShot: () => {},
    serverNow: () => 0,
    transport: () => {},
    listenAs: () => {},
  };
}

describe("DuckingSettings", () => {
  afterEach(() => {
    localStorage.clear();
  });

  it("renders every control labeled and persists a master-enable toggle", async () => {
    const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
    const context = setAppContextForTest({ audio: audioFixture() });
    render(DuckingSettings, { props: { controller }, context });
    const master = screen.getByLabelText("ducking.masterEnable") as HTMLInputElement;
    expect(master.checked).toBe(true);
    await fireEvent.click(master);
    expect(master.checked).toBe(false);
    const stored = JSON.parse(localStorage.getItem(DUCKING_MIRROR_STORAGE_KEY)!);
    expect(stored.masterEnabled).toBe(false);
    controller.dispose();
  });

  it("binds a new key via the capture button", async () => {
    const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
    const context = setAppContextForTest({ audio: audioFixture() });
    render(DuckingSettings, { props: { controller }, context });
    await fireEvent.click(screen.getByRole("button", { name: /ducking\.keySource\.bind/ }));
    const event = new KeyboardEvent("keydown", { code: "KeyV" });
    window.dispatchEvent(event);
    await waitFor(() => {
      const stored = JSON.parse(localStorage.getItem(DUCKING_MIRROR_STORAGE_KEY)!);
      expect(stored.keyBinding).toBe("KeyV");
    });
    controller.dispose();
  });

  it("commits the watch-list text as a trimmed, filtered array on change", async () => {
    const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
    const context = setAppContextForTest({ audio: audioFixture() });
    render(DuckingSettings, { props: { controller }, context });
    const input = screen.getByLabelText("ducking.osSource.watchList") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "discord, , teams ,zoom" } });
    await fireEvent.change(input);
    const stored = JSON.parse(localStorage.getItem(DUCKING_MIRROR_STORAGE_KEY)!);
    expect(stored.watchList).toEqual(["discord", "teams", "zoom"]);
    controller.dispose();
  });

  it("a mic denial reason shows a message and resets the checkbox", async () => {
    const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
    controller.micToggle = vi.fn().mockResolvedValue("permission-denied");
    const context = setAppContextForTest({ audio: audioFixture() });
    render(DuckingSettings, { props: { controller }, context });
    const micCheckbox = screen.getByLabelText("ducking.micSource.enable") as HTMLInputElement;
    await fireEvent.click(micCheckbox);
    await waitFor(() => {
      expect(screen.getByText("ducking.micSource.denied.permission-denied")).toBeTruthy();
    });
    expect(micCheckbox.checked).toBe(false);
    controller.dispose();
  });

  it("reflects the OS monitor's live status", async () => {
    const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
    const context = setAppContextForTest({ audio: audioFixture() });
    render(DuckingSettings, { props: { controller }, context });
    expect(screen.getByText("ducking.osSource.status.connecting")).toBeTruthy();
    controller.dispose();
  });

  it("the depth slider forwards its value to audio.duck.setDepth and never touches the mirror", async () => {
    const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
    const setDepth = vi.fn();
    const context = setAppContextForTest({ audio: audioFixture(0.5, setDepth) });
    render(DuckingSettings, { props: { controller }, context });
    const slider = screen.getByLabelText("ducking.depth") as HTMLInputElement;
    expect(slider.value).toBe("0.5");
    await fireEvent.input(slider, { target: { value: "0.3" } });
    expect(setDepth).toHaveBeenCalledWith(0.3);
    expect(localStorage.getItem(DUCKING_MIRROR_STORAGE_KEY) ?? "").not.toContain("depth");
    controller.dispose();
  });
});
