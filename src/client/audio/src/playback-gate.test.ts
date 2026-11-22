// @vitest-environment node
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { AssetResolver, type SceneAudibility } from "@shadowcat/core";
import { FallbackTrackPlayer } from "./fallback-player";
import { PlaybackGate } from "./playback-gate";
import { setMediaElementFactory } from "./track-player";
import { entry, rig, settle, state } from "./__fixtures__/engineHarness";
import { quietGate, stubMediaElement, type StubMediaElement } from "./__fixtures__/stubContext";

/** The browser's autoplay refusal. */
function notAllowed(): Error {
  const error = new Error("play() failed because the user didn't interact with the document first");
  error.name = "NotAllowedError";
  return error;
}

/** Make `el.play` reject with `error` and leave the element paused, as a refused play does. */
function refusePlay(el: StubMediaElement, error: Error): ReturnType<typeof vi.fn> {
  const play = vi.fn(() => Promise.reject(error));
  el.play = play;
  return play;
}

beforeEach(() => {
  setMediaElementFactory(() => stubMediaElement());
});
afterEach(() => vi.restoreAllMocks());

describe("PlaybackGate", () => {
  it("starts a paused element once and never restarts it while it plays", () => {
    const gate = quietGate();
    const el = stubMediaElement();
    const play = vi.spyOn(el, "play");
    for (let i = 0; i < 10; i++) gate.play(el);
    expect(play).toHaveBeenCalledTimes(1);
    el.pause();
    gate.play(el);
    expect(play).toHaveBeenCalledTimes(2);
  });

  it("an autoplay refusal reports once, is handled, and is not retried until released", async () => {
    const onBlocked = vi.fn();
    const gate = new PlaybackGate(onBlocked);
    const el = stubMediaElement();
    const play = refusePlay(el, notAllowed());
    for (let i = 0; i < 30; i++) {
      gate.play(el);
      await settle();
    }
    expect(play).toHaveBeenCalledTimes(1);
    expect(onBlocked).toHaveBeenCalledTimes(1);
    gate.release();
    gate.play(el);
    await settle();
    expect(play).toHaveBeenCalledTimes(2);
    expect(onBlocked).toHaveBeenCalledTimes(2);
  });

  it("a rejection that is not an autoplay refusal is handled, asks for no gesture and is retried", async () => {
    const onBlocked = vi.fn();
    const gate = new PlaybackGate(onBlocked);
    const el = stubMediaElement();
    const abort = new Error("interrupted by a call to pause()");
    abort.name = "AbortError";
    const play = refusePlay(el, abort);
    gate.play(el);
    gate.play(el);
    await settle();
    expect(play).toHaveBeenCalledTimes(2);
    expect(onBlocked).not.toHaveBeenCalled();
  });
});

describe("every arm starts its element through the gate", () => {
  it("FallbackTrackPlayer: a playing element is not asked again on later syncs", () => {
    const el = stubMediaElement();
    setMediaElementFactory(() => el);
    const play = vi.spyOn(el, "play");
    const player = new FallbackTrackPlayer(new AssetResolver(), () => ({ gain: 1, muted: false }), entry("e1"), () => {}, quietGate());
    for (let i = 0; i < 20; i++) player.sync(entry("e1"), i * 1000);
    expect(play).toHaveBeenCalledTimes(1);
  });
});

describe("AudioEngine start-up through the gate", () => {
  it("a refused streaming element sets needsGesture, notifies once, and the ticks do not re-ask", async () => {
    const { engine, elements, timer, notified } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    const play = refusePlay(elements[0], notAllowed());
    // The element was already started by applyState; re-pause so the gate sees it paused.
    elements[0].paused = true;
    for (let i = 0; i < 20; i++) {
      timer.fire();
      await settle();
    }
    expect(play).toHaveBeenCalledTimes(1);
    expect(engine.needsGesture).toBe(true);
    expect(notified).toHaveBeenCalledTimes(1);
  });

  it("a degraded carried emitter is started only while paused", async () => {
    const { engine, elements } = rig({
      createContext: () => {
        throw new Error("no web audio");
      },
    });
    await engine.unlock();
    const payload: SceneAudibility = {
      scene: "s1",
      listener: null,
      spatial: true,
      emitters: [{ token: "t1", asset: "a1", gain: 1, pan: 0, muffled: false, loop: true }],
    };
    const first = vi.fn();
    engine.applyAudibility(payload);
    const el = elements[0];
    el.play = first.mockImplementation(async () => {
      el.paused = false;
    });
    el.paused = true;
    for (let i = 0; i < 10; i++) engine.applyAudibility(payload);
    expect(first).toHaveBeenCalledTimes(1);
  });
});
