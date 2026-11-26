import { test, expect, request } from "@playwright/test";
import { loginAt, OPS } from "./https-support";
import { ALWAYS_PRESENT, measureSecureContextMembers, presentMembers } from "../e2e-support/secure-context";

// Runs under `https-chromium`, `https-firefox` and `https-webkit` against the machine's real
// non-loopback interface address. No stage is mounted. It measures:
//   - the browser's own secure-context verdict and the `__Host-` session cookie attributes;
//   - the shared `[SecureContext]` member table (`e2e-support/secure-context.ts`) on the HTTPS
//     origin and on `http://127.0.0.1` (a potentially trustworthy origin, so a secure-context
//     control served by the same server): the two present sets must be identical, and non-empty
//     on Chromium and Firefox;
//   - that `crypto.randomUUID` is a function on the HTTPS origin, the counterpart of its absence
//     on plain HTTP in `lan-plain-http.spec.ts`;
//   - the request-level behaviour the later tests name.

test("an https page is a secure context and its session cookie carries the __Host- attributes", async ({
  page,
  context,
  baseURL,
}) => {
  await loginAt(page, baseURL!);
  const shape = await page.evaluate(() => ({
    protocol: location.protocol,
    isSecureContext: window.isSecureContext,
  }));
  expect(shape).toEqual({ protocol: "https:", isSecureContext: true });

  const sessions = (await context.cookies()).filter((c) => c.name.endsWith("sc_sid"));
  expect(sessions.map((c) => c.name)).toEqual(["__Host-sc_sid"]);
  // `SameSite` is asserted on the raw header below, not on the jar: the WebKit build Playwright
  // ships for Windows reports `sameSite: "None"` for a cookie whose header says `SameSite=Lax`.
  expect(sessions[0]).toMatchObject({ secure: true, httpOnly: true, path: "/" });
  // A `__Host-` cookie is host-only: the jar entry must not carry a leading-dot Domain.
  expect(sessions[0].domain.startsWith(".")).toBe(false);
});

test("the [SecureContext] member table measures the same on the https LAN origin as on the secure loopback control", async ({
  page,
  baseURL,
  browserName,
}) => {
  const httpsOrigin = baseURL!;
  // `127.0.0.1` is a potentially trustworthy origin, so an `http://127.0.0.1` page is a secure
  // context: the control shows what a secure context exposes in this engine. The address is used
  // rather than the name `localhost`, which some resolvers map to `::1` only.
  const control = `http://127.0.0.1:${new URL(httpsOrigin).port}`;
  await page.goto(`${httpsOrigin}/`);
  const onHttps = await measureSecureContextMembers(page);
  // The UUID member is `[SecureContext]`: present on this HTTPS origin, absent on plain HTTP (the
  // `lan-plain-http` project), which is why the client builds UUIDs through `randomUuid`.
  expect(await page.evaluate(() => typeof crypto.randomUUID)).toBe("function");
  await page.goto(`${control}/`);
  const onControl = await measureSecureContextMembers(page);
  test.info().attach("measured-https", { body: JSON.stringify(onHttps, null, 2), contentType: "application/json" });
  test.info().attach("measured-control", { body: JSON.stringify(onControl, null, 2), contentType: "application/json" });

  expect(onHttps.isSecureContext).toBe("true");
  expect(onControl.isSecureContext).toBe("true");
  for (const [name, type] of Object.entries(ALWAYS_PRESENT)) {
    expect(onHttps[name], name).toBe(type);
    expect(onControl[name], name).toBe(type);
  }
  expect(presentMembers(onHttps)).toEqual(presentMembers(onControl));
  if (browserName === "chromium" || browserName === "firefox") {
    expect(presentMembers(onHttps).length, "a vacuous measurement proves nothing").toBeGreaterThan(0);
  }
});

test("the login response sets the __Host- cookie with Secure, HttpOnly, SameSite=Lax, Path=/ and no Domain", async ({
  baseURL,
}, testInfo) => {
  // A request-level assertion: no engine is involved, so one project is enough.
  test.skip(testInfo.project.name !== "https-chromium", "engine-independent; asserted once under https-chromium");
  const api = await request.newContext({ ignoreHTTPSErrors: true });
  const res = await api.post(`${baseURL}/api/login`, { data: OPS });
  expect(res.status()).toBe(204);
  const setCookies = res
    .headersArray()
    .filter((h) => h.name.toLowerCase() === "set-cookie")
    .map((h) => h.value);
  const session = setCookies.filter((c) => c.startsWith("__Host-sc_sid="));
  expect(session).toHaveLength(1);
  const attrs = session[0].split(";").map((a) => a.trim().toLowerCase());
  expect(attrs).toEqual(expect.arrayContaining(["secure", "httponly", "samesite=lax", "path=/"]));
  expect(attrs.some((a) => a.startsWith("domain="))).toBe(false);
  await api.dispose();
});
