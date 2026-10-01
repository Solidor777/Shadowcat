import { test, expect, request } from "@playwright/test";
import { loginAt, plainOrigin } from "./https-support";

// The one port answers both schemes. A phone or laptop that types the plain `http://` LAN address
// gets a working, non-secure-context app whose entry view offers the HTTPS address; the cookie it
// receives is the plain-scheme name without `Secure`.

test("plain http on the TLS port works, offers the https address, and remembers a dismissal", async ({
  page,
  context,
  baseURL,
}) => {
  const secure = baseURL!;
  const plain = plainOrigin(secure);

  await page.goto(`${plain}/`);
  await expect(page.getByLabel("Username")).toBeVisible();
  expect(await page.evaluate(() => window.isSecureContext)).toBe(false);

  const notice = page.getByTestId("https-notice");
  await expect(notice).toBeVisible();
  // The link keeps the current route (the hash), so the player lands where they were.
  const href = await page.getByTestId("https-notice-link").first().getAttribute("href");
  expect(href!.startsWith(`${secure}/`)).toBe(true);
  expect(new URL(href!).protocol).toBe("https:");
  expect(new URL(href!).hash, "the link carries the current route").toBe(new URL(page.url()).hash);
  expect(new URL(href!).hash).not.toBe("");
  await expect(notice).toContainText("your browser will warn once");

  // Dismissal is remembered per origin across a reload.
  await page.getByTestId("https-notice-dismiss").click();
  await expect(notice).toBeHidden();
  await page.reload();
  await expect(page.getByLabel("Username")).toBeVisible();
  await expect(page.getByTestId("https-notice")).toHaveCount(0);

  await loginAt(page, plain);
  const session = (await context.cookies()).filter((c) => c.name.endsWith("sc_sid"));
  expect(session.map((c) => c.name)).toEqual(["sc_sid"]);
  expect(session[0].secure).toBe(false);
});

test("the same port completes a TLS handshake and publishes the same tls block over both schemes", async ({
  baseURL,
}, testInfo) => {
  // A request-level assertion: no engine is involved, so one project is enough.
  test.skip(testInfo.project.name !== "https-chromium", "engine-independent; asserted once under https-chromium");
  const api = await request.newContext({ ignoreHTTPSErrors: true });
  const overHttps = await (await api.get(`${baseURL}/api/config`)).json();
  const overHttp = await (await api.get(`${plainOrigin(baseURL!)}/api/config`)).json();
  expect(overHttps.tls.enabled).toBe(true);
  expect(overHttp.tls).toEqual(overHttps.tls);
  await api.dispose();
});
