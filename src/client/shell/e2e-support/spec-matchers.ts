/** Which Playwright projects run which spec files. The patterns live here, apart from the config,
 * so the config's project definitions and the partition tests read the SAME values: a spec's
 * legal runner set is whatever these matchers say, never a second copy of them. */

/** The test file that runs only under the `lan-plain-http` project. */
export const LAN_SPEC = /lan-plain-http\.spec\.ts$/;

/** Specs that run under the three `https-*` browser projects (not the upgrade matrix). */
export const HTTPS_SPECS = /https-(?!upgrade)[a-z0-9-]+\.spec\.ts$/;

/** The Firefox/WebKit upgrade spec (the engine under test is the project's own browser). */
export const UPGRADE_SPEC = /https-upgrade\.spec\.ts$/;

/** The Chromium upgrade test file: it launches its own profiles because HTTPS-First Mode is a profile
 * preference, not a launch switch. */
export const UPGRADE_CHROMIUM_SPEC = /https-upgrade-chromium\.spec\.ts$/;

/** Every spec that runs against the HTTPS server instead of the loopback one: the union of the
 * three HTTPS matchers, so the loopback project's ignore list cannot drift from them. */
export const ANY_HTTPS_SPEC: readonly RegExp[] = [HTTPS_SPECS, UPGRADE_SPEC, UPGRADE_CHROMIUM_SPEC];
