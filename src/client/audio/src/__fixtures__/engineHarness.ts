import { vi } from "vitest";
import { AssetResolver, type AudioStateEngine, type PlayingTrack } from "@shadowcat/core";
import { AudioEngine, type AudioEngineOpts, type AudioTimer, type VisibilitySource } from "../engine";
import { setMediaElementFactory } from "../track-player";
import {
  stubAudioContext,
  stubBuffer,
  stubMediaElement,
  stubWasmDecoder,
  wavBytes,
  type StubAudioContext,
  type StubMediaElement,
} from "./stubContext";

/** `timerHarness`'s return shape: the injectable timer plus the controls a test drives it by. */
export interface TimerHarness extends AudioTimer {
  /** Run the scheduled callback once, as one elapsed period; a no-op while none is scheduled.
   * @returns Nothing. */
  fire(): void;
  /** Run every pending one-shot timeout, as if its delay elapsed.
   * @returns Nothing. */
  fireTimeouts(): void;
  /** The delays of the pending one-shot timeouts, ms.
   * @returns The delays. */
  pendingTimeoutMs(): number[];
  /** The period the engine scheduled, ms, or `null` while nothing is scheduled.
   * @returns The period, or `null`. */
  periodMs(): number | null;
  /** How many `clearInterval` calls the engine made.
   * @returns The count. */
  cleared(): number;
}

/** One scheduled one-shot timeout. */
interface PendingTimeout {
  /** The callback to run when the delay elapses. */
  cb: () => void;
  /** The delay, milliseconds. */
  ms: number;
}

/** A manually fired `AudioTimer`: nothing runs until `fire()`.
 * @returns The harness.
 * @example
 * ```ts
 * import { timerHarness } from "../__fixtures__/engineHarness";
 * timerHarness().periodMs(); // null
 * ```
 */
export function timerHarness(): TimerHarness {
  let callback: (() => void) | null = null;
  let period: number | null = null;
  let clears = 0;
  let nextTimeout = 100;
  const timeouts = new Map<number, PendingTimeout>();
  return {
    setTimeout(cb: () => void, ms: number): number {
      const handle = nextTimeout++;
      timeouts.set(handle, { cb, ms });
      return handle;
    },
    clearTimeout(handle: number): void {
      timeouts.delete(handle);
    },
    fireTimeouts(): void {
      for (const [handle, pending] of [...timeouts]) {
        timeouts.delete(handle);
        pending.cb();
      }
    },
    pendingTimeoutMs: () => [...timeouts.values()].map((t) => t.ms),
    setInterval(cb: () => void, ms: number): number {
      callback = cb;
      period = ms;
      return 7;
    },
    clearInterval(): void {
      callback = null;
      period = null;
      clears++;
    },
    fire(): void {
      callback?.();
    },
    periodMs: () => period,
    cleared: () => clears,
  };
}

/** `visibilityHarness`'s return shape: the injectable source plus the controls a test drives it by. */
export interface VisibilityHarness extends VisibilitySource {
  /** Set the page's visibility and notify every observer.
   * @param visible The new visibility. */
  set(visible: boolean): void;
  /** How many observers are currently subscribed.
   * @returns The count. */
  observers(): number;
}

/** A scripted `VisibilitySource`, visible until `set(false)`.
 * @returns The harness.
 * @example
 * ```ts
 * import { visibilityHarness } from "../__fixtures__/engineHarness";
 * visibilityHarness().isVisible(); // true
 * ```
 */
export function visibilityHarness(): VisibilityHarness {
  let visible = true;
  const observers = new Set<() => void>();
  return {
    isVisible: () => visible,
    subscribe(cb: () => void): () => void {
      observers.add(cb);
      return () => observers.delete(cb);
    },
    set(next: boolean): void {
      visible = next;
      for (const cb of [...observers]) cb();
    },
    observers: () => observers.size,
  };
}

/** A playing entry on the `music` channel at server time 0, playing and not looping.
 * @param id The entry id.
 * @param over Fields to override.
 * @returns The entry.
 * @example
 * ```ts
 * import { entry } from "../__fixtures__/engineHarness";
 * entry("e1", { loop: true }).loop; // true
 * ```
 */
export function entry(id: string, over: Partial<PlayingTrack> = {}): PlayingTrack {
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

/** An `AudioStateEngine` over `playing`.
 * @param playing The playing entries.
 * @returns The state.
 * @example
 * ```ts
 * import { entry, state } from "../__fixtures__/engineHarness";
 * state(entry("e1")).playing.length; // 1
 * ```
 */
export function state(...playing: PlayingTrack[]): AudioStateEngine {
  return { playing, shuffleSeed: 0 };
}

/** Let queued promise continuations and zero-delay timers run.
 * @returns Resolves after several macrotask turns.
 * @example
 * ```ts
 * import { settle } from "../__fixtures__/engineHarness";
 * await settle();
 * ```
 */
export async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

/** The clock `rig` hands the engine as `serverNow`. */
export interface RigClock {
  /** The server time, ms, the engine reads. */
  ms: number;
  /** How many times the engine read `serverNow` (one per `applyState` on the graph path). */
  reads: number;
}

/** `rig`'s return shape. */
export interface EngineRig {
  /** The engine under test. */
  engine: AudioEngine;
  /** The scripted context `createContext` returns (unless a case overrides `createContext`). */
  ctx: StubAudioContext;
  /** The server clock the test advances. */
  clock: RigClock;
  /** The manual timer. */
  timer: TimerHarness;
  /** The scripted visibility source. */
  visibility: VisibilityHarness;
  /** Every media element the engine created, in creation order. */
  elements: StubMediaElement[];
  /** A spy subscribed to the engine's change notifications. */
  notified: ReturnType<typeof vi.fn>;
}

/** Build an `AudioEngine` over a scripted context, a manual timer and a scripted visibility
 * source, with `fetch` answering every request and every created media element recorded. The
 * caller restores mocks (`vi.restoreAllMocks`) after each case.
 * @param over Engine options to replace.
 * @returns The engine and the controls a case drives it by.
 * @example
 * ```ts
 * import { rig } from "../__fixtures__/engineHarness";
 * rig().timer.periodMs(); // null
 * ```
 */
export function rig(over: Partial<AudioEngineOpts> = {}): EngineRig {
  vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(wavBytes()));
  const ctx = stubAudioContext();
  ctx.decodeAudioData = async () => stubBuffer(100);
  const clock: RigClock = { ms: 0, reads: 0 };
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
    ...over,
  });
  const notified = vi.fn();
  engine.subscribe(notified);
  return { engine, ctx, clock, timer, visibility, elements, notified };
}
