import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import StatusBar from "./StatusBar.svelte";

function ctxWithAudio(audio: {
  unlock: () => Promise<void>;
  setChannel: (id: string, patch: { muted?: boolean }) => void;
  channels: Record<string, { gain: number; muted: boolean }>;
  needsGesture?: boolean;
}) {
  return setAppContextForTest({ role: "gm", audio: audio as never });
}

describe("StatusBar audio control", () => {
  it("renders the unlock button, calls unlock on click, then shows the mute toggle", async () => {
    const unlock = vi.fn().mockResolvedValue(undefined);
    render(StatusBar, {
      context: ctxWithAudio({
        unlock,
        setChannel: () => {},
        channels: { master: { gain: 1, muted: false } },
      }),
    });
    const button = screen.getByTestId("audio-unlock");
    expect(screen.queryByTestId("audio-mute-toggle")).toBeNull();
    await fireEvent.click(button);
    expect(unlock).toHaveBeenCalledTimes(1);
    expect(await screen.findByTestId("audio-mute-toggle")).toBeTruthy();
    expect(screen.queryByTestId("audio-unlock")).toBeNull();
  });

  it("the mute toggle calls setChannel('master', { muted: true }) from the unmuted state", async () => {
    const setChannel = vi.fn();
    render(StatusBar, {
      context: ctxWithAudio({
        unlock: async () => {},
        setChannel,
        channels: { master: { gain: 1, muted: false } },
      }),
    });
    await fireEvent.click(screen.getByTestId("audio-unlock"));
    await fireEvent.click(await screen.findByTestId("audio-mute-toggle"));
    expect(setChannel).toHaveBeenCalledWith("master", { muted: true });
  });

  it("needsGesture true keeps the unlock button after an unlock, and clicking it calls unlock again", async () => {
    const unlock = vi.fn().mockResolvedValue(undefined);
    render(StatusBar, {
      context: ctxWithAudio({
        unlock,
        setChannel: () => {},
        channels: { master: { gain: 1, muted: false } },
        needsGesture: true,
      }),
    });
    await fireEvent.click(screen.getByTestId("audio-unlock"));
    expect(unlock).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId("audio-unlock")).toBeTruthy();
    expect(screen.queryByTestId("audio-mute-toggle")).toBeNull();
    await fireEvent.click(screen.getByTestId("audio-unlock"));
    expect(unlock).toHaveBeenCalledTimes(2);
  });

  it("an unlock that clears needsGesture swaps to the mute toggle", async () => {
    const audio = {
      needsGesture: true,
      unlock: vi.fn(async () => {
        audio.needsGesture = false;
      }),
      setChannel: () => {},
      channels: { master: { gain: 1, muted: false } },
    };
    render(StatusBar, { context: ctxWithAudio(audio) });
    await fireEvent.click(screen.getByTestId("audio-unlock"));
    expect(await screen.findByTestId("audio-mute-toggle")).toBeTruthy();
    expect(screen.queryByTestId("audio-unlock")).toBeNull();
  });
});
