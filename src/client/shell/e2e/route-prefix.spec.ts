import { expect, test } from "./fixtures";

// Runs only under the `route-prefix` project: a server mounted under `/ttrpg`, whose base URL is
// `http://127.0.0.1:<port>/ttrpg/`. The shared `login` helper opens the origin root with an
// absolute `/`, which would leave the prefix, so the test drives the form from a relative
// `./`. The seeded admin identity is the one the test servers boot with.

/** The subpath the project's server is mounted under. */
const PREFIX = "/ttrpg";

test("a server mounted under a route prefix serves the whole client through it", async ({ page, baseURL }) => {
  const sameOrigin = new URL(baseURL!).origin;
  const requested: string[] = [];
  const sockets: string[] = [];
  page.on("request", (request) => {
    if (request.url().startsWith(sameOrigin)) requested.push(new URL(request.url()).pathname);
  });
  page.on("websocket", (socket) => sockets.push(new URL(socket.url()).pathname));

  await page.goto("./");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();

  // Unique name: the test server persists across local runs (reuseExistingServer).
  await page.getByLabel("New world name").fill(`Prefix ${Date.now().toString(36)}`);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(page.getByTestId("stage-canvas")).toBeVisible();

  // The page, its scripts, every REST call and the socket all went through the prefix.
  expect(requested.length).toBeGreaterThan(0);
  expect(requested.filter((path) => path !== PREFIX && !path.startsWith(`${PREFIX}/`))).toEqual([]);
  expect(requested.some((path) => path.startsWith(`${PREFIX}/api/`))).toBe(true);
  expect(sockets).toEqual([`${PREFIX}/ws`]);
});

test("nothing is served outside the prefix", async ({ request, baseURL }) => {
  const origin = new URL(baseURL!).origin;
  expect((await request.get(`${origin}/api/config`)).ok()).toBe(false);
  expect((await request.get(`${origin}${PREFIX}/api/config`)).ok()).toBe(true);
});
