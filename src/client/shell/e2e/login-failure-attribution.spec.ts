import { test, expect, login } from "./fixtures";

// Positive control for `login`'s load-failure guard: a failed same-origin `/assets/` fetch must
// fail the call promptly and name the request, instead of surfacing as a 120 s timeout waiting for
// a login form that can never mount. Without this control the guard could be inert (green while
// enforcing nothing), since the failure it detects only occurs sporadically on a loaded host.
test("a failed app-module fetch fails login at once and names the request", async ({ page }) => {
  await page.route("**/assets/*.js", (route) => route.abort("connectionreset"));
  const started = Date.now();
  await expect(login(page, "nobody", "nothing")).rejects.toThrow(/login page never loaded: .*\/assets\/.*ERR_CONNECTION_RESET/);
  expect(Date.now() - started).toBeLessThan(15_000);
});

// The guard must not fire on a healthy load: a full login round trip still completes.
test("login still succeeds when every fetch succeeds", async ({ page, account }) => {
  await login(page, account.username, account.password);
  await expect(page.getByLabel("New world name")).toBeVisible();
});
