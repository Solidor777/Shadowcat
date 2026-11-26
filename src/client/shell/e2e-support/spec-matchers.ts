/** Which Playwright projects run which spec files. The patterns live here, apart from the config,
 * so the config's project definitions and the partition tests read the SAME values: a spec's
 * legal runner set is whatever these matchers say, never a second copy of them. */

/** The test file that runs only under the `lan-plain-http` project. */
export const LAN_SPEC = /lan-plain-http\.spec\.ts$/;

/** The media-range spec. It runs only under `media-range-loopback`, against its own server started
 * with a short `egress_window_secs`, so a response the media element leaves idle between its reads
 * is cut within seconds; the shared loopback server never runs with that window, so no other spec
 * can see a spurious cut. */
export const MEDIA_RANGE_SPEC = /media-range\.spec\.ts$/;

/** The sampled long-audio streaming spec. It runs under the `long-audio-*` projects against a server
 * of its own with a short `egress_window_secs`; the play-through file name does not match. */
export const LONG_AUDIO_SPEC = /long-audio\.spec\.ts$/;

/** The full 20-minute real-time play-through spec, run under the two `long-audio-playthrough-*`
 * projects (one per protocol, each its own CI job) against the same long-audio server. */
export const LONG_AUDIO_PLAYTHROUGH_SPEC = /long-audio-playthrough\.spec\.ts$/;

/** The route-prefix spec. It runs only under the `route-prefix` project, against a server mounted
 * under a subpath (`Config.route_prefix`), where every same-origin request must carry the prefix. */
export const ROUTE_PREFIX_SPEC = /route-prefix\.spec\.ts$/;

/** The first-use onboarding spec. It runs only under the `onboarding` project, against a server of
 * its own with no seeded admin and the tour offers on: every other server runs with onboarding off
 * so the offer's toast never covers the controls a spec drives. */
export const ONBOARDING_SPEC = /onboarding\.spec\.ts$/;

/** The WebM/Ogg derivative decode spec. It runs under the two `webm-decode-*` projects, one per
 * engine whose `decodeAudioData` it measures, against the shared loopback server. */
export const WEBM_DECODE_SPEC = /webm-decode\.spec\.ts$/;

/** Specs that run under the three `https-*` browser projects (not the upgrade matrix). */
export const HTTPS_SPECS = /https-(?!upgrade)[a-z0-9-]+\.spec\.ts$/;

/** The Firefox/WebKit upgrade spec (the engine under test is the project's own browser). */
export const UPGRADE_SPEC = /https-upgrade\.spec\.ts$/;

/** The Chromium upgrade test file: it launches its own profiles because HTTPS-First Mode is a profile
 * preference, not a launch switch. */
export const UPGRADE_CHROMIUM_SPEC = /https-upgrade-chromium\.spec\.ts$/;

/** Every spec file named `https-*`. The loopback project ignores this, so a spec with that name can
 * never fall into the loopback project; each such spec must also match exactly one of
 * `HTTPS_SPECS`, `UPGRADE_SPEC` and `UPGRADE_CHROMIUM_SPEC`, which select its HTTPS project. */
export const ANY_HTTPS_NAMED = /https-.*\.spec\.ts$/;

/** The loopback project's HTTPS ignore list: the name catch-all, which covers the three project
 * selectors above and any new `https-*` spec alike. */
export const ANY_HTTPS_SPEC: readonly RegExp[] = [ANY_HTTPS_NAMED];
