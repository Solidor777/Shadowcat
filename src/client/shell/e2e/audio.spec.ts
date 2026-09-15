import { test, expect, login, createAccount, openPanel, DUAL_SESSION_TIMEOUT_MS } from "./fixtures";

/** A minimal valid WAV (PCM16, mono, 8kHz, ~0.2s of silence) — small enough to keep the upload
 * fast, long enough for `symphonia`'s WAV decoder to report a non-zero `duration_ms` and for the
 * transport handler's elapsed-duration gate (`audio::transport::handle_transport`) to have
 * something to compare against. Mirrors the synthetic-WAV builder `process::audio`'s own
 * Rust-side tests use (`sample_rate`, `data`/`fmt ` chunk layout), reimplemented in JS since this
 * runs in the Playwright/Node process, not the server.
 * @param seconds Duration of silence to encode.
 * @returns The WAV file bytes.
 */
function buildSilentWav(seconds: number): Buffer {
  const sampleRate = 8000;
  const numSamples = Math.round(sampleRate * seconds);
  const dataSize = numSamples * 2; // 16-bit mono
  const buf = Buffer.alloc(44 + dataSize);
  buf.write("RIFF", 0, "ascii");
  buf.writeUInt32LE(36 + dataSize, 4);
  buf.write("WAVE", 8, "ascii");
  buf.write("fmt ", 12, "ascii");
  buf.writeUInt32LE(16, 16); // fmt chunk size
  buf.writeUInt16LE(1, 20); // PCM
  buf.writeUInt16LE(1, 22); // mono
  buf.writeUInt32LE(sampleRate, 24);
  buf.writeUInt32LE(sampleRate * 2, 28); // byte rate
  buf.writeUInt16LE(2, 32); // block align
  buf.writeUInt16LE(16, 34); // bits per sample
  buf.write("data", 36, "ascii");
  buf.writeUInt32LE(dataSize, 40);
  // Remaining bytes are already zero (`Buffer.alloc` zero-fills) — silence.
  return buf;
}

// Dual-session (GM + player), same rationale `combat-tracker.spec.ts`/`tables.spec.ts` state for
// their own dual-session assertions: the server is the sole authority for what each recipient's
// client ever renders, so a single-session spec cannot prove the player-side gate.
test("audio: GM uploads a track, creates a playlist, plays it, and a player sees no transport controls", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Audio World ${Date.now().toString(36)}`;

  const gm = page;
  await login(gm, account.username, account.password);
  await gm.getByLabel("New world name").fill(worldName);
  await gm.getByRole("button", { name: "Create world" }).click();
  await expect(gm.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await openPanel(gm, "settings:panel");
  await createAccount(gm, playerName, playerPassword);
  await gm.getByLabel("World role").selectOption("player");
  await gm.getByRole("button", { name: "Create invite" }).click();
  const code = await gm.getByLabel("Invite code").inputValue();
  expect(code.length).toBeGreaterThan(0);

  const playerCtx = await browser.newContext({ baseURL: test.info().project.use.baseURL });
  const player = await playerCtx.newPage();

  try {
    await login(player, playerName, playerPassword);
    await player.getByLabel("Invite code").fill(code);
    await player.getByRole("button", { name: "Join with an invite code" }).click();
    await expect(player.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(gm.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // Upload the test track via the asset browser.
    await openPanel(gm, "asset-browser:panel");
    await gm
      .getByTestId("asset-upload-input")
      .setInputFiles({ name: "tone.wav", mimeType: "audio/wav", buffer: buildSilentWav(0.2) });
    const tile = gm.getByTestId("asset-tile");
    await expect(tile).toHaveCount(1);

    // Both sides open the Audio panel.
    await openPanel(gm, "audio:panel");
    await openPanel(player, "audio:panel");

    // GM unlocks its own device audio (per-tab gesture the shell requires before the first
    // `AudioContext.resume()` — see `StatusBar`'s `audio-unlock` control).
    await gm.getByTestId("audio-unlock").click();

    // Create a playlist and add the uploaded track. `PlaylistSheet`'s add-track opens the
    // asset-pick overlay scoped to `kind: "audio"` FIRST — a track row always carries a
    // non-empty asset id (`PlaylistEngine::validate` rejects an unassigned track), so picking
    // precedes the row — and the just-uploaded tile is the world's only audio asset, hence the
    // sole pick candidate.
    await gm.getByTestId("playlists-name").fill("Tavern Loop");
    await gm.getByTestId("playlists-create").click();
    // The just-created playlist's sheet floats to the front (its chrome's accessible name is
    // `Sheet — floating window. …`, so a substring name match — exact "Sheet" matches nothing).
    const sheet = gm.getByRole("dialog", { name: "Sheet" });
    await expect(sheet).toBeVisible({ timeout: 15_000 });
    await expect(sheet.getByTestId("playlist-name")).toHaveValue("Tavern Loop");
    await sheet.getByTestId("playlist-add-track").click();
    const pickDialog = gm.getByTestId("asset-pick-dialog");
    await expect(pickDialog).toBeVisible({ timeout: 15_000 });
    await pickDialog.getByTestId("asset-tile").click();
    await gm.getByTestId("pick-confirm").click();
    await expect(sheet.getByTestId("track-row")).toHaveCount(1);

    // Play it from the panel's playlists list — the list is search-first (the ActorsPanel
    // live-search shape), so the row appears only once the search names it.
    await gm.getByTestId("playlists-search").fill("Tavern");
    await expect(gm.getByTestId("playlist-row")).toHaveCount(1);
    await gm.getByTestId("playlist-play").click();

    // GM sees exactly one live entry; the player's own panel reflects the SAME server-derived
    // `audio-state` singleton, so its `data-audio-playing` count agrees without the player
    // performing any action.
    await expect(gm.getByTestId("audio-panel")).toHaveAttribute("data-audio-playing", "1", { timeout: 15_000 });
    await expect(player.getByTestId("audio-panel")).toHaveAttribute("data-audio-playing", "1", { timeout: 15_000 });

    // The player sees the now-playing row but no transport controls: gated on `ctx.role ===
    // "gm"` in `AudioPanel.svelte`.
    await expect(player.getByTestId("playing-row")).toHaveCount(1);
    await expect(player.getByTestId("playing-pause")).toHaveCount(0);
    await expect(player.getByTestId("playing-stop")).toHaveCount(0);

    // GM pauses; both sides drop to 0.
    await gm.getByTestId("playing-pause").click();
    await expect(gm.getByTestId("audio-panel")).toHaveAttribute("data-audio-playing", "0", { timeout: 15_000 });
    await expect(player.getByTestId("audio-panel")).toHaveAttribute("data-audio-playing", "0", { timeout: 15_000 });
  } finally {
    await playerCtx.close();
  }
});
