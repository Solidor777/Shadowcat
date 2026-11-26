import { test, expect } from "@playwright/test";
import { loginAt, meStatus, plainOrigin, sessionCookieNames } from "./https-support";

// One browser context signs in over https, then over plain http, then over https again. The two
// schemes use different cookie names (`__Host-sc_sid` Secure / `sc_sid` non-Secure), so neither
// session can shadow, overwrite or delete the other. A single shared name would let the
// plain-http cookie collide with the Secure one in the jar, which is the failure this test pins.

test("https and plain-http sessions coexist and each logout removes only its own cookie", async ({
  page,
  context,
  baseURL,
}) => {
  const secure = baseURL!;
  const plain = plainOrigin(secure);

  await loginAt(page, secure);
  expect(await sessionCookieNames(context)).toEqual(["__Host-sc_sid"]);

  const plainPage = await context.newPage();
  await loginAt(plainPage, plain);
  expect(await sessionCookieNames(context)).toEqual(["__Host-sc_sid", "sc_sid"]);

  // Both sessions are live at once, each authenticated by its own cookie.
  expect(await meStatus(page)).toBe(200);
  expect(await meStatus(plainPage)).toBe(200);

  // HTTPS logout must really delete the Secure cookie (the removal cookie carries Secure and
  // Path=/, otherwise the browser ignores it) and must leave the plain session alone.
  await page.evaluate(() => fetch("/api/logout", { method: "POST" }));
  expect(await sessionCookieNames(context)).toEqual(["sc_sid"]);
  expect(await meStatus(page)).toBe(401);
  expect(await meStatus(plainPage)).toBe(200);

  // A second https sign-in works with the plain cookie still in the jar.
  await loginAt(page, secure);
  expect(await sessionCookieNames(context)).toEqual(["__Host-sc_sid", "sc_sid"]);
  expect(await meStatus(page)).toBe(200);

  // Plain logout removes only the plain cookie.
  await plainPage.evaluate(() => fetch("/api/logout", { method: "POST" }));
  expect(await sessionCookieNames(context)).toEqual(["__Host-sc_sid"]);
  expect(await meStatus(page)).toBe(200);
  expect(await meStatus(plainPage)).toBe(401);
});
