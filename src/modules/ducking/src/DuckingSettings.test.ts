import { describe, it, expect, afterEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DuckSourcesController } from "./controller";
import { DEFAULT_DUCKING_PREFERENCES, DUCKING_MIRROR_STORAGE_KEY } from "./duckingMirror";
import DuckingSettings from "./DuckingSettings.svelte";

const logger = { debug() {}, warn() {}, error() {} };

describe("DuckingSettings", () => {
  afterEach(() => {
    localStorage.clear();
  });

  it("renders every control labeled and persists a master-enable toggle", async () => {
    const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
    const context = setAppContextForTest({});
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
    const context = setAppContextForTest({});
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
    const context = setAppContextForTest({});
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
    const context = setAppContextForTest({});
    const onMicToggle = vi.fn().mockResolvedValue("permission-denied");
    render(DuckingSettings, { props: { controller, onMicToggle }, context });
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
    const context = setAppContextForTest({});
    render(DuckingSettings, { props: { controller }, context });
    expect(screen.getByText("ducking.osSource.status.connecting")).toBeTruthy();
    controller.dispose();
  });

  it("the depth slider forwards its value to onDepthChange and never touches the mirror", async () => {
    const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
    const context = setAppContextForTest({});
    const onDepthChange = vi.fn();
    render(DuckingSettings, { props: { controller, depth: 0.5, onDepthChange }, context });
    const slider = screen.getByLabelText("ducking.depth") as HTMLInputElement;
    expect(slider.value).toBe("0.5");
    await fireEvent.input(slider, { target: { value: "0.3" } });
    expect(onDepthChange).toHaveBeenCalledWith(0.3);
    expect(localStorage.getItem(DUCKING_MIRROR_STORAGE_KEY) ?? "").not.toContain("depth");
    controller.dispose();
  });
});
