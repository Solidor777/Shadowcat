import { describe, it, expect, afterEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DuckSourcesController } from "./controller";
import { DEFAULT_DUCKING_PREFERENCES, DUCKING_MIRROR_STORAGE_KEY, type DuckingPreferences } from "./duckingMirror";
import { makeBindings } from "./__fixtures__/bindings";
import DuckingSettings from "./DuckingSettings.svelte";

const logger = { debug() {}, warn() {}, error() {} };

/**
 * Builds a controller whose push-to-duck binding registers on a real keybindings controller
 * (listening on a private target, so tests never share `window` listeners).
 * @param prefs The initial preferences.
 * @returns The controller.
 * @example
 * ```
 * const controller = newController(DEFAULT_DUCKING_PREFERENCES);
 * ```
 */
function newController(prefs: DuckingPreferences): DuckSourcesController {
  return new DuckSourcesController(prefs, logger, makeBindings(new EventTarget()).bindings);
}

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
    needsGesture: false,
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
    preload: () => {},
  };
}

describe("DuckingSettings", () => {
  afterEach(() => {
    localStorage.clear();
  });

  it("renders every control labeled and persists a master-enable toggle", async () => {
    const controller = newController(DEFAULT_DUCKING_PREFERENCES);
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

  it("no longer offers a ducking-specific key capture: rebinding lives in the keybindings section", () => {
    const controller = newController(DEFAULT_DUCKING_PREFERENCES);
    const context = setAppContextForTest({ audio: audioFixture() });
    render(DuckingSettings, { props: { controller }, context });
    expect(screen.queryByRole("button", { name: /ducking\.keySource/ })).toBeNull();
    controller.dispose();
  });

  it("commits the watch-list text as a trimmed, filtered array on change", async () => {
    const controller = newController(DEFAULT_DUCKING_PREFERENCES);
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
    const controller = newController(DEFAULT_DUCKING_PREFERENCES);
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

  it("an unavailable microphone shows its own message and resets the checkbox", async () => {
    const controller = newController(DEFAULT_DUCKING_PREFERENCES);
    controller.micToggle = vi.fn().mockResolvedValue("unavailable");
    const context = setAppContextForTest({ audio: audioFixture() });
    render(DuckingSettings, { props: { controller }, context });
    const micCheckbox = screen.getByLabelText("ducking.micSource.enable") as HTMLInputElement;
    await fireEvent.click(micCheckbox);
    await waitFor(() => {
      expect(screen.getByText("ducking.micSource.denied.unavailable")).toBeTruthy();
    });
    expect(micCheckbox.checked).toBe(false);
    controller.dispose();
  });

  it("an unavailable microphone on a server that answers HTTPS names the https address", async () => {
    const controller = newController(DEFAULT_DUCKING_PREFERENCES);
    controller.micToggle = vi.fn().mockResolvedValue("unavailable");
    const tls = {
      enabled: true,
      source: "self-signed" as const,
      https_origins: [window.location.origin.replace("http:", "https:")],
      alpn: ["h2", "http/1.1"],
    };
    const context = setAppContextForTest({
      audio: audioFixture(),
      tls,
      t: (key, params) => (params ? `${key} ${JSON.stringify(params)}` : key),
    });
    render(DuckingSettings, { props: { controller }, context });
    await fireEvent.click(screen.getByLabelText("ducking.micSource.enable"));
    await waitFor(() => {
      expect(
        screen.getByText(
          `ducking.micSource.denied.unavailableHttpsAvailable ${JSON.stringify({ url: tls.https_origins[0] + "/" })}`,
        ),
      ).toBeTruthy();
    });
    controller.dispose();
  });

  it("on an https page an unavailable microphone keeps the generic message, never an https address", async () => {
    const controller = newController(DEFAULT_DUCKING_PREFERENCES);
    controller.micToggle = vi.fn().mockResolvedValue("unavailable");
    const tls = {
      enabled: true,
      source: "self-signed" as const,
      https_origins: ["https://play.example"],
      alpn: ["h2", "http/1.1"],
    };
    const context = setAppContextForTest({ audio: audioFixture(), tls });
    vi.stubGlobal("location", { ...window.location, protocol: "https:" });
    try {
      render(DuckingSettings, { props: { controller }, context });
      await fireEvent.click(screen.getByLabelText("ducking.micSource.enable"));
      await waitFor(() => {
        expect(screen.getByText("ducking.micSource.denied.unavailable")).toBeTruthy();
      });
    } finally {
      vi.unstubAllGlobals();
      controller.dispose();
    }
  });

  it("reflects the OS monitor's live status", async () => {
    const controller = newController(DEFAULT_DUCKING_PREFERENCES);
    const context = setAppContextForTest({ audio: audioFixture() });
    render(DuckingSettings, { props: { controller }, context });
    expect(screen.getByText("ducking.osSource.status.connecting")).toBeTruthy();
    controller.dispose();
  });

  it("shows the loopback-ws hint on an https page while the OS monitor is not connected, never on http", async () => {
    const controller = newController(DEFAULT_DUCKING_PREFERENCES);
    const context = setAppContextForTest({ audio: audioFixture() });
    const first = render(DuckingSettings, { props: { controller }, context });
    expect(screen.queryByText("ducking.osSource.httpsHint")).toBeNull();
    first.unmount();
    vi.stubGlobal("location", { ...window.location, protocol: "https:" });
    try {
      render(DuckingSettings, { props: { controller }, context });
      expect(screen.getByText("ducking.osSource.httpsHint")).toBeTruthy();
    } finally {
      vi.unstubAllGlobals();
      controller.dispose();
    }
  });

  it("the depth slider forwards its value to audio.duck.setDepth and never touches the mirror", async () => {
    const controller = newController(DEFAULT_DUCKING_PREFERENCES);
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
