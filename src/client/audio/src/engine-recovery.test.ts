// @vitest-environment node
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { RESUME_WAIT_MS } from "./engine";
import { SYNC_SEEK_THRESHOLD_SECS, setMediaElementFactory } from "./track-player";
import { entry, rig, settle, state } from "./__fixtures__/engineHarness";
import { stubMediaElement, type StubAudioContext } from "./__fixtures__/stubContext";

/** Buffer sources started and not stopped. */
function liveSources(ctx: StubAudioContext): number {
  return ctx.sources.filter((s) => (s.stop as ReturnType<typeof vi.fn>).mock.calls.length === 0).length;
}

beforeEach(() => {
  setMediaElementFactory(() => stubMediaElement());
});
afterEach(() => vi.restoreAllMocks());

describe("AudioEngine interruption and suspension recovery", () => {
  it("running -> interrupted -> running: one resume, needsGesture false, one live node, audible position at the server position", async () => {
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

  it("suspended and resume() rejects: needsGesture true with a notification; unlock() runs it, clears the flag, no duplicate node", async () => {
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

  it("resume() pends forever: after RESUME_WAIT_MS needsGesture is true", async () => {
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

  it("hidden -> visible while suspended: one resume() and one applyState", async () => {
    const { ctx, clock, visibility, engine } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    ctx.resumeMode = "pend";
    ctx.setState("suspended");
    await settle();
    const resumes = ctx.resumeCalls;
    const reads = clock.reads;
    visibility.set(false);
    await settle();
    expect(ctx.resumeCalls - resumes).toBe(0);
    expect(clock.reads - reads).toBe(0);
    visibility.set(true);
    await settle();
    expect(ctx.resumeCalls - resumes).toBe(1);
    expect(clock.reads - reads).toBe(1);
  });

  it("statechange to running: no resume(), one applyState", async () => {
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

  it("after dispose no state change recovers anything", async () => {
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

  it("a first unlock() whose resume wait outlives dispose() sets no flag, notifies nobody and installs nothing", async () => {
    const raf = vi.fn(() => 1);
    const { ctx, clock, timer, visibility, engine, notified } = rig({ raf });
    engine.applyState(state(entry("e1")));
    ctx.resumeMode = "pend";
    const unlocking = engine.unlock();
    await settle();
    expect(timer.pendingTimeoutMs()).toEqual([RESUME_WAIT_MS]);
    engine.dispose();
    notified.mockClear();
    timer.fireTimeouts();
    await unlocking;
    expect(engine.needsGesture).toBe(false);
    expect(notified).not.toHaveBeenCalled();
    expect(timer.periodMs()).toBeNull();
    expect(visibility.observers()).toBe(0);
    expect(raf).not.toHaveBeenCalled();
    expect(clock.reads).toBe(0);
  });

  it("a later unlock() whose resume wait outlives dispose() sets no flag, notifies nobody and applies no state", async () => {
    const { ctx, clock, timer, engine, notified } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    ctx.resumeMode = "pend";
    ctx.setState("suspended");
    await settle();
    const unlocking = engine.unlock();
    await settle();
    engine.dispose();
    notified.mockClear();
    const reads = clock.reads;
    timer.fireTimeouts();
    await unlocking;
    expect(engine.needsGesture).toBe(false);
    expect(notified).not.toHaveBeenCalled();
    expect(clock.reads).toBe(reads);
  });

  describe("a streaming element under a suspended context, corrected by the next tick", () => {
    /** An engine playing one streaming entry for 4 s, then suspended with a `resume()` that never
     * settles, and the server clock moved on to 10 s. Records every write to the element's
     * `currentTime`; the element sits at 4 s, the position it had when the context stopped.
     * @returns The element, the tick control and the write count. */
    async function suspendedAtFourSeconds() {
      const { ctx, clock, timer, elements, engine } = rig();
      await engine.unlock();
      engine.applyState(state(entry("e1")));
      clock.ms = 4000;
      timer.fire();
      const el = elements[0];
      let position = el.currentTime;
      const writes = { count: 0 };
      Object.defineProperty(el, "currentTime", {
        configurable: true,
        get: () => position,
        set: (value: number) => {
          writes.count++;
          position = value;
        },
      });
      ctx.resumeMode = "pend";
      ctx.setState("suspended");
      await settle();
      clock.ms = 10_000;
      return { el, timer, writes, advanceElementTo: (secs: number) => (position = secs) };
    }

    it("an element whose clock froze with the context (Chromium) is seeked to the server position", async () => {
      const { el, timer, writes } = await suspendedAtFourSeconds();
      expect(el.currentTime).toBeCloseTo(4, 5);
      timer.fire();
      expect(writes.count).toBe(1);
      expect(el.currentTime).toBeCloseTo(10, 5);
    });

    it("an element whose clock kept running (Firefox) is left in position", async () => {
      const { el, timer, writes, advanceElementTo } = await suspendedAtFourSeconds();
      advanceElementTo(10);
      timer.fire();
      expect(writes.count).toBe(0);
      expect(el.currentTime).toBe(10);
      expect(el.playbackRate).toBe(1);
    });
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
