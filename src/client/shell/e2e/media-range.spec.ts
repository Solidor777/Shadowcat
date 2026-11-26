import { test, expect, login } from "./fixtures";
import { latchAttributeMax } from "./attribute-latch";
import { buildSilentWav, wavSecondsForBytes } from "../e2e-support/wav";

// A browser media element seeks and plays a long WAV through the server's byte-range responses.
// Runs only under `media-range-loopback`, whose server has a 3 second egress window: a response the
// element leaves idle between its read-ahead bursts for that long is really cut, so the run fails
// if the element's requests are not completed or recovered through `Range`.

/** Samples per second per channel of the fixture. */
const SAMPLE_RATE = 48_000;

/** Interleaved channels of the fixture. */
const CHANNELS = 2;

/** The fixture's size ceiling: far enough under the GM upload cap (2 x 25 MiB) to upload in one
 * request, and several times the 2 MiB read-ahead burst a media element asks for. */
const WAV_BYTES_CEILING = 24 * 1024 * 1024;

/** One media read-ahead slice, in bytes (the server's `MAX_RANGE_SLICE`). A seek past the buffer
 * must land at least this far into the file. */
const SLICE_BYTES = 2 * 1024 * 1024;

/** The failure text Chromium reports for a request it cancels itself (a seek replaces the request
 * in flight); every other failure text on an asset request fails the run. */
const SELF_CANCELLED = "net::ERR_ABORTED";

/** One response to a request for the asset, as the browser saw it. */
interface Seen {
  /** HTTP status. */
  status: number;
  /** The `Content-Range` header, if any. */
  contentRange: string | null;
  /** The `Content-Encoding` header, if any. */
  contentEncoding: string | null;
}

test("a long WAV seeks through Range and plays to the end with no failing asset request", async ({ page, account }) => {
  test.setTimeout(240_000);
  const worldName = `Media Range World ${Date.now().toString(36)}`;
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(worldName);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(page.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  // The upload runs the server's audio pipeline (transcode to the Opus derivatives) before it
  // answers, so its budget is the transcode's, not an HTTP round trip's.
  const worlds = (await (await page.request.get("/api/worlds")).json()) as { id: string; name: string }[];
  const world = worlds.find((w) => w.name === worldName);
  expect(world, "the world just created is listed").toBeDefined();
  const seconds = wavSecondsForBytes(WAV_BYTES_CEILING, SAMPLE_RATE, CHANNELS);
  const wav = buildSilentWav({ seconds, sampleRate: SAMPLE_RATE, channels: CHANNELS });
  const uploaded = await page.request.post(`/api/worlds/${world!.id}/assets`, {
    multipart: { file: { name: "long.wav", mimeType: "audio/wav", buffer: wav } },
    timeout: 180_000,
  });
  expect(uploaded.status()).toBe(200);
  const assetId = ((await uploaded.json()) as { id: string }).id;
  const assetPath = `/api/assets/${assetId}`;

  const responses: Seen[] = [];
  const failures: { url: string; error: string }[] = [];
  page.on("response", (r) => {
    if (!new URL(r.url()).pathname.startsWith(assetPath)) return;
    const headers = r.headers();
    responses.push({
      status: r.status(),
      contentRange: headers["content-range"] ?? null,
      contentEncoding: headers["content-encoding"] ?? null,
    });
  });
  page.on("requestfailed", (r) => {
    if (new URL(r.url()).pathname.startsWith(assetPath)) {
      failures.push({ url: r.url(), error: r.failure()?.errorText ?? "" });
    }
  });

  // A bare media element on the canonical URL; its events are mirrored into counters on its own
  // attributes, which only ever rise, so the latches below read what happened between polls.
  await page.evaluate((src) => {
    const audio = document.createElement("audio");
    audio.id = "media-probe";
    audio.preload = "auto";
    for (const name of ["loadedmetadata", "seeked", "ended", "error"]) {
      audio.setAttribute(`data-${name}`, "0");
      audio.addEventListener(name, () => {
        audio.setAttribute(`data-${name}`, String(Number(audio.getAttribute(`data-${name}`)) + 1));
      });
    }
    audio.src = src;
    document.body.appendChild(audio);
  }, assetPath);
  const probe = page.locator("#media-probe");
  await expect(probe).toHaveAttribute("data-loadedmetadata", /^[1-9]/, { timeout: 30_000 });
  const duration = await probe.evaluate((a: HTMLAudioElement) => a.duration);
  expect(duration).toBeGreaterThan(seconds - 1);
  expect(duration).toBeLessThan(seconds + 1);

  // (1) A seek to 80 percent of the track lands, with no media error, through a 206 that starts
  // beyond the first slice: the element could only reach that offset with a Range request.
  const seeked = await latchAttributeMax(probe, "data-seeked");
  await probe.evaluate((a: HTMLAudioElement, to) => {
    a.currentTime = to;
  }, duration * 0.8);
  await expect.poll(() => seeked.max(), { timeout: 30_000 }).toBeGreaterThanOrEqual(1);
  expect(await probe.evaluate((a: HTMLAudioElement) => a.error === null)).toBe(true);
  const starts = responses
    .filter((r) => r.status === 206 && r.contentRange !== null)
    .map((r) => Number(/^bytes (\d+)-/.exec(r.contentRange!)?.[1]));
  expect(
    starts.some((s) => s >= SLICE_BYTES),
    `a 206 starting at or beyond ${SLICE_BYTES}; saw ${JSON.stringify(responses)}`,
  ).toBe(true);

  // (2) From the start at 16 times speed the track plays to its end: the element reads across the
  // whole file in bursts with idle gaps, none of which may end in a failed request.
  const ended = await latchAttributeMax(probe, "data-ended");
  await probe.evaluate(async (a: HTMLAudioElement) => {
    a.currentTime = 0;
    a.playbackRate = 16;
    await a.play();
  });
  await expect.poll(() => ended.max(), { timeout: 90_000 }).toBeGreaterThanOrEqual(1);
  expect(await probe.evaluate((a: HTMLAudioElement) => a.error === null)).toBe(true);

  const unexpected = failures.filter((f) => f.error !== SELF_CANCELLED);
  expect(unexpected, "asset requests that failed other than by the element's own cancel").toEqual([]);
  for (const r of responses) {
    expect([200, 206], `status ${r.status} ${r.contentRange ?? ""}`).toContain(r.status);
    expect(r.contentEncoding, "a ranged type is never compressed").toBeNull();
  }
});
