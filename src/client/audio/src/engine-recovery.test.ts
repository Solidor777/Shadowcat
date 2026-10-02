// @vitest-environment node
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { AssetResolver, type AudioStateEngine, type PlayingTrack } from "@shadowcat/core";
import { AudioEngine, RESUME_WAIT_MS } from "./engine";
import { SYNC_SEEK_THRESHOLD_SECS, setMediaElementFactory } from "./track-player";
import { timerHarness, visibilityHarness } from "./__fixtures__/engineHarness";
import {
  stubAudioContext,
  stubBuffer,
  stubMediaElement,
  stubWasmDecoder,
  wavBytes,
  type StubAudioContext,
  type StubMediaElement,
} from "./__fixtures__/stubContext";

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

async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

/** Everything one recovery case needs: the engine over a scripted context, the clock, the
 * timer and visibility harnesses, a counter of `serverNow` reads (one per `applyState` on the
 * graph path) and every element the engine created. */
function rig() {
  vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(wavBytes()));
  const ctx: StubAudioContext = stubAudioContext();
  ctx.decodeAudioData = async () => stubBuffer(100);
  const clock = { ms: 0, reads: 0 };
  const timer = timerHarness();
  const visibility = visibilityHarness();
  const elements: StubMediaElement[] = [];
  setMediaElementFactory(() => {
    const el = stubMediaElement();
    elements.push(el);
    return el;
  });
  const engine = new AudioEngine({
    resolver: new AssetResolver(),
    serverNow: () => {
      clock.reads++;
      return clock.ms;
    },
    transport: vi.fn(),
    createContext: () => ctx,
    createOggOpusDecoder: async () => stubWasmDecoder(),
    raf: () => 1,
    caf: () => {},
    timer,
    visibility,
  });
  const notified = vi.fn();
  engine.subscribe(notified);
  return { ctx, clock, timer, visibility, elements, engine, notified };
}

/** Buffer sources started and not stopped. */
function liveSources(ctx: StubAudioContext): number {
  return ctx.sources.filter((s) => (s.stop as ReturnType<typeof vi.fn>).mock.calls.length === 0).length;
}

beforeEach(() => {
  setMediaElementFactory(() => stubMediaElement());
});
afterEach(() => vi.restoreAllMocks());

describe("AudioEngine interruption and suspension recovery", () => {
  it("(1) running -> interrupted -> running: one resume, needsGesture false, one live node, audible position at the server position", async () => {
    const { ctx, clock, engine } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1", { loop: true })));
    await settle();
    expect(liveSources(ctx)).toBe(1);
    const resumesBefore = ctx.resumeCalls;
    // The context is interrupted for 30 s: its clock stops while the server clock runs on.
    clock.ms = 30_000;
    ctx.setState("interrupted");
    await settle();
    expect(ctx.resumeCalls - resumesBefore).toBe(1);
    expect(ctx.state).toBe("running");
    expect(engine.needsGesture).toBe(false);
    expect(liveSources(ctx)).toBe(1);
    const restarted = ctx.sources[ctx.sources.length - 1];
    const [, offset] = (restarted.start as ReturnType<typeof vi.fn>).mock.calls[0] as [number, number];
    expect(Math.abs(offset - 30)).toBeLessThanOrEqual(SYNC_SEEK_THRESHOLD_SECS);
  });

  it("(2) suspended and resume() rejects: needsGesture true with a notification; unlock() runs it, clears the flag, no duplicate node", async () => {
    const { ctx, engine, notified } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1", { loop: true })));
    await settle();
    ctx.resumeMode = "reject";
    notified.mockClear();
    ctx.setState("suspended");
    await settle();
    expect(engine.needsGesture).toBe(true);
    expect(notified).toHaveBeenCalled();
    ctx.resumeMode = "resolve";
    await engine.unlock();
    expect(ctx.state).toBe("running");
    expect(engine.needsGesture).toBe(false);
    expect(liveSources(ctx)).toBe(1);
  });

  it("(3) resume() pends forever: after RESUME_WAIT_MS needsGesture is true", async () => {
    const { ctx, timer, engine } = rig();
    ctx.resumeMode = "pend";
    const unlocking = engine.unlock();
    await settle();
    expect(timer.pendingTimeoutMs()).toEqual([RESUME_WAIT_MS]);
    expect(engine.needsGesture).toBe(false);
    timer.fireTimeouts();
    await unlocking;
    expect(engine.needsGesture).toBe(true);
  });

  it("(4) hidden -> visible while suspended: one resume() and one applyState", async () => {
    const { ctx, clock, visibility, engine } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    ctx.resumeMode = "pend";
    ctx.setState("suspended");
    await settle();
    visibility.set(false);
    const resumes = ctx.resumeCalls;
    const reads = clock.reads;
    visibility.set(true);
    await settle();
    expect(ctx.resumeCalls - resumes).toBe(1);
    expect(clock.reads - reads).toBe(1);
  });

  it("(5) statechange to running: no resume(), one applyState", async () => {
    const { ctx, clock, engine } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    ctx.resumeMode = "pend";
    ctx.setState("suspended");
    await settle();
    const resumes = ctx.resumeCalls;
    const reads = clock.reads;
    ctx.setState("running");
    await settle();
    expect(ctx.resumeCalls).toBe(resumes);
    expect(clock.reads - reads).toBe(1);
    expect(engine.needsGesture).toBe(false);
  });

  it("(6) after dispose no state change recovers anything", async () => {
    const { ctx, clock, engine } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    engine.dispose();
    const resumes = ctx.resumeCalls;
    const reads = clock.reads;
    ctx.setState("suspended");
    ctx.setState("running");
    await settle();
    expect(ctx.resumeCalls).toBe(resumes);
    expect(clock.reads).toBe(reads);
  });

  it("recovery never starts a second player for the entry already held (streaming)", async () => {
    const { ctx, elements, engine } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    expect(elements).toHaveLength(1);
    ctx.setState("interrupted");
    await settle();
    ctx.setState("suspended");
    await settle();
    expect(elements).toHaveLength(1);
  });

  it("a gesture after a refused play() retries the element and clears needsGesture", async () => {
    const { elements, engine } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    const el = elements[0];
    el.paused = true;
    const refusal = new Error("play() failed because the user didn't interact with the document first");
    refusal.name = "NotAllowedError";
    el.play = vi.fn(() => Promise.reject(refusal));
    engine.applyState(state(entry("e1")));
    await settle();
    expect(engine.needsGesture).toBe(true);
    el.play = vi.fn(async () => {
      el.paused = false;
    });
    await engine.unlock();
    await settle();
    expect(el.play).toHaveBeenCalledTimes(1);
    expect(engine.needsGesture).toBe(false);
  });
});
