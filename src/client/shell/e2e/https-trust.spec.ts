import { test, expect } from "@playwright/test";
import { CONNECTION_LEVEL_FAILURE, plainOrigin } from "./https-support";

// The self-signed certificate is untrusted by design. With certificate errors NOT ignored the
// browser must refuse the https page for the certificate (a bypassable, browser-owned
// interstitial), never for a connection-level reason, and the plain-http twin on the same port
// must keep working. The failure text is engine-specific, so the assertion is the class: the
// navigation fails, and not with a connection-level error. A negative check alone would also pass
// for an unrelated failure, so each engine's certificate-error wording is asserted positively;
// the patterns are deliberately loose enough to span OS builds of the same engine (measured:
// Chromium `ERR_CERT_AUTHORITY_INVALID`, Firefox `SSL_ERROR_UNKNOWN`, WebKit on Windows
// "SSL peer certificate or SSH remote key was not OK").
//
// WebKit on Linux reports the refusal in one of two wordings for the same navigation. Measured on
// ubuntu-24.04 runners with WebKit's `requestfailed` and frame events logged around each
// `page.goto` to the untrusted https origin: 142 of 150 repeated navigations in one browser and
// 37 of 40 in a fresh browser each failed with "Unacceptable TLS certificate"; the other 8 and 3,
// and 7 of 120 repeats of the test below alone, failed with "Operation was cancelled; maybe frame was
// detached?". In both wordings the log holds one `requestfailed` for the main request at about
// 25 ms (its error text is the same wording) and no frame-detach or frame-navigation event before
// `goto` settles, so the engine reports the refused certificate as a cancelled load and no frame
// is lost. Other engines and other platforms were measured with one wording and stay strict.

/** The certificate-refusal wording WebKit shares on every platform. */
const WEBKIT_CERTIFICATE_ERROR = /certificate|SSL|TLS/i;

/** WebKit on Linux additionally reports a refused certificate as a cancelled load. */
const WEBKIT_LINUX_CANCELLED_LOAD = /Operation was cancelled/;

/** What each engine's certificate-refusal message contains. */
const CERTIFICATE_ERROR: Record<string, RegExp> = {
  "https-chromium": /ERR_CERT_/,
  "https-firefox": /SSL_ERROR|SEC_ERROR|MOZILLA_PKIX/,
  "https-webkit":
    process.platform === "linux"
      ? new RegExp(`${WEBKIT_CERTIFICATE_ERROR.source}|${WEBKIT_LINUX_CANCELLED_LOAD.source}`, "i")
      : WEBKIT_CERTIFICATE_ERROR,
};

test.use({ ignoreHTTPSErrors: false });

test("the browser refuses the self-signed page for the certificate while plain http on the same port works", async ({
  page,
  context,
  baseURL,
}, testInfo) => {
  let message = "";
  try {
    await page.goto(`${baseURL}/`);
  } catch (e) {
    message = String(e);
  }
  testInfo.annotations.push({ type: "untrusted-cert-error", description: message.split("\n")[0] });
  expect(message, "navigation to an untrusted https page must fail").not.toBe("");
  expect(message).not.toMatch(CONNECTION_LEVEL_FAILURE);
  expect(message, "a certificate error in this engine's own wording").toMatch(
    CERTIFICATE_ERROR[testInfo.project.name],
  );

  // A fresh page: the failed navigation leaves its error document pending on `page`.
  const plainPage = await context.newPage();
  await plainPage.goto(`${plainOrigin(baseURL!)}/`);
  await expect(plainPage.getByLabel("Username")).toBeVisible();
});
