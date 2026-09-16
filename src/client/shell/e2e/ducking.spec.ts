import { test, expect, login } from "./fixtures";
import type { WorkerAccount } from "./fixtures";

async function enterFreshWorld(
  page: import("@playwright/test").Page,
  name: string,
  account: WorkerAccount,
): Promise<void> {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(name);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(page.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });
}

// Only the key source is driven here: mic and OS sources need hardware, so their unit
// tests are the gate. Every selector below is verified against the ACTUAL merged
// `AudioPanel.svelte` and `Settings.svelte`/`DuckingSettings.svelte` markup — written against
// real DOM (settings labels, the `ducking.masterEnable`/`ducking.keySource.enable`
// i18n keys resolved to their English strings) plus `AudioPanel.svelte`'s `data-duck-gain`
// attribute naming the duck indicator; if the actual attribute name differs, use the
// real one and update this comment, never invent a selector unverified against the merged
// source.
test("holding the bound push-to-duck key drops the audio panel's duck gain, releasing restores it", async ({
  page,
  account,
}) => {
  await enterFreshWorld(page, "Ducking World", account);

  // Unlocks this device's `AudioContext` from the click gesture (`StatusBar`'s `audio-unlock`
  // control) — `AudioEngine.unlock()` is what starts the duck-gain smoothing loop
  // (`AudioEngine.#startDuckLoop`, driven off `requestAnimationFrame`); before it runs,
  // `DuckControllerImpl.tick()` is never called and `duck.gain` stays pinned at 1 no matter what
  // demand a source reports. `audio.spec.ts` establishes the same precedent.
  await page.getByTestId("audio-unlock").click();

  await page.getByTestId("topbar-settings").click();
  await expect(page.getByLabel("Enable voice ducking")).toBeChecked();
  await page.getByLabel("Enable push-to-duck key").check();

  // The default binding is Backquote; the settings section shows it once bound.
  await expect(page.getByRole("button", { name: /Bound to Backquote/ })).toBeVisible();

  // Open the audio panel to observe the duck-gain indicator.
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-audio:panel").click();

  await expect(page.locator("[data-duck-gain]")).toHaveAttribute("data-duck-gain", "1");

  await page.keyboard.down("Backquote");
  await expect
    .poll(async () => Number(await page.locator("[data-duck-gain]").getAttribute("data-duck-gain")))
    .toBeLessThan(1);

  await page.keyboard.up("Backquote");
  // `DuckControllerImpl.tick` is exponential one-pole smoothing (`approach`'s `1 -
  // exp(-elapsedMs/tauMs)` shape): it asymptotically approaches the released target of 1 but
  // never reaches it exactly within this test's window (reaching bit-exact `1` needs the
  // smoothed demand to underflow double precision's spacing near 1, tens of seconds past the
  // release tau) — so this asserts "recovered", not "bit-identical to the pre-duck reading".
  await expect
    .poll(async () => Number(await page.locator("[data-duck-gain]").getAttribute("data-duck-gain")))
    .toBeGreaterThan(0.99);
});
