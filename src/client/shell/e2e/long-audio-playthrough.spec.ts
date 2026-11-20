import { test, expect } from "./fixtures";
import { MEDIA_RANGE_EGRESS_WINDOW_SECS } from "../e2e-support/egress-window";
import { sampleSlots } from "../e2e-support/slot-sampling";
import {
  MANIFEST,
  assertTrafficContract,
  contextStates,
  createWorld,
  elementState,
  expectedCode,
  installAudioProbes,
  playingEntries,
  readMarker,
  recordAssetTraffic,
  recordNetworkFacts,
  serverPositionSecs,
  startPlaylist,
  uploadTrack,
  type TrafficEvent,
} from "./long-audio-support";

// One 20-minute track plays from its start to the element's natural end in real time over the
// project's protocol (plain HTTP/1.1 or HTTPS/h2), with no seek, no rate change and no manual
// `currentTime`. The server runs a 3 second egress window, shorter than the idle gap the media
// element leaves between its read-ahead bursts, so a response left open across a real gap is cut
// and the run fails if the element's requests are not completed or recovered through `Range`.
// The position code embedded in the track (one semitone per 10-second slot) is read from the
// page's mixed output once per slot for the whole run, so every gap between asset requests that
// the egress window could cut holds at least one reading. Runs under the
// `long-audio-playthrough-*` projects, one per protocol and each its own CI job.
//
// What the traffic whitelist (`SELF_CANCELLED` in `long-audio-support`) can see: on HTTP/1.1 a
// server-side cut surfaces as a failure text the whitelist does not list, so it fails the run. On
// h2 it cannot see a cut. Chromium reports a cut stream as `net::ERR_ABORTED`, which the whitelist
// accepts as the element's own cancel, and Firefox raises no failure at all. An h2 cut is caught
// only through the other checks: the element's `errorCode` and `errorEvents`, the position-code
// readings, the natural end and the server's stopped entry. The slice bound and the failed-request
// check run after every slot, not only at the end.

/** The fixture track played in full: the one whose served sizes were measured. */
const TRACK = "ambient-bed" as const;

/** Phase, in seconds into a slot, at which the position code is read: the middle of the slot. */
const READ_PHASE_SECS = 5;

/** Tolerance, in milliseconds, between the stored `duration_ms` and the manifest's track length. */
const STORED_DURATION_TOLERANCE_MS = 50;

/** The protocol the browser must report for the project: plain HTTP is HTTP/1.1, HTTPS is h2. */
function expectedProtocol(projectName: string): string {
  return projectName.includes("-https-") ? "h2" : "http/1.1";
}

/** The egress window, in milliseconds, of the server the play-through projects run against:
 * `MEDIA_RANGE_EGRESS_WINDOW_SECS`, the value the Playwright configuration sets on that server.
 * A response left open across a gap at least this long is cut, so only such a gap can expose a cut
 * the readings must cover. The first reading exists no earlier than the middle of slot 0
 * (`READ_PHASE_SECS`), so the start-up requests, which are all closer together than the window,
 * cannot hold one. */
const EGRESS_WINDOW_MS = MEDIA_RANGE_EGRESS_WINDOW_SECS * 1000;

/** The gaps, in milliseconds, between two consecutive asset requests that last at least
 * `EGRESS_WINDOW_MS` and hold no reading, over the request log and the reading times.
 * @param requests - The request events, in arrival order.
 * @param readings - The times of the readings, in milliseconds since recording began.
 * @returns The gaps with no reading inside, as `[from, to]` pairs.
 * @example
 * ```
 * const empty = gapsWithoutReading([], []);
 * ```
 */
function gapsWithoutReading(requests: TrafficEvent[], readings: number[]): [number, number][] {
  const times = requests.filter((e) => e.kind === "request").map((e) => e.atMs);
  const empty: [number, number][] = [];
  for (let i = 1; i < times.length; i++) {
    const [from, to] = [times[i - 1], times[i]];
    if (to - from >= EGRESS_WINDOW_MS && !readings.some((t) => t > from && t < to)) empty.push([from, to]);
  }
  return empty;
}

test("a 20-minute track plays from its start to its natural end in real time with no failing request", async ({
  page,
  account,
}, testInfo) => {
  test.setTimeout(2_400_000);
  await installAudioProbes(page.context(), false);
  const world = await createWorld(page, account, "Playthrough World");
  const track = await uploadTrack(page, world, TRACK);
  const lengthSecs = MANIFEST.tracks[TRACK].duration_secs;
  expect(Math.abs(track.durationMs - lengthSecs * 1000), `stored duration_ms ${track.durationMs}`).toBeLessThanOrEqual(
    STORED_DURATION_TOLERANCE_MS,
  );

  let log = recordAssetTraffic(page, track.assetId);
  const network = await recordNetworkFacts(page, track.assetId);
  const started = { at: 0 };
  await startPlaylist(page, track, "sequential", () => {
    log = recordAssetTraffic(page, track.assetId);
    started.at = Date.now();
  });
  expect(await contextStates(page), "the unlock click leaves every context running").not.toContain("suspended");
  const entries = await playingEntries(page.request, world.id);
  expect(entries, "exactly one entry plays").toHaveLength(1);
  const entry = entries[0];
  const hasWebAudio = (await contextStates(page)).length > 0;

  const readings: { slot: number; at: number; position: number; drift: number | null; code: number | null; db: number | null }[] = [];
  try {
    const slots = lengthSecs / MANIFEST.slot_secs;
    /** Waits for the middle of `slot`, asserts the element and the position code, and records the reading.
     * @param slot - The slot index.
     * @returns Resolves once the slot is recorded.
     */
    const readSlot = async (slot: number): Promise<void> => {
      const target = slot * MANIFEST.slot_secs + READ_PHASE_SECS;
      await expect.poll(() => serverPositionSecs(entry), { intervals: [500], timeout: 120_000 }).toBeGreaterThanOrEqual(target);
      const position = serverPositionSecs(entry);
      const reading = hasWebAudio ? await readMarker(page) : { hz: null, db: null, code: null };
      const element = await elementState(page, track.assetId);
      expect(element, "an element streams the asset").not.toBeNull();
      expect(element!.paused, `the element plays at server position ${position}`).toBe(false);
      expect(element!.errorCode, "no media error").toBeNull();
      readings.push({
        slot,
        at: Date.now() - started.at,
        position,
        drift: element!.currentTime - position,
        code: reading.code,
        db: reading.db,
      });
      if (hasWebAudio) {
        expect(reading.code, `position code at ${position.toFixed(1)}s: ${JSON.stringify(readings.at(-1))}`).toBe(
          expectedCode(TRACK, position),
        );
      }
    };
    await sampleSlots(slots, readSlot, () => assertTrafficContract(log, "opus", network?.rawExchanges() ?? null));

    // The natural end: the element ends with no error and the server stops the sequential entry.
    await expect
      .poll(async () => (await elementState(page, track.assetId))?.ended ?? 0, { intervals: [500], timeout: 60_000 })
      .toBeGreaterThanOrEqual(1);
    expect((await elementState(page, track.assetId))!.errorEvents, "no media error up to the end").toBe(0);
    await expect(page.getByTestId("audio-panel")).toHaveAttribute("data-audio-playing", "0", { timeout: 30_000 });
    expect(await playingEntries(page.request, world.id), "the server stopped the entry at the end").toHaveLength(0);

    const gaps = gapsWithoutReading(log.events, readings.map((r) => r.at));
    expect(gaps, "every gap of at least the egress window between asset requests holds a reading").toEqual([]);
    if (network !== null) expect(network.protocols()).toEqual([expectedProtocol(testInfo.project.name)]);
  } finally {
    testInfo.attach("readings", { body: JSON.stringify(readings), contentType: "application/json" });
    testInfo.attach("traffic", { body: JSON.stringify({ events: log.events, raw: network?.rawExchanges() ?? [] }), contentType: "application/json" });
  }
});
