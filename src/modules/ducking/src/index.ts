import type { Module } from "@shadowcat/core";

/** Voice ducking: a mic voice-activity source, an OS audio-session monitor source (the
 * `shadowcat audio-monitor` subcommand), and a push-to-duck key — three `DuckSource`s behind
 * the audio engine's `DuckController` contract. Contributes its settings section once the
 * settings-section component exists; wired to the real `ctx.audio.duck` once the real
 * integration is complete. */
export const ducking: Module = {
  manifest: {
    id: "ducking",
    version: "0.1.0",
    dependencies: {},
    requires: [],
    provides: [],
  },
  register() {
    // Extended later to contribute the `SETTINGS_SECTION_CONTRACT` section, once the
    // settings-section component exists.
  },
};
