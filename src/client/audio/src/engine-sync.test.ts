// @vitest-environment node
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { AssetResolver, type AudioStateEngine, type PlayingTrack } from "@shadowcat/core";
import { AudioEngine, SYNC_TICK_MS, type AudioEngineOpts } from "./engine";
import { setMediaElementFactory } from "./track-player";
import { timerHarness, visibilityHarness } from "./__fixtures__/engineHarness";
import { stubAudioContext, stubMediaElement, stubWasmDecoder } from "./__fixtures__/stubContext";
import type { MediaElementLike } from "./context";

function entry(id: string, over: Partial<PlayingTrack> = {}): PlayingTrack {
  return {
    id,
    playlist: null,
    trackIndex: 0,
    asset: "a1",
    channel: "music",
    gain: 1,
    loop: false,
    startedAt: 0,
    pausedAt: null,
    ...over,
  };
}

function state(...playing: PlayingTrack[]): AudioStateEngine {
  return { playing, shuffleSeed: 0 };
}

/** The harness every case builds an engine from: the clock the tests advance, the manual
 * timer and the scripted visibility source, plus every element the engine created. */
function rig(over: Partial<AudioEngineOpts> = {}) {
  const clock = { ms: 0 };
  const timer = timerHarness();
  const visibility = visibilityHarness();
  const elements: MediaElementLike[] = [];
  setMediaElementFactory(() => {
    const el = stubMediaElement();
    elements.push(el);
    return el;
  });
  const engine = new AudioEngine({
    resolver: new AssetResolver(),
    serverNow: () => clock.ms,
    transport: vi.fn(),
    createContext: () => stubAudioContext(),
    createOggOpusDecoder: async () => stubWasmDecoder(),
    raf: () => 1,
    caf: () => {},
    timer,
    visibility,
    ...over,
  });
  return { engine, clock, timer, visibility, elements };
}

beforeEach(() => {
  setMediaElementFactory(() => stubMediaElement());
});
afterEach(() => vi.restoreAllMocks());

describe("AudioEngine synchronisation tick", () => {
  it("schedules nothing before unlock and one SYNC_TICK_MS period after it", async () => {
    const { engine, timer } = rig();
    expect(timer.periodMs()).toBeNull();
    await engine.unlock();
    expect(timer.periodMs()).toBe(SYNC_TICK_MS);
  });

  it("re-applies the latest state on every tick: a position change that arrives only through the tick is applied", async () => {
    const { engine, clock, timer, elements } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    expect(elements).toHaveLength(1);
    expect(elements[0].currentTime).toBe(0);
    // The server clock advances 10 s with no audio-state change and the element did not move.
    clock.ms = 10_000;
    timer.fire();
    expect(elements[0].currentTime).toBeCloseTo(10, 5);
  });

  it("a tick starts no second player for an entry the engine already holds", async () => {
    const { engine, timer, elements } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    timer.fire();
    timer.fire();
    expect(elements).toHaveLength(1);
  });

  it("replays the latest state at unlock through the same source the tick uses", async () => {
    const { engine, clock, timer, elements } = rig();
    engine.applyState(state(entry("e1")));
    engine.applyState(state(entry("e1"), entry("e2", { asset: "a2" })));
    expect(elements).toHaveLength(0);
    clock.ms = 3_000;
    await engine.unlock();
    expect(elements).toHaveLength(2);
    expect(elements[0].currentTime).toBeCloseTo(3, 5);
    timer.fire();
    expect(elements).toHaveLength(2);
  });

  it("a page returning to the foreground re-applies the latest state; going hidden does not", async () => {
    const { engine, clock, visibility, elements } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    clock.ms = 20_000;
    visibility.set(false);
    expect(elements[0].currentTime).toBe(0);
    visibility.set(true);
    expect(elements[0].currentTime).toBeCloseTo(20, 5);
  });

  it("after dispose the timer is cleared, the observer is removed and nothing re-applies", async () => {
    const { engine, clock, timer, visibility, elements } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    engine.dispose();
    expect(timer.cleared()).toBe(1);
    expect(timer.periodMs()).toBeNull();
    expect(visibility.observers()).toBe(0);
    clock.ms = 30_000;
    timer.fire();
    visibility.set(true);
    expect(elements[0].currentTime).toBe(0);
  });

  it("the degraded no-Web-Audio engine ticks too", async () => {
    const { engine, clock, timer, elements } = rig({
      createContext: () => {
        throw new Error("no web audio");
      },
    });
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    expect(timer.periodMs()).toBe(SYNC_TICK_MS);
    clock.ms = 12_000;
    timer.fire();
    expect(elements[0].currentTime).toBeCloseTo(12, 5);
  });
});
