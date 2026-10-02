import { test, expect, login } from "./fixtures";
import { buildSilentWav } from "../e2e-support/wav";
import { firefoxWebmDecodeDelta } from "../e2e-support/firefox-webm-trim";

// `decodeAudioData` of the server's Opus derivatives returns exactly the uploaded sample count in
// each engine. A decoder applies the container's trim itself (the WebM `CodecDelay` at the front
// and `DiscardPadding` at the end, the Ogg pre-skip and final granule position), so a derivative
// whose declared trim disagrees with its packets decodes to a different length. Runs under the
// `webm-decode-chromium` and `webm-decode-firefox` projects. Chromium (both containers) and
// Firefox's Ogg decode the exact count; Firefox's WebM decode is asserted against
// `firefoxWebmDecodeDelta`, which models the one-sample loss of its microsecond-quantised end trim.

/** Samples per second per channel of every fixture; the decode context runs at the same rate, so
 * the decoded length is not resampled. */
const SAMPLE_RATE = 48_000;

/** Opus frame length in samples at 48 kHz (20 ms). */
const FRAME_SAMPLES = 960;

/** Whole frames in every fixture's valid length, so the residue below is the only partial frame. */
const WHOLE_FRAMES = 50;

/** Residues of the valid sample count modulo one frame. 0, 649 and 959 lose their tail samples
 * without the encoder's lookahead flush and 648 is the largest residue that does not. For
 * Firefox's WebM trim, 0, 648 and 649 decode exactly, 4 and 959 lose one sample (the last
 * packet's audible count modulo 6 is 4 and 5), and 3 and 33 have an audible count of 315 and 345
 * (modulo 6 is 3), where the rounding of the sum decides: 3 decodes exactly and 33 loses one. */
const RESIDUES = [0, 3, 4, 33, 648, 649, 959];

/** The derivative variants the server serves and the decode of each. */
const VARIANTS = ["opus-webm", "opus"] as const;

test("each Opus derivative decodes to the uploaded sample count", async ({ page, account, browserName }) => {
  test.setTimeout(120_000);
  const worldName = `Webm Decode World ${Date.now().toString(36)}`;
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(worldName);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(page.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });
  const worlds = (await (await page.request.get("/api/worlds")).json()) as { id: string; name: string }[];
  const world = worlds.find((w) => w.name === worldName);
  expect(world, "the world just created is listed").toBeDefined();

  const observed: Record<string, number> = {};
  for (const residue of RESIDUES) {
    const valid = WHOLE_FRAMES * FRAME_SAMPLES + residue;
    const wav = buildSilentWav({ seconds: valid / SAMPLE_RATE, sampleRate: SAMPLE_RATE, channels: 1 });
    const uploaded = await page.request.post(`/api/worlds/${world!.id}/assets`, {
      multipart: { file: { name: `residue-${residue}.wav`, mimeType: "audio/wav", buffer: wav } },
      timeout: 60_000,
    });
    expect(uploaded.status()).toBe(200);
    const assetId = ((await uploaded.json()) as { id: string }).id;
    for (const variant of VARIANTS) {
      const length = await page.evaluate(
        async ({ url, sampleRate }) => {
          const bytes = await (await fetch(url)).arrayBuffer();
          const context = new OfflineAudioContext(1, 1, sampleRate);
          const decoded = await context.decodeAudioData(bytes);
          return decoded.length;
        },
        { url: `/api/assets/${assetId}?variant=${variant}`, sampleRate: SAMPLE_RATE },
      );
      observed[`${variant} residue ${residue}`] = length - valid;
    }
  }
  // Reported as the difference from the valid length, so a failure names the engine's observed
  // extra or missing samples per container and residue.
  const expected: Record<string, number> = {};
  for (const residue of RESIDUES) {
    const valid = WHOLE_FRAMES * FRAME_SAMPLES + residue;
    for (const variant of VARIANTS) {
      const firefoxWebm = browserName === "firefox" && variant === "opus-webm";
      expected[`${variant} residue ${residue}`] = firefoxWebm ? firefoxWebmDecodeDelta(valid) : 0;
    }
  }
  expect(observed).toEqual(expected);
});
