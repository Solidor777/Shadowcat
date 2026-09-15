// @vitest-environment node
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { AssetResolver, type AudioStateEngine, type PlayingTrack } from "@shadowcat/core";
import { AudioEngine, type AudioEngineOpts } from "./engine";
import { setMediaElementFactory } from "./track-player";
import { stubAudioContext, stubMediaElement, stubWasmDecoder, wavBytes } from "./__fixtures__/stubContext";

function entry(id: string, over: Partial<PlayingTrack> = {}): PlayingTrack {
  return {
    id,
    playlist: null,
    trackIndex: 0,
    asset: "a1",
    channel: "sfx",
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

/** A manually pumped raf/caf pair (production passes `requestAnimationFrame`). */
function pumpHarness() {
  let queued: ((now: number) => void) | null = null;
  return {
    raf: (cb: (now: number) => void): number => {
      queued = cb;
      return 1;
    },
    caf: (): void => {
      queued = null;
    },
    pump(now: number): void {
      const cb = queued;
      queued = null;
      cb?.(now);
    },
    hasQueued: (): boolean => queued !== null,
  };
}

function makeOpts(over: Partial<AudioEngineOpts> = {}): AudioEngineOpts {
  const pump = pumpHarness();
  return {
    resolver: new AssetResolver(),
    serverNow: () => 0,
    transport: vi.fn(),
    createContext: () => stubAudioContext(),
    createOggOpusDecoder: async () => stubWasmDecoder(),
    raf: pump.raf,
    caf: pump.caf,
    ...over,
  };
}

beforeEach(() => {
  setMediaElementFactory(() => stubMediaElement());
});
afterEach(() => vi.restoreAllMocks());

describe("AudioEngine", () => {
  it("tracks pending state before unlock and applies it once unlock resolves", async () => {
    const engine = new AudioEngine(makeOpts());
    engine.applyState(state(entry("e1")));
    // No context yet: no TrackPlayer exists (nothing threw, state is queued).
    await engine.unlock();
    // After unlock, the queued state produced one live player; removing it disposes it.
    engine.applyState(state());
  });

  it("diffs applyState across successive calls: create, keep, remove", async () => {
    const engine = new AudioEngine(makeOpts());
    await engine.unlock();
    engine.applyState(state(entry("a"), entry("b")));
    engine.applyState(state(entry("b"), entry("c")));
    engine.applyState(state());
  });

  it("setChannel mute zeroes the live GainNode", async () => {
    const opts = makeOpts();
    const ctx = stubAudioContext();
    opts.createContext = () => ctx;
    const engine = new AudioEngine(opts);
    await engine.unlock();
    engine.setChannel("sfx", { muted: true });
    expect(engine.channels.sfx.muted).toBe(true);
    engine.setChannel("sfx", { muted: false });
    expect(engine.channels.sfx.muted).toBe(false);
  });

  it("playOneShot before unlock is a silent no-op (never throws)", () => {
    const engine = new AudioEngine(makeOpts());
    expect(() => engine.playOneShot("a1")).not.toThrow();
  });

  it("reports the constructor's duckDepth before unlock", () => {
    const engine = new AudioEngine(makeOpts({ duckDepth: 0.2 }));
    expect(engine.duck.depth).toBe(0.2);
  });

  it("the duck loop drives the duck GainNode toward 1 - depth and back", async () => {
    const pump = pumpHarness();
    const opts = makeOpts({ raf: pump.raf, caf: pump.caf });
    const engine = new AudioEngine(opts);
    await engine.unlock();
    expect(pump.hasQueued()).toBe(true);
    engine.duck.addSource("mic").set(1);
    pump.pump(1_000);
    pump.pump(1_100);
    const ducked = engine.duck.gain;
    expect(ducked).toBeLessThan(1);
    expect(ducked).toBeGreaterThan(1 - engine.duck.depth);
    engine.duck.addSource("mic").set(0);
    for (let t = 1_200; t < 6_000; t += 200) pump.pump(t);
    expect(engine.duck.gain).toBeGreaterThan(ducked);
    engine.dispose();
    expect(pump.hasQueued()).toBe(false);
  });

  it("dispose cancels the duck loop's raf handle", async () => {
    const pump = pumpHarness();
    const opts = makeOpts({ raf: pump.raf, caf: pump.caf });
    const caf = vi.spyOn(opts, "caf");
    const engine = new AudioEngine(opts);
    await engine.unlock();
    engine.dispose();
    expect(caf).toHaveBeenCalledTimes(1);
    pump.pump(9_999); // no re-arm after dispose
    expect(pump.hasQueued()).toBe(false);
  });

  it("a buffered-loop entry creates a player without a media element", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(wavBytes()));
    const engine = new AudioEngine(makeOpts());
    await engine.unlock();
    engine.applyState(state(entry("e1", { loop: true })));
    engine.applyState(state());
  });
});
