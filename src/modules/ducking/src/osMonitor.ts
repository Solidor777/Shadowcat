import type { Logger } from "@shadowcat/core";
import type { DuckSink } from "./keySource";
import { NULL_SINK } from "./keySource";


/** Peak threshold above which a watched process counts as "talking". */
export const DEFAULT_THRESHOLD = 0.02;
/** Hangover held after the peak drops back under threshold, ms (same shape as the mic
 * source's hangover). */
const HANGOVER_MS = 400;
/** Reconnect backoff schedule, ms — capped, repeating the last entry. */
const RECONNECT_BACKOFF_MS = [500, 1000, 2000, 5000];
/** Failed connection attempts before the status reports "not running". */
const NOT_RUNNING_AFTER_ATTEMPTS = 3;

/** The monitor's `hello` frame. */
interface HelloFrame {
  /** Frame discriminant. */
  type: "hello";
  /** The monitor process's OS, for diagnostics. */
  os: string;
  /** Whether this OS/OS-version has a working backend. */
  supported: boolean;
  /** Player-presentable reason, present when `supported` is `false`. */
  reason?: string;
}
/** One watched session's reading, as the `levels` frame reports it. */
interface SessionReading {
  /** The process basename, already watch-filtered server-side. */
  process: string;
  /** Peak output level, clamped to `[0, 1]`. */
  peak: number;
}
/** The monitor's `levels` frame. */
interface LevelsFrame {
  /** Frame discriminant. */
  type: "levels";
  /** Already watch-filtered per-process peak readings. */
  sessions: SessionReading[];
}

/** `OsMonitorSource`'s externally observable connection status: "connected", "not
 * running", or "unsupported on this OS" as the monitor reports. */
export type OsMonitorStatus = "connecting" | "connected" | "not-running" | "unsupported";

/** Constructor options for {@link OsMonitorSource}. */
export interface OsMonitorSourceOptions {
  /** Localhost port `shadowcat audio-monitor` is expected to listen on. */
  port: number;
  /** Initial watched-process substrings, sent as the first `watch` frame right after
   * connect (the monitor itself already seeded `--watch`, but a client-configured list set
   * before the monitor was even started must still apply once connected). */
  watch: string[];
  /** Diagnostic sink. */
  logger: Logger;
  /** Injectable `WebSocket` constructor (tests supply a fake); defaults to the global
   * `WebSocket`. */
  createSocket?: (url: string) => WebSocket;
}

/**
 * Browser-side client for the `shadowcat audio-monitor` localhost WebSocket: connects to
 * `ws://127.0.0.1:<port>/levels`, reconnects with backoff, and reports demand 1 whenever any
 * (already server-filtered) watched session's peak exceeds `threshold`, held for
 * `HANGOVER_MS` after it drops back.
 */
export class OsMonitorSource {
  /** Localhost port to connect to. */
  private readonly port: number;
  /** The live watch list, resent as a `watch` frame on every reconnect/`setWatch`. */
  private watch: string[];
  /** Diagnostic sink. */
  private readonly logger: Logger;
  /** Injected `WebSocket` constructor (real or fake). */
  private readonly createSocket: (url: string) => WebSocket;
  /** The sink demand is forwarded to; replaceable via `setSink`. */
  private sink: DuckSink = NULL_SINK;
  /** Peak threshold above which a watched session counts as "talking". */
  private threshold = DEFAULT_THRESHOLD;
  /** The active socket, or `null` while disconnected. */
  private socket: WebSocket | null = null;
  /** Consecutive failed connection attempts since the last successful `onopen`. */
  private attempts = 0;
  /** The pending reconnect timer, or `null` when none is scheduled. */
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  /** The pending hangover timer, or `null` when not currently in hangover. */
  private hangoverTimer: ReturnType<typeof setTimeout> | null = null;
  /** Whether `start()` has been called without a matching `stop()`. */
  private started = false;
  /** The current connection status (the value `getStatus()` returns). */
  private status: OsMonitorStatus = "connecting";
  /** Subscribers registered via `onStatusChange`. */
  private readonly statusListeners = new Set<(status: OsMonitorStatus) => void>();

  /**
   * Constructs an OS audio-session monitor source.
   * @param opts Construction options — port, initial watch list, and diagnostics.
   * @example
   * ```
   * const source = new OsMonitorSource({ port: 31998, watch: ["discord"], logger: { debug() {}, warn() {}, error() {} } });
   * ```
   */
  constructor(opts: OsMonitorSourceOptions) {
    this.port = opts.port;
    this.watch = opts.watch;
    this.logger = opts.logger;
    this.createSocket = opts.createSocket ?? ((url) => new WebSocket(url));
  }

  /**
   * Begins connecting; a no-op if already started.
   * @example
   * ```
   * const source = new OsMonitorSource({ port: 31998, watch: ["discord"], logger: { debug() {}, warn() {}, error() {} } });
   * source.start();
   * ```
   */
  start(): void {
    if (this.started) return;
    this.started = true;
    this.connect();
  }

  /**
   * Tears down the connection and any pending timers; resets demand to 0.
   * @example
   * ```
   * const source = new OsMonitorSource({ port: 31998, watch: ["discord"], logger: { debug() {}, warn() {}, error() {} } });
   * source.stop();
   * ```
   */
  stop(): void {
    this.started = false;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    if (this.hangoverTimer) clearTimeout(this.hangoverTimer);
    this.socket?.close();
    this.socket = null;
    this.sink.set(0);
  }

  /**
   * Replaces the sink demand is forwarded to (the integration task wires the real one).
   * @param sink The replacement sink.
   * @example
   * ```
   * const source = new OsMonitorSource({ port: 31998, watch: ["discord"], logger: { debug() {}, warn() {}, error() {} } });
   * source.setSink(NULL_SINK);
   * ```
   */
  setSink(sink: DuckSink): void {
    this.sink = sink;
  }

  /**
   * Replaces the peak threshold used to derive demand (a settings change).
   * @param threshold The replacement peak threshold.
   * @example
   * ```
   * const source = new OsMonitorSource({ port: 31998, watch: ["discord"], logger: { debug() {}, warn() {}, error() {} } });
   * source.setThreshold(0.05);
   * ```
   */
  setThreshold(threshold: number): void {
    this.threshold = threshold;
  }

  /**
   * Replaces the live watch list, sending a `watch` frame immediately if connected.
   * @param watch The replacement watched-process substring list.
   * @example
   * ```
   * const source = new OsMonitorSource({ port: 31998, watch: ["discord"], logger: { debug() {}, warn() {}, error() {} } });
   * source.setWatch(["discord", "teams"]);
   * ```
   */
  setWatch(watch: string[]): void {
    this.watch = watch;
    if (this.socket && this.socket.readyState === WebSocket.OPEN) {
      this.socket.send(JSON.stringify({ type: "watch", names: watch }));
    }
  }

  /**
   * Subscribes to connection-status changes.
   * @param cb Called with the new status on every change.
   * @returns An unsubscribe function.
   * @example
   * ```
   * const source = new OsMonitorSource({ port: 31998, watch: ["discord"], logger: { debug() {}, warn() {}, error() {} } });
   * const off = source.onStatusChange((status) => console.log(status));
   * ```
   */
  onStatusChange(cb: (status: OsMonitorStatus) => void): () => void {
    this.statusListeners.add(cb);
    return () => this.statusListeners.delete(cb);
  }

  /**
   * The current connection status.
   * @returns The current `OsMonitorStatus`.
   * @example
   * ```
   * const source = new OsMonitorSource({ port: 31998, watch: ["discord"], logger: { debug() {}, warn() {}, error() {} } });
   * const status = source.getStatus();
   * ```
   */
  getStatus(): OsMonitorStatus {
    return this.status;
  }

  /**
   * Updates `status` and notifies subscribers, if it actually changed.
   * @param status The new status.
   * @example
   * ```
   * // private method; not part of the public API — called from connect()'s event handlers
   * this.setStatus("connected");
   * ```
   */
  private setStatus(status: OsMonitorStatus): void {
    if (this.status === status) return;
    this.status = status;
    for (const cb of this.statusListeners) cb(status);
  }

  /**
   * Opens a new socket to `ws://127.0.0.1:<port>/levels` and wires its event handlers.
   * @example
   * ```
   * // private method; not part of the public API — called from start() and the reconnect timer
   * this.connect();
   * ```
   */
  private connect(): void {
    if (!this.started) return;
    const socket = this.createSocket(`ws://127.0.0.1:${this.port}/levels`);
    this.socket = socket;
    socket.onopen = () => {
      this.attempts = 0;
      if (this.watch.length > 0) {
        socket.send(JSON.stringify({ type: "watch", names: this.watch }));
      }
    };
    socket.onmessage = (event) => {
      this.handleMessage(String(event.data));
    };
    socket.onclose = () => {
      if (!this.started) return;
      this.attempts += 1;
      this.setStatus(this.attempts >= NOT_RUNNING_AFTER_ATTEMPTS ? "not-running" : "connecting");
      const delay = RECONNECT_BACKOFF_MS[Math.min(this.attempts - 1, RECONNECT_BACKOFF_MS.length - 1)];
      this.reconnectTimer = setTimeout(() => this.connect(), delay);
    };
    socket.onerror = () => {
      this.logger.warn("audio-monitor socket error");
    };
  }

  /**
   * Parses and dispatches one incoming WebSocket message.
   * @param raw The raw `MessageEvent.data`, coerced to a string.
   * @example
   * ```
   * // private method; not part of the public API — called from connect()'s onmessage handler
   * this.handleMessage('{"type":"hello","os":"linux","supported":true}');
   * ```
   */
  private handleMessage(raw: string): void {
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      return;
    }
    if (isHelloFrame(parsed)) {
      this.setStatus(parsed.supported ? "connected" : "unsupported");
      return;
    }
    if (!isLevelsFrame(parsed)) return;
    // Sessions arriving here are already watch-filtered server-side: this class
    // only evaluates peak against threshold, never re-matches process names.
    const talking = parsed.sessions.some((s) => s.peak > this.threshold);
    if (talking) {
      if (this.hangoverTimer) {
        clearTimeout(this.hangoverTimer);
        this.hangoverTimer = null;
      }
      this.sink.set(1);
    } else if (!this.hangoverTimer) {
      this.hangoverTimer = setTimeout(() => {
        this.hangoverTimer = null;
        this.sink.set(0);
      }, HANGOVER_MS);
    }
  }
}

/** The minimal shape probed to read a parsed frame's `type` discriminant before it is known
 * which frame kind (if any) `v` actually is. */
interface FrameDiscriminant {
  /** The frame-kind discriminant, if present. */
  type?: unknown;
}

/**
 * Narrows `v` to a `hello` frame.
 * @param v The parsed JSON value to check.
 * @returns Whether `v` is a `hello` frame.
 * @example
 * ```
 * const parsed: unknown = JSON.parse('{"type":"hello"}');
 * if (isHelloFrame(parsed)) console.log(parsed.supported);
 * ```
 */
function isHelloFrame(v: unknown): v is HelloFrame {
  return typeof v === "object" && v !== null && (v as FrameDiscriminant).type === "hello";
}
/**
 * Narrows `v` to a `levels` frame.
 * @param v The parsed JSON value to check.
 * @returns Whether `v` is a `levels` frame.
 * @example
 * ```
 * const parsed: unknown = JSON.parse('{"type":"levels","sessions":[]}');
 * if (isLevelsFrame(parsed)) console.log(parsed.sessions.length);
 * ```
 */
function isLevelsFrame(v: unknown): v is LevelsFrame {
  return (
    typeof v === "object" &&
    v !== null &&
    (v as FrameDiscriminant).type === "levels" &&
    Array.isArray((v as LevelsFrame).sessions)
  );
}
