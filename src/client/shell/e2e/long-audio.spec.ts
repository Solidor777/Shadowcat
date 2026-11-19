import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  MANIFEST,
  MAX_RANGE_SLICE_BYTES,
  TRACK_NAMES,
  assertTrafficContract,
  contextStates,
  createWorld,
  elementState,
  expectedCode,
  installAudioProbes,
  parseContentRange,
  playingEntries,
  readMarker,
  recordAssetTraffic,
  recordNetworkFacts,
  seekThroughPanel,
  serverPositionSecs,
  slotPhase,
  startPlaylist,
  uploadTrack,
  type PlayingEntry,
  type TrackName,
  type Variant,
} from "./long-audio-support";

// The audio engine streams a 20-minute NON-looping playlist entry through an `<audio>` element over
// the server's byte-range responses. Each leg uploads one fixture track, plays it from the start
// for about a minute, sends the server `Seek` transport op to 15 seconds before the end, plays to
// the element's natural end, and watches the playlist wrap (a one-entry `loop_all` playlist
// advances to its own first entry). The position code embedded in the fixture (one semitone per
// 10-second slot) is read from the page's mixed output, so the audible position is checked rather
// than the element's own claim. Runs under the `long-audio-*` projects, whose server uses a 3
// second egress window: a response left idle across a read-ahead gap is cut, so the run fails if
// the element's requests are not completed or recovered through `Range`.

/** One leg: a fixture track played over one derivative container. */
interface Leg {
  /** The fixture track. */
  track: TrackName;
  /** The derivative the player is expected to request. */
  variant: Variant;
}

/** The three Ogg legs and one WebM leg a Chromium project runs. */
const CHROMIUM_LEGS: Leg[] = [
  ...TRACK_NAMES.map((track): Leg => ({ track, variant: "opus" })),
  { track: "ambient-bed", variant: "opus-webm" },
];

/** The legs each project runs: Chromium over h1 and h2 run every leg, the other engines one track
 * over the container they select. */
const LEGS_BY_PROJECT: Record<string, Leg[]> = {
  "long-audio-https-chromium": CHROMIUM_LEGS,
  "long-audio-http-chromium": CHROMIUM_LEGS,
  "long-audio-https-chromium-noflag": [{ track: "ambient-bed", variant: "opus" }],
  "long-audio-https-firefox": [
    { track: "ambient-bed", variant: "opus" },
    { track: "ambient-bed", variant: "opus-webm" },
  ],
  "long-audio-https-webkit": [{ track: "ambient-bed", variant: "opus-webm" }],
};

/** Seconds before the end of the track the server `Seek` lands. */
const SEEK_BEFORE_END_SECS = 15;

/** Seconds of play from the start of the track the first window covers. */
const START_WINDOW_SECS = 60;

/** Phase, in seconds into a slot, at which the position code is read: the middle of the slot. */
const READ_PHASE_SECS = 5;

/** Tolerance, in milliseconds, between the `duration_ms` the server stores and the manifest's track
 * length. */
const STORED_DURATION_TOLERANCE_MS = 50;

/** Tolerance, in milliseconds, between a derivative element's `duration` and `duration_ms`. */
const ELEMENT_DURATION_TOLERANCE_MS = 100;

/** Longest wait, in milliseconds, for the audible output to carry the new position after a seek. */
const SEEK_SETTLE_MS = 10_000;

/** One position-code reading and the drift measured with it. */
interface Sample {
  /** Which part of the leg. */
  phase: string;
  /** The slot read. */
  slot: number;
  /** The server position, in seconds, at the read. */
  serverPositionSecs: number;
  /** The element's `currentTime` minus the server position, in seconds, or `null`. */
  elementDriftSecs: number | null;
  /** The decoded code. */
  code: number | null;
  /** The peak magnitude, in decibels. */
  db: number | null;
  /** How long the read waited for the code of the new position after a seek or wrap, or `null`. */
  settledAfterMs: number | null;
}

/** The `duration` a metadata-only `<audio>` element reports for `url`, or `null` when the engine
 * cannot play the type.
 * @param page - The page to evaluate in.
 * @param url - The derivative URL.
 * @returns The duration in seconds.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const secs = await probeDuration(page, "/api/assets/x?variant=opus");
 * ```
 */
async function probeDuration(page: Page, url: string): Promise<number | null> {
  return page.evaluate(
    (src) =>
      new Promise<number | null>((resolve) => {
        const audio = new Audio();
        audio.preload = "metadata";
        audio.onloadedmetadata = () => resolve(audio.duration);
        audio.onerror = () => resolve(null);
        audio.src = src;
      }),
    url,
  );
}

for (const leg of [...CHROMIUM_LEGS]) {
  test(`${leg.track} streams over ${leg.variant}: start window, server Seek near the end, natural end, wrap`, async ({
    page,
    account,
    browserName,
  }, testInfo) => {
    const legs = LEGS_BY_PROJECT[testInfo.project.name] ?? [];
    test.skip(!legs.some((l) => l.track === leg.track && l.variant === leg.variant), "not a leg of this project");
    // Playwright's Windows WebKit build reports `canPlayType("audio/webm; codecs=opus")` as
    // "probably" yet fails every media load with MEDIA_ERR_SRC_NOT_SUPPORTED, also over plain HTTP,
    // and no shipping Safari exists on Windows; the WebKit leg runs on macOS and Linux only.
    test.skip(
      browserName === "webkit" && process.platform === "win32",
      "Playwright's Windows WebKit cannot load Opus media (MEDIA_ERR_SRC_NOT_SUPPORTED); the leg runs on macOS and Linux",
    );
    test.setTimeout(900_000);
    await installAudioProbes(page.context(), leg.variant === "opus-webm");
    const world = await createWorld(page, account, "Long Audio World");
    const track = await uploadTrack(page, world, leg.track);
    const lengthSecs = MANIFEST.tracks[leg.track].duration_secs;

    // Precondition of the end-of-track advance: the stored duration is the track's real length in
    // every container the engine may play.
    expect(Math.abs(track.durationMs - lengthSecs * 1000), `stored duration_ms ${track.durationMs}`).toBeLessThanOrEqual(
      STORED_DURATION_TOLERANCE_MS,
    );
    const durations: Record<string, number | null> = {};
    for (const variant of ["opus", "opus-webm"] as const) {
      const secs = await probeDuration(page, `/api/assets/${track.assetId}?variant=${variant}`);
      durations[variant] = secs;
      if (secs !== null) {
        expect(
          Math.abs(secs * 1000 - track.durationMs),
          `${variant} element duration ${secs}s against duration_ms ${track.durationMs}`,
        ).toBeLessThanOrEqual(ELEMENT_DURATION_TOLERANCE_MS);
      }
    }
    const playable = await page.evaluate(() => {
      const probe = new Audio();
      return { ogg: probe.canPlayType("audio/ogg; codecs=opus"), webm: probe.canPlayType("audio/webm; codecs=opus") };
    });
    testInfo.attach("durations", {
      body: JSON.stringify({ stored: track.durationMs, ...durations, canPlayType: playable }),
      contentType: "application/json",
    });
    expect(durations[leg.variant], `the engine can play the derivative under test; canPlayType ${JSON.stringify(playable)}`).not.toBeNull();

    const samples: Sample[] = [];
    let log = recordAssetTraffic(page, track.assetId);
    const network = await recordNetworkFacts(page, track.assetId);
    try {
      await startPlaylist(page, track, "loop_all", () => {
        log = recordAssetTraffic(page, track.assetId);
      });
      expect(await contextStates(page), "the unlock click leaves every context running").not.toContain("suspended");

      const firstEntry = await currentEntry(page, world.id);
      const hasWebAudio = (await contextStates(page)).length > 0;

      /** Reads the position code once the server position reaches the middle of `slot`. After a seek
       * the output carries the old position until the element's new data arrives, so such a read
       * waits up to `SEEK_SETTLE_MS` for the code of the new position and records how long it took. */
      const readSlot = async (entry: PlayingEntry, phase: string, slot: number, settle = false): Promise<void> => {
        const target = slot * MANIFEST.slot_secs + READ_PHASE_SECS;
        await expect
          .poll(() => serverPositionSecs(entry), { intervals: [200], timeout: 120_000 })
          .toBeGreaterThanOrEqual(target);
        const begun = Date.now();
        let position = serverPositionSecs(entry);
        let reading = hasWebAudio ? await readMarker(page) : { hz: null, db: null, code: null };
        while (hasWebAudio && settle && reading.code !== expectedCode(leg.track, position) && Date.now() - begun < SEEK_SETTLE_MS) {
          await page.waitForTimeout(100);
          position = serverPositionSecs(entry);
          reading = await readMarker(page);
        }
        const element = await elementState(page, track.assetId);
        expect(element, "an element streams the asset").not.toBeNull();
        expect(element!.paused, `the element plays at server position ${position}`).toBe(false);
        expect(element!.errorCode, "no media error").toBeNull();
        samples.push({
          phase,
          slot,
          serverPositionSecs: position,
          elementDriftSecs: element!.currentTime - position,
          code: reading.code,
          db: reading.db,
          settledAfterMs: settle ? Date.now() - begun : null,
        });
        if (hasWebAudio) {
          expect(reading.code, `position code at ${position.toFixed(1)}s (${phase}); sample ${JSON.stringify(samples.at(-1))}`).toBe(
            expectedCode(leg.track, position),
          );
        }
      };

      // (1) The start window: one reading in the middle of each slot for about a minute.
      for (let slot = 0; slot < START_WINDOW_SECS / MANIFEST.slot_secs; slot++) {
        await readSlot(firstEntry, "start", slot);
      }

      // (2) The server `Seek` to 15 seconds before the end, sent through the panel's own control.
      const seekTarget = lengthSecs - SEEK_BEFORE_END_SECS;
      const beforeSeek = log.events.length;
      await seekThroughPanel(page, seekTarget);
      let seeked: PlayingEntry | undefined;
      await expect
        .poll(
          async () => {
            seeked = (await playingEntries(page.request, world.id)).find((e) => e.id === firstEntry.id);
            return seeked === undefined ? 0 : serverPositionSecs(seeked);
          },
          { intervals: [250], timeout: 30_000 },
        )
        .toBeGreaterThanOrEqual(seekTarget - 1);
      await expect
        .poll(async () => (await elementState(page, track.assetId))?.currentTime ?? 0, { intervals: [250], timeout: 30_000 })
        .toBeGreaterThanOrEqual(seekTarget - 2);
      await expect
        .poll(() => log.events.slice(beforeSeek).filter((e) => e.kind === "response" && e.status === 206).length, {
          intervals: [100],
          timeout: 15_000,
          message: "the seek past the buffer is served by a ranged response",
        })
        .toBeGreaterThan(0);
      const afterSeek = log.events.slice(beforeSeek).filter((e) => e.kind === "response" && e.status === 206);
      const landed = afterSeek.map((e) => parseContentRange(e.contentRange!));
      expect(
        landed.some((r) => r.start >= r.total - MAX_RANGE_SLICE_BYTES),
        `a response after the seek starts in the last slice region; saw ${JSON.stringify(landed)}`,
      ).toBe(true);
      const lastSlot = Math.floor(lengthSecs / MANIFEST.slot_secs) - 1;
      if (slotPhase(serverPositionSecs(seeked!)) < READ_PHASE_SECS + 1) await readSlot(seeked!, "after-seek", lastSlot - 1, true);
      await readSlot(seeked!, "after-seek", lastSlot, true);

      // (3) The natural end: the element ends with no error and the server advances to the playlist's
      // own first entry, a new playing entry for the same track that starts again at 0.
      await expect
        .poll(async () => (await elementState(page, track.assetId))?.ended ?? 0, { intervals: [250], timeout: 60_000 })
        .toBeGreaterThanOrEqual(1);
      const ended = await elementState(page, track.assetId);
      expect(ended!.errorEvents, "no media error up to the end").toBe(0);
      let wrapped: PlayingEntry | undefined;
      await expect
        .poll(
          async () => {
            wrapped = (await playingEntries(page.request, world.id)).find((e) => e.id !== firstEntry.id);
            return wrapped?.trackIndex ?? -1;
          },
          { intervals: [250], timeout: 30_000, message: "the server advances after the natural end" },
        )
        .toBe(0);
      expect(wrapped!.asset).toBe(track.assetId);
      expect(serverPositionSecs(wrapped!), "the wrapped entry restarts from the start").toBeLessThan(30);
      await readSlot(wrapped!, "wrap", 0);

      await page.getByTestId("stop-all").click();
      await expect(page.getByTestId("audio-panel")).toHaveAttribute("data-audio-playing", "0", { timeout: 15_000 });

      assertTrafficContract(log, leg.variant, network?.rawExchanges() ?? null);
      if (network !== null) expect(network.protocols()).toEqual([testInfo.project.name.includes("-https-") ? "h2" : "http/1.1"]);
    } finally {
      testInfo.attach("samples", { body: JSON.stringify(samples, null, 1), contentType: "application/json" });
      testInfo.attach("traffic", { body: JSON.stringify({ events: log.events, raw: network?.rawExchanges() ?? [] }), contentType: "application/json" });
    }
  });
}

/** The single live entry of the world's audio state.
 * @param page - The GM page.
 * @param worldId - The world id.
 * @returns The entry.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const entry = await currentEntry(page, "w");
 * ```
 */
async function currentEntry(page: Page, worldId: string): Promise<PlayingEntry> {
  const entries = await playingEntries(page.request, worldId);
  expect(entries, "exactly one entry plays").toHaveLength(1);
  return entries[0];
}
