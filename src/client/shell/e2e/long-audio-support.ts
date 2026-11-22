import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import type { APIRequestContext, BrowserContext, Page } from "@playwright/test";
import { expect, login, openPanel } from "./fixtures";

/** Directory of the committed long-audio fixtures (`manifest.json` and the three Ogg tracks). */
const FIXTURE_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../e2e-fixtures/audio");

/** The fixture manifest: the slot length, the marker base frequency and each track's sequence of
 * semitone codes. */
interface Manifest {
  /** Length of one position-code slot, in seconds. */
  slot_secs: number;
  /** Frequency of semitone 0, in hertz. */
  marker_base_hz: number;
  /** Per track: length in seconds and the marker code of every slot. */
  tracks: Record<string, TrackManifest>;
}

/** One track's entry in the fixture manifest. */
interface TrackManifest {
  /** Length of the track, in seconds. */
  duration_secs: number;
  /** The semitone code of every slot, in order. */
  sequence: number[];
}

/** The media events an element's log records: the ones that separate a stall (`waiting`,
 * `stalled`, `suspend`), an unrequested stop (`pause`), the natural end (`ended`), a seek
 * (`seeking`, `seeked`), a failure (`error`) and the start of output (`play`, `playing`). */
export const LOGGED_MEDIA_EVENTS = [
  "waiting",
  "stalled",
  "suspend",
  "pause",
  "ended",
  "seeking",
  "seeked",
  "error",
  "play",
  "playing",
] as const;

/** One media event an element fired, with the element's state at that instant. */
export interface ElementEvent {
  /** The event name (one of `LOGGED_MEDIA_EVENTS`). */
  type: string;
  /** Wall-clock time of the event, in epoch milliseconds (the host and the page share the clock). */
  atEpochMs: number;
  /** The element's `currentTime` in seconds. */
  currentTime: number;
  /** The element's `readyState`. */
  readyState: number;
  /** The element's `networkState`. */
  networkState: number;
  /** Whether the element was paused. */
  paused: boolean;
}

/** The serialisable argument `installAudioProbes` passes into the page's init script. */
interface ProbeArgs {
  /** Whether `canPlayType` hides Ogg. */
  hide: boolean;
  /** The media event names each element logs. */
  logged: readonly string[];
}

/** An `<audio>` element the page created, with the count of its `ended` and `error` events and
 * the log of every media event in `LOGGED_MEDIA_EVENTS`. */
interface TrackedElement {
  /** The element. */
  el: HTMLAudioElement;
  /** `ended` events seen. */
  ended: number;
  /** `error` events seen. */
  errors: number;
  /** Every logged media event, in firing order. */
  events: ElementEvent[];
}

/** The observers `installAudioProbes` installs on the page's `window`. */
interface PageProbes {
  /** Every `<audio>` element created. */
  __scElements: TrackedElement[];
  /** Every `AudioContext` constructed. */
  __scContexts: BaseAudioContext[];
  /** The analyser tapped onto each context's destination. */
  __scAnalysers: Map<BaseAudioContext, AnalyserNode>;
}

/** The parsed fixture manifest. */
export const MANIFEST: Manifest = JSON.parse(readFileSync(path.join(FIXTURE_DIR, "manifest.json"), "utf8")) as Manifest;

/** The fixture track names, in the order the sampled legs visit them. */
export const TRACK_NAMES = ["ambient-bed", "cave-atmosphere", "battle-bed"] as const;

/** One fixture track's name. */
export type TrackName = (typeof TRACK_NAMES)[number];

/** The bytes of a fixture track's Ogg file.
 * @param name - The track name.
 * @returns The file bytes.
 * @example
 * ```
 * const bytes = trackBytes("ambient-bed");
 * ```
 */
export function trackBytes(name: TrackName): Buffer {
  return readFileSync(path.join(FIXTURE_DIR, `${name}.ogg`));
}

/** The server's `MAX_RANGE_SLICE`: no asset response carries more bytes than this. */
export const MAX_RANGE_SLICE_BYTES = 2 * 1024 * 1024;

/** The position drift, in seconds, beyond which the audio engine hard-seeks an element
 * (`SYNC_SEEK_THRESHOLD_SECS` in `@shadowcat/audio`). */
export const SYNC_SEEK_THRESHOLD_SECS = 0.25;

/** The failure texts an engine reports for a request its own media element cancels (a seek
 * replaces the request in flight; Firefox also cancels a request whose parsed data it already
 * holds). Any other failure text on an asset request fails a leg. A server-side cut of an
 * HTTP/1.1 response reads as `net::ERR_CONTENT_LENGTH_MISMATCH` or `net::ERR_CONNECTION_RESET` in
 * Chromium and `NS_ERROR_NET_PARTIAL_TRANSFER` or `NS_ERROR_NET_RESET` in Firefox; none is listed
 * here, so such a cut fails a leg. WebKit reports its element's cancel as `Load request cancelled`
 * (measured on the Playwright WebKit build on Linux). A cut of an HTTP/2 stream reads as `net::ERR_ABORTED` in
 * Chromium, which this list accepts, and raises no failure at all in Firefox: this list cannot
 * observe an HTTP/2 cut. The server's own `warn` line (`connection cut: ...` or `response stream
 * cut: ...`) does, on both protocols and in every engine: the Playwright configuration pipes the
 * windowed servers' standard output (`WINDOWED_SERVER_STDOUT`), so the line appears in the test
 * output under the `[WebServer]` prefix. */
export const SELF_CANCELLED: readonly string[] = [
  "net::ERR_ABORTED",
  "NS_BINDING_ABORTED",
  "NS_ERROR_PARSED_DATA_CACHED",
  "Load request cancelled",
];

/** The Opus derivative a leg asserts the player requests. */
export type Variant = "opus" | "opus-webm";

/** Installs, before any page script runs, the test-side observers the legs read: every created
 * `<audio>` element registered with a count of its `ended` events, every `AudioContext`, and an
 * `AnalyserNode` fed by whatever connects to a context's destination (the page's mixed output).
 * When `hideOgg` is set, `canPlayType` answers `""` for Ogg types (the measured WebKit answer), so
 * the engine's own source selection falls to the WebM derivative.
 * @param context - The browser context whose pages are instrumented.
 * @param hideOgg - Whether `canPlayType` hides Ogg.
 * @example
 * ```
 * declare const context: import("@playwright/test").BrowserContext;
 * await installAudioProbes(context, false);
 * ```
 */
export async function installAudioProbes(context: BrowserContext, hideOgg: boolean): Promise<void> {
  await context.addInitScript(({ hide, logged }: ProbeArgs) => {
    const w = window as unknown as Record<string, unknown>;
    const elements: TrackedElement[] = [];
    const contexts: BaseAudioContext[] = [];
    const analysers = new Map<BaseAudioContext, AnalyserNode>();
    w.__scElements = elements;
    w.__scContexts = contexts;
    w.__scAnalysers = analysers;
    const create = document.createElement.bind(document);
    document.createElement = ((tag: string, options?: ElementCreationOptions) => {
      const el = create(tag, options);
      if (String(tag).toLowerCase() === "audio") {
        const audio = el as HTMLAudioElement;
        const entry: TrackedElement = { el: audio, ended: 0, errors: 0, events: [] };
        el.addEventListener("ended", () => {
          entry.ended += 1;
        });
        el.addEventListener("error", () => {
          entry.errors += 1;
        });
        for (const type of logged) {
          el.addEventListener(type, () => {
            entry.events.push({
              type,
              atEpochMs: Date.now(),
              currentTime: audio.currentTime,
              readyState: audio.readyState,
              networkState: audio.networkState,
              paused: audio.paused,
            });
          });
        }
        elements.push(entry);
      }
      return el;
    }) as typeof document.createElement;
    if (hide) {
      const original = HTMLMediaElement.prototype.canPlayType;
      HTMLMediaElement.prototype.canPlayType = function (type: string) {
        return /^audio\/ogg/i.test(type) ? "" : original.call(this, type);
      };
    }
    if (typeof AudioContext !== "undefined") {
      window.AudioContext = new Proxy(AudioContext, {
        construct(target, args, newTarget) {
          const created = Reflect.construct(target, args, newTarget) as AudioContext;
          contexts.push(created);
          return created;
        },
      });
    }
    const connect = AudioNode.prototype.connect as (this: AudioNode, ...args: unknown[]) => unknown;
    AudioNode.prototype.connect = function (this: AudioNode, ...args: unknown[]) {
      const target = args[0];
      if (typeof AudioDestinationNode !== "undefined" && target instanceof AudioDestinationNode) {
        let analyser = analysers.get(this.context);
        if (analyser === undefined) {
          analyser = this.context.createAnalyser();
          analyser.fftSize = 32768;
          analyser.smoothingTimeConstant = 0;
          analysers.set(this.context, analyser);
        }
        connect.call(this, analyser);
      }
      return connect.apply(this, args);
    } as typeof AudioNode.prototype.connect;
  }, { hide: hideOgg, logged: LOGGED_MEDIA_EVENTS });
}

/** What the page's analyser read at one instant. */
export interface MarkerReading {
  /** The strongest frequency in 100..215 Hz, in hertz, or `null` when no analyser exists yet. */
  hz: number | null;
  /** The magnitude of that peak in decibels. */
  db: number | null;
  /** The semitone code the peak maps to, relative to `MANIFEST.marker_base_hz`. */
  code: number | null;
}

/** Reads the position code from the page's mixed output: the strongest spectral peak in
 * 100..215 Hz mapped to the nearest semitone above the marker base frequency.
 * @param page - The page to read.
 * @returns The reading.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const reading = await readMarker(page);
 * ```
 */
export async function readMarker(page: Page): Promise<MarkerReading> {
  return page.evaluate((base: number) => {
    const analysers = (window as unknown as PageProbes).__scAnalysers;
    for (const analyser of analysers.values()) {
      const bins = new Float32Array(analyser.frequencyBinCount);
      analyser.getFloatFrequencyData(bins);
      const binHz = analyser.context.sampleRate / analyser.fftSize;
      let best = -1;
      for (let i = Math.ceil(100 / binHz); i <= Math.floor(215 / binHz); i++) {
        if (best < 0 || bins[i] > bins[best]) best = i;
      }
      if (best < 0) continue;
      const hz = best * binHz;
      return { hz, db: bins[best], code: Math.round(12 * Math.log2(hz / base)) + 0 };
    }
    return { hz: null, db: null, code: null };
  }, MANIFEST.marker_base_hz);
}

/** The state of the page's streaming element for an asset. */
export interface ElementState {
  /** `currentTime` in seconds. */
  currentTime: number;
  /** Whether the element is paused. */
  paused: boolean;
  /** `error` code, or `null`. */
  errorCode: number | null;
  /** Total `error` events seen on every element that streamed this asset. */
  errorEvents: number;
  /** Total `ended` events seen on every element that streamed this asset. */
  ended: number;
  /** The element's `src` path and query. */
  src: string;
  /** The element's `readyState`. */
  readyState: number;
  /** The element's `networkState`. */
  networkState: number;
}

/** The `<audio>` element playing `assetId`, as the page sees it: the one that is not paused, the
 * newest of those, else the one furthest along (the engine also holds an unplayed preload element
 * for the next track, which stays paused at 0).
 * @param page - The page to read.
 * @param assetId - The asset id.
 * @returns The state, or `null` when no element streams the asset.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const state = await elementState(page, "00000000-0000-0000-0000-000000000001");
 * ```
 */
export async function elementState(page: Page, assetId: string): Promise<ElementState | null> {
  return page.evaluate((id: string) => {
    const all = (window as unknown as PageProbes).__scElements;
    const mine = all.filter((e) => e.el.src.includes(`/api/assets/${id}`));
    if (mine.length === 0) return null;
    const live = mine.filter((e) => !e.el.paused && !e.el.ended);
    const chosen =
      live.length > 0
        ? live[live.length - 1]
        : mine.reduce((a, b) => (b.el.currentTime >= a.el.currentTime ? b : a));
    const url = new URL(chosen.el.src);
    return {
      currentTime: chosen.el.currentTime,
      paused: chosen.el.paused,
      errorCode: chosen.el.error === null ? null : chosen.el.error.code,
      errorEvents: mine.reduce((sum, e) => sum + e.errors, 0),
      ended: mine.reduce((sum, e) => sum + e.ended, 0),
      src: url.pathname + url.search,
      readyState: chosen.el.readyState,
      networkState: chosen.el.networkState,
    };
  }, assetId);
}

/** The media event log of every element that streamed `assetId`, one list per element in creation
 * order (the preload element and each playing element are separate lists).
 * @param page - The page to read.
 * @param assetId - The asset id.
 * @returns The event lists.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const logs = await elementEventLogs(page, "00000000-0000-0000-0000-000000000001");
 * ```
 */
export async function elementEventLogs(page: Page, assetId: string): Promise<ElementEvent[][]> {
  return page.evaluate((id: string) => {
    const all = (window as unknown as PageProbes).__scElements;
    return all.filter((e) => e.el.src.includes(`/api/assets/${id}`)).map((e) => e.events);
  }, assetId);
}

/** The state of every `AudioContext` the page created.
 * @param page - The page to read.
 * @returns The context states, in creation order.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const states = await contextStates(page);
 * ```
 */
export async function contextStates(page: Page): Promise<string[]> {
  return page.evaluate(() =>
    (window as unknown as PageProbes).__scContexts.map((c) => c.state),
  );
}

/** One live entry of the world's `audio-state` document. */
export interface PlayingEntry {
  /** The entry id (a fresh id on every advance). */
  id: string;
  /** The asset id. */
  asset: string;
  /** The index of the track in its playlist. */
  trackIndex: number;
  /** The server clock, in milliseconds, at which position 0 was (virtually) played. */
  startedAt: number;
  /** The server clock at which the entry paused, or `null`. */
  pausedAt: number | null;
}

/** The world snapshot, narrowed to its documents. */
interface WorldSnapshot {
  /** Every document the caller may read. */
  documents: SnapshotDocument[];
}

/** A document of the world snapshot, narrowed to what the audio state reader needs. */
interface SnapshotDocument {
  /** The document type. */
  doc_type: string;
  /** The typed engine body (the `audio-state` body carries the playing entries). */
  engine: AudioStateBody;
}

/** The `audio-state` engine body. */
interface AudioStateBody {
  /** The live entries. */
  playing: PlayingEntry[];
}

/** The live entries of the world's `audio-state` document, read through the snapshot route so the
 * answer is the server's, not the page's.
 * @param request - An authenticated request context.
 * @param worldId - The world id.
 * @returns The playing entries.
 * @example
 * ```
 * declare const request: import("@playwright/test").APIRequestContext;
 * const playing = await playingEntries(request, "w");
 * ```
 */
export async function playingEntries(request: APIRequestContext, worldId: string): Promise<PlayingEntry[]> {
  const response = await request.get(`/api/worlds/${worldId}/snapshot`);
  expect(response.status()).toBe(200);
  const snapshot = (await response.json()) as WorldSnapshot;
  return snapshot.documents.find((d) => d.doc_type === "audio-state")?.engine.playing ?? [];
}

/** The server position of an entry, in seconds, now: the host clock stands in for the server clock
 * (the server runs on the same machine).
 * @param entry - The entry.
 * @returns The position in seconds from the start of the track.
 * @example
 * ```
 * const secs = serverPositionSecs({ id: "e", asset: "a", trackIndex: 0, startedAt: Date.now() - 5000, pausedAt: null });
 * ```
 */
export function serverPositionSecs(entry: PlayingEntry): number {
  return ((entry.pausedAt ?? Date.now()) - entry.startedAt) / 1000;
}

/** One asset request, response or failure the page produced, with the time since recording began. */
export interface TrafficEvent {
  /** Milliseconds since `recordAssetTraffic` was called. */
  atMs: number;
  /** `request`, `response` or `failed`. */
  kind: "request" | "response" | "failed";
  /** The `variant` query parameter, or `null` for the canonical URL. */
  variant: string | null;
  /** The request's `Range` header, or `null`. */
  range: string | null;
  /** HTTP status (responses). */
  status: number | null;
  /** The `Content-Range` header (responses). */
  contentRange: string | null;
  /** The `Content-Encoding` header (responses). */
  contentEncoding: string | null;
  /** The `Content-Length` header (responses). */
  contentLength: string | null;
  /** The request's `If-Range` header, or `null`. */
  ifRange: string | null;
  /** The failure text (failed). */
  error: string | null;
}

/** A live recording of every request the page makes for one asset. */
export interface TrafficLog {
  /** The recorded events, in arrival order. */
  events: TrafficEvent[];
  /** Wall-clock time recording began, in epoch milliseconds: add `TrafficEvent.atMs` to place an
   * event on the same clock as `ElementEvent.atEpochMs`. */
  startedAtEpochMs: number;
}

/** Records every request, response and failure for `/api/assets/{assetId}` on `page`.
 * @param page - The page to observe.
 * @param assetId - The asset id.
 * @returns The live log.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const log = recordAssetTraffic(page, "00000000-0000-0000-0000-000000000001");
 * ```
 */
export function recordAssetTraffic(page: Page, assetId: string): TrafficLog {
  const started = Date.now();
  const log: TrafficLog = { events: [], startedAtEpochMs: started };
  const mine = (url: string): URL | null => {
    const parsed = new URL(url);
    return parsed.pathname === `/api/assets/${assetId}` ? parsed : null;
  };
  const blank = {
    status: null,
    contentRange: null,
    contentEncoding: null,
    contentLength: null,
    ifRange: null,
    error: null,
  };
  page.on("request", (r) => {
    const url = mine(r.url());
    if (url === null) return;
    log.events.push({
      ...blank,
      atMs: Date.now() - started,
      kind: "request",
      variant: url.searchParams.get("variant"),
      range: r.headers()["range"] ?? null,
      ifRange: r.headers()["if-range"] ?? null,
    });
  });
  page.on("response", (r) => {
    const url = mine(r.url());
    if (url === null) return;
    const headers = r.headers();
    log.events.push({
      atMs: Date.now() - started,
      kind: "response",
      variant: url.searchParams.get("variant"),
      range: r.request().headers()["range"] ?? null,
      status: r.status(),
      contentRange: headers["content-range"] ?? null,
      contentEncoding: headers["content-encoding"] ?? null,
      contentLength: headers["content-length"] ?? null,
      ifRange: r.request().headers()["if-range"] ?? null,
      error: null,
    });
  });
  page.on("requestfailed", (r) => {
    const url = mine(r.url());
    if (url === null) return;
    log.events.push({
      ...blank,
      atMs: Date.now() - started,
      kind: "failed",
      variant: url.searchParams.get("variant"),
      range: r.headers()["range"] ?? null,
      error: r.failure()?.errorText ?? "",
    });
  });
  return log;
}

/** One asset request as it crossed the network. */
export interface RawExchange {
  /** The request headers the stack sent. */
  request: Record<string, string>;
  /** The response headers the server sent. */
  response: Record<string, string>;
  /** The response status the server sent. */
  status: number;
}

/** The response half of one raw exchange. */
interface RawResponse {
  /** The response headers. */
  headers: Record<string, string>;
  /** The status code. */
  status: number;
}

/** What the browser's network layer reports about the responses for one asset. */
export interface NetworkFacts {
  /** The distinct protocol names the browser reported (`h2`, `http/1.1`), sorted.
   * @returns The protocol names. */
  protocols(): string[];
  /** The raw request and response of every asset request as the network stack sent and received
   * them: the page-level events also show responses the browser's cache assembled after a `304`,
   * which the server never sent.
   * @returns The exchanges, in request order. */
  rawExchanges(): RawExchange[];
}

/** Records, through the browser's own network instrumentation, the protocol of every response for
 * `/api/assets/{assetId}` and which of them came from the browser's HTTP cache. Only Chromium
 * exposes it.
 * @param page - The page to observe.
 * @param assetId - The asset id.
 * @returns The live facts, or `null` when the engine is not Chromium.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const facts = await recordNetworkFacts(page, "00000000-0000-0000-0000-000000000001");
 * ```
 */
export async function recordNetworkFacts(page: Page, assetId: string): Promise<NetworkFacts | null> {
  if (page.context().browser()?.browserType().name() !== "chromium") return null;
  const session = await page.context().newCDPSession(page);
  await session.send("Network.enable");
  const protocols = new Set<string>();
  const rawRequests = new Map<string, Record<string, string>>();
  const rawResponses = new Map<string, RawResponse>();
  const mine = new Set<string>();
  session.on("Network.requestWillBeSentExtraInfo", (event) => {
    rawRequests.set(event.requestId, event.headers as Record<string, string>);
  });
  session.on("Network.responseReceivedExtraInfo", (event) => {
    rawResponses.set(event.requestId, { headers: event.headers as Record<string, string>, status: event.statusCode });
  });
  session.on("Network.requestWillBeSent", (event) => {
    if (new URL(event.request.url).pathname === `/api/assets/${assetId}`) {
      mine.add(event.requestId);
    }
  });
  session.on("Network.responseReceived", (event) => {
    if (new URL(event.response.url).pathname !== `/api/assets/${assetId}`) return;
    protocols.add(event.response.protocol ?? "");
  });
  return {
    protocols: () => [...protocols].sort(),
    rawExchanges: () =>
      [...mine].flatMap((id): RawExchange[] => {
        const response = rawResponses.get(id);
        const request = rawRequests.get(id);
        return response === undefined || request === undefined
          ? []
          : [{ request, response: response.headers, status: response.status }];
      }),
  };
}

/** The parts of a `Content-Range: bytes a-b/total` value. */
export interface ContentRange {
  /** The first byte. */
  start: number;
  /** The last byte. */
  end: number;
  /** The total length of the representation. */
  total: number;
}

/** The parts of a `Content-Range: bytes a-b/total` value.
 * @param contentRange - The header value.
 * @returns The first byte, last byte and total length.
 * @example
 * ```
 * const { start, total } = parseContentRange("bytes 10-19/100");
 * ```
 */
export function parseContentRange(contentRange: string): ContentRange {
  const match = /^bytes (\d+)-(\d+)\/(\d+)$/.exec(contentRange);
  expect(match, `Content-Range ${contentRange}`).not.toBeNull();
  return { start: Number(match![1]), end: Number(match![2]), total: Number(match![3]) };
}

/** The value of header `name` in `headers`, matched without regard to case.
 * @param headers - The headers.
 * @param name - The header name.
 * @returns The value, or `undefined`.
 * @example
 * ```
 * const length = headerValue({ "Content-Length": "5" }, "content-length");
 * ```
 */
function headerValue(headers: Record<string, string>, name: string): string | undefined {
  const key = Object.keys(headers).find((k) => k.toLowerCase() === name);
  return key === undefined ? undefined : headers[key];
}

/** Counts the recorded responses by status, for a failure message.
 * @param log - The recorded traffic.
 * @returns The number of responses per status.
 * @example
 * ```
 * declare const log: TrafficLog;
 * const counts = statusCounts(log);
 * ```
 */
function statusCounts(log: TrafficLog): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const e of log.events.filter((x) => x.kind === "response")) {
    counts[String(e.status)] = (counts[String(e.status)] ?? 0) + 1;
  }
  return counts;
}

/** Asserts the traffic contract every long-audio leg shares: only `variant` is requested (never
 * the canonical URL, never the other derivative), every response the page saw is a `206`, the
 * initial `200` or a `304` revalidation, nothing is compressed, no request fails except
 * the element's own cancellation, and no response the SERVER sent carries more than
 * `MAX_RANGE_SLICE_BYTES`. The slice bound is read from the network stack's own record (`raw`)
 * where the engine exposes one, because a page level `206` can be assembled by the browser's cache
 * after a `304` and carry bytes the server never sent; otherwise from the page-level events. An
 * engine whose media loader reports some responses with no status and no headers (`opaqueResponses`)
 * has those responses skipped, and the rest must still be present and satisfy the contract.
 * @param log - The recorded traffic.
 * @param variant - The derivative the player is expected to request.
 * @param raw - The network stack's exchanges, or `null` when the engine exposes none.
 * @param opaqueResponses - Whether the engine reports media responses with status `0` and no headers.
 * @example
 * ```
 * declare const log: TrafficLog;
 * assertTrafficContract(log, "opus", null, false);
 * ```
 */
export function assertTrafficContract(
  log: TrafficLog,
  variant: Variant,
  raw: RawExchange[] | null,
  opaqueResponses = false,
): void {
  const requested = new Set(log.events.map((e) => e.variant));
  expect([...requested], "the player requests only the expected derivative").toEqual([variant]);
  const responses = log.events.filter((x) => x.kind === "response");
  const isOpaque = (e: TrafficEvent): boolean =>
    opaqueResponses && e.status === 0 && e.contentRange === null && e.contentLength === null && e.range === null;
  expect(
    responses.filter((e) => !isOpaque(e)).length,
    `responses with a status; statuses ${JSON.stringify(statusCounts(log))}`,
  ).toBeGreaterThan(0);
  for (const e of responses.filter((x) => !isOpaque(x))) {
    // A `304` is an engine revalidating a cached derivative before ranging into it. The engine
    // adds its conditional header below the page-level request record, so the request cannot be
    // checked here (the server's `304` rule is asserted by its own tests); the response must still
    // carry no range.
    if (e.status === 304) {
      expect(e.contentRange, `a 304 carries no Content-Range; event ${JSON.stringify(e)}`).toBeNull();
    } else {
      expect(
        [200, 206],
        `status ${e.status} for ${e.range}; event ${JSON.stringify(e)}; statuses ${JSON.stringify(statusCounts(log))}`,
      ).toContain(e.status);
    }
    if (raw === null && e.status === 206) {
      const { start, end } = parseContentRange(e.contentRange!);
      expect(end - start + 1, `slice ${e.contentRange}`).toBeLessThanOrEqual(MAX_RANGE_SLICE_BYTES);
    }
    expect(e.contentEncoding, "a ranged type is never compressed").toBeNull();
  }
  for (const exchange of raw ?? []) {
    expect([200, 206, 304], `server status for ${headerValue(exchange.request, "range")}`).toContain(exchange.status);
    if (exchange.status === 206) {
      const length = Number(headerValue(exchange.response, "content-length"));
      expect(length, `server slice for ${headerValue(exchange.request, "range")}`).toBeLessThanOrEqual(MAX_RANGE_SLICE_BYTES);
    }
    expect(headerValue(exchange.response, "content-encoding"), "a ranged type is never compressed").toBeUndefined();
  }
  const failures = log.events.filter((e) => e.kind === "failed" && !SELF_CANCELLED.includes(e.error ?? ""));
  expect(failures, "asset requests that failed other than by the element's own cancel").toEqual([]);
}

/** The world a leg runs in. */
export interface LegWorld {
  /** The world id. */
  id: string;
  /** The world name. */
  name: string;
}

/** The credentials of the worker's account. */
export interface AccountCredentials {
  /** The account's username. */
  username: string;
  /** The account's password. */
  password: string;
}

/** A world as the world listing returns it. */
interface WorldListing {
  /** The world id. */
  id: string;
  /** The world name. */
  name: string;
}

/** The asset an upload returns, narrowed to the fields the legs read. */
interface UploadResponse {
  /** The asset id. */
  id: string;
  /** The probed duration in milliseconds, or `null`. */
  duration_ms: number | null;
}

/** Logs in, creates a world and waits for its stage.
 * @param page - The page to drive.
 * @param account - The worker account.
 * @param label - Names the world.
 * @returns The world.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const world = await createWorld(page, { username: "u", password: "p" }, "Long Audio");
 * ```
 */
export async function createWorld(
  page: Page,
  account: AccountCredentials,
  label: string,
): Promise<LegWorld> {
  const name = `${label} ${Date.now().toString(36)}`;
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(name);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(page.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 60_000 });
  const worlds = (await (await page.request.get("/api/worlds")).json()) as WorldListing[];
  const world = worlds.find((w) => w.name === name);
  expect(world, "the world just created is listed").toBeDefined();
  return { id: world!.id, name };
}

/** An uploaded fixture track. */
export interface UploadedTrack {
  /** The asset id. */
  assetId: string;
  /** The uploaded file name (the tile's title in the asset picker). */
  fileName: string;
  /** The `duration_ms` the server stored. */
  durationMs: number;
}

/** Uploads a fixture track through the asset route (the import transcodes it before answering).
 * @param page - A page with an authenticated session.
 * @param world - The world.
 * @param track - The track name.
 * @returns The stored asset.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const uploaded = await uploadTrack(page, { id: "w", name: "n" }, "ambient-bed");
 * ```
 */
export async function uploadTrack(page: Page, world: LegWorld, track: TrackName): Promise<UploadedTrack> {
  const fileName = `${track}.ogg`;
  const uploaded = await page.request.post(`/api/worlds/${world.id}/assets`, {
    multipart: { file: { name: fileName, mimeType: "audio/ogg", buffer: trackBytes(track) } },
    timeout: 600_000,
  });
  expect(uploaded.status(), await uploaded.text()).toBe(200);
  const asset = (await uploaded.json()) as UploadResponse;
  expect(asset.duration_ms, "the pipeline probed a duration").not.toBeNull();
  return { assetId: asset.id, fileName, durationMs: Number(asset.duration_ms) };
}

/** Opens the audio panel, unlocks the device's audio from the real control, creates a one-track
 * playlist (non-looping entry) in `mode` and starts it from the panel, as the GM would.
 * @param page - The GM page.
 * @param track - The uploaded track.
 * @param mode - The playlist mode (`sequential` or `loop_all`).
 * @param beforePlay - Called after every setup request (the asset picker's thumbnails included)
 * and immediately before the play click, so a traffic recording started here sees playback only.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await startPlaylist(page, { assetId: "a", fileName: "x.ogg", durationMs: 1 }, "loop_all", () => {});
 * ```
 */
export async function startPlaylist(
  page: Page,
  track: UploadedTrack,
  mode: "sequential" | "loop_all",
  beforePlay: () => void,
): Promise<void> {
  await openPanel(page, "audio:panel");
  await page.getByTestId("audio-unlock").click();
  // The control swaps to the mute toggle once the engine's context is unlocked.
  await expect(page.getByTestId("audio-mute-toggle")).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("playlists-name").fill("Long Playlist");
  await page.getByTestId("playlists-create").click();
  const sheet = page.getByRole("dialog", { name: "Sheet" });
  await expect(sheet).toBeVisible({ timeout: 15_000 });
  await expect(sheet.getByTestId("playlist-name")).toHaveValue("Long Playlist");
  await sheet.getByTestId("playlist-mode").selectOption(mode);
  await sheet.getByTestId("playlist-add-track").click();
  const pick = page.getByTestId("asset-pick-dialog");
  await expect(pick).toBeVisible({ timeout: 15_000 });
  await pick.locator(`[data-testid="asset-tile"][title="${track.fileName}"]`).click();
  await page.getByTestId("pick-confirm").click();
  await expect(sheet.getByTestId("track-row")).toHaveCount(1);
  await expect(sheet.getByTestId("track-loop")).not.toBeChecked();
  await page.getByTestId("playlists-search").fill("Long Playlist");
  await expect(page.getByTestId("playlist-row")).toHaveCount(1);
  beforePlay();
  await page.getByTestId("playlist-play").click();
  await expect(page.getByTestId("audio-panel")).toHaveAttribute("data-audio-playing", "1", { timeout: 15_000 });
}

/** Sends the server `Seek` transport op through the audio panel's own seek control.
 * @param page - The GM page.
 * @param seconds - The target position in seconds.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await seekThroughPanel(page, 1185);
 * ```
 */
export async function seekThroughPanel(page: Page, seconds: number): Promise<void> {
  await page.getByTestId("playing-seek").fill(String(seconds));
}

/** The semitone code the manifest holds for the slot containing `positionSecs`.
 * @param track - The track name.
 * @param positionSecs - A position in seconds.
 * @returns The expected marker code.
 * @example
 * ```
 * const code = expectedCode("ambient-bed", 5);
 * ```
 */
export function expectedCode(track: TrackName, positionSecs: number): number {
  return MANIFEST.tracks[track].sequence[Math.floor(positionSecs / MANIFEST.slot_secs)];
}

/** The offset in seconds of `positionSecs` inside its slot.
 * @param positionSecs - A position in seconds.
 * @returns The phase in `[0, slot_secs)`.
 * @example
 * ```
 * const phase = slotPhase(15);
 * ```
 */
export function slotPhase(positionSecs: number): number {
  return positionSecs % MANIFEST.slot_secs;
}
