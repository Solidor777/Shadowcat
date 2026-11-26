import { test, expect } from "@playwright/test";
import { CONNECTION_LEVEL_FAILURE, requirePrivateAddress } from "./https-support";
import { checkMeasuredOutcome, type OutcomeTable } from "../e2e-support/platform-outcomes";

// Typing the plain `http://` LAN address into a browser whose HTTPS-upgrade feature is on. The
// server answers TLS on that same port with an untrusted certificate, so an upgrade attempt lands
// on a certificate error. Acceptable outcomes, and only these:
//   - the app loads (the browser fell back to http, or upgraded and the page rendered), or
//   - the navigation fails with a CERTIFICATE error, a browser-owned interstitial the user can
//     bypass: proven by repeating the navigation with certificate errors ignored (what "proceed
//     anyway" does) and reaching a working app.
// A connection-level failure is a dead end with no bypass and fails the test.
//
// Which of the two happens is engine- and OS-specific, so the branch each project takes is pinned
// per (project, platform); a cell without a measurement is an `UNMEASURED` annotation locally and
// fails on CI with the observed value, so the table is completed from CI runs.

/** The measured branch per project and platform: `loaded:<protocol>` or `certificate-error`. */
const EXPECTED: OutcomeTable<string> = {
  "https-upgrade-firefox": { win32: "loaded:http:", linux: "loaded:http:", darwin: "loaded:http:" },
  "https-upgrade-firefox-named": { win32: "certificate-error", linux: "certificate-error", darwin: "certificate-error" },
  "https-upgrade-firefox-local": { win32: "certificate-error", linux: "certificate-error", darwin: "certificate-error" },
  "https-upgrade-webkit": { win32: "loaded:http:", linux: "loaded:http:", darwin: "loaded:http:" },
};

test("http://<lan-address> is never a dead end", async ({ page, baseURL }, testInfo) => {
  requirePrivateAddress(baseURL!, testInfo);
  let failure = "";
  try {
    await page.goto(`${baseURL}/`, { waitUntil: "domcontentloaded" });
  } catch (e) {
    failure = String(e);
  }

  if (failure === "") {
    await expect(page.getByLabel("Username")).toBeVisible();
    const finalProtocol = new URL(page.url()).protocol;
    testInfo.annotations.push({
      type: "upgrade-outcome",
      description: `app loaded over ${finalProtocol}`,
    });
    checkMeasuredOutcome(testInfo, EXPECTED, `loaded:${finalProtocol}`);
    return;
  }

  testInfo.annotations.push({
    type: "upgrade-outcome",
    description: `error: ${failure.split("\n")[0]}`,
  });
  expect(failure).not.toMatch(CONNECTION_LEVEL_FAILURE);
  checkMeasuredOutcome(testInfo, EXPECTED, "certificate-error");
  // "Proceed anyway": the same navigation with certificate errors ignored reaches the app.
  const bypass = await page.context().browser()!.newContext({ ignoreHTTPSErrors: true });
  try {
    const bypassPage = await bypass.newPage();
    await bypassPage.goto(`${baseURL}/`, { waitUntil: "domcontentloaded" });
    await expect(bypassPage.getByLabel("Username")).toBeVisible();
  } finally {
    await bypass.close();
  }
});
