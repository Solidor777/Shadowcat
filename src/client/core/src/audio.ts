import type { WireAudioOp } from "./wire";

/** A device-independent audio bus. Mirrors `crate::data::engine::audio::AudioChannel`'s three
 * server-known values PLUS the two client-only mixer buses (`"master"`, `"ui"`) the server
 * never sees — a track or emitter always names one of the three server values; `"master"`/
 * `"ui"` exist only as `AudioApi.channels` keys for device volume/mute. */
export type AudioChannelId = "master" | "music" | "ambience" | "sfx" | "ui";

/** One channel's device gain + mute state (the values of `AudioApi.channels`). */
export interface AudioChannelState {
  /** Device gain multiplier, `0..=1`. */
  gain: number;
  /** Whether the channel is muted (mixed at gain 0 regardless of `gain`). */
  muted: boolean;
}

/** A registered ducking demand source (`DuckController.addSource`). Any number may be active;
 * the controller's effective gain is `1 - max(every source's current demand) * depth`. */
export interface DuckSource {
  /** Set this source's demand, `0..=1` (1 = fully ducked). The controller takes the max across
   * every registered source, never a sum (two simultaneous full-demand sources duck no harder
   * than one).
   * @param level The new demand level. */
  set(level: number): void;
}

/** The mixer's master ducking bus: any number of independent demand sources (mic voice-
 * activity, an OS audio-session monitor, a manual push-to-duck key) feed one shared duck gain,
 * applied to the `duckable` channels (music + ambience by default). */
export interface DuckController {
  /** Register a ducking source; returns its handle. Any number may be active simultaneously.
   * @param id A stable identifier for this source — it IS the dedup key: calling this twice
   * with the same `id` replaces the first source's demand slot rather than adding a second.
   * @returns The handle this source uses to report its demand. */
  addSource(id: string): DuckSource;
  /** Unregister a source by the id it was added with. A source that never registered is a
   * no-op.
   * @param id The source's id, as passed to `addSource`. */
  removeSource(id: string): void;
  /** The current effective duck gain, `0..=1` (1 = no ducking). Reactive: a caller reading this
   * through a framework-provided reactive wrapper (see `@shadowcat/ui-kit`'s shell wiring)
   * re-renders when it changes. */
  readonly gain: number;
  /** How hard an active demand source ducks, `0..=1` (the effective gain floor is `1 - depth`
   * when demand is fully saturated). Per-device: seeded from and persisted to this device's own
   * `shadowcat.audio` mirror, never server state. Reactive, same wrapper as `gain`. */
  readonly depth: number;
  /** Change the duck depth; clamped to `0..=1`. Re-targets the smoothed gain on the next tick
   * (does not jump instantly) and persists the new value to this device's `shadowcat.audio`
   * mirror.
   * @param depth The new depth, `0..=1`. */
  setDepth(depth: number): void;
}

/** The per-device mixer + transport-observer seam every UI surface reads. Server-side ownership
 * (the `playlist`/`audio-state` engine doc types, the `"audibility"` derived channel, the
 * transcode pipeline) lives entirely in the Rust server; this interface is the CLIENT's own
 * device state (channel gains/mutes, ducking, one-shot playback) plus a thin wrapper over the
 * `AudioTransport`/`AudioListenAs` wire frames. Exposed as `AppContext.audio`. */
export interface AudioApi {
  /** Per-channel device gain + mute state, keyed by `AudioChannelId` (all five buses, including
   * the client-only `"master"`/`"ui"`). Reactive when read through the shell's context wrapper. */
  readonly channels: Record<AudioChannelId, AudioChannelState>;
  /** Adjust one channel's device gain and/or mute state; omitted fields are unchanged.
   * @param id The channel to adjust.
   * @param patch The fields to change.
   * @param patch.gain The new gain, `0..=1`; omitted = unchanged.
   * @param patch.muted The new mute state; omitted = unchanged. */
  setChannel(id: AudioChannelId, patch: {
    /** The new gain, `0..=1`; omitted = unchanged. */
    gain?: number;
    /** The new mute state; omitted = unchanged. */
    muted?: boolean;
  }): void;
  /** Unlocks this device's `AudioContext` — Web Audio requires a user gesture before any sound
   * can play; the shell's `AudioUnlock` control calls this from a click handler. Idempotent:
   * calling it again after a successful unlock resolves immediately.
   * @returns Resolves once the context is running. */
  unlock(): Promise<void>;
  /** The shared engine `AudioContext`, for a consumer that needs to register its own
   * `AudioWorkletNode` against the SAME graph `AudioApi` otherwise owns entirely (the
   * ducking module's `MicVadSource`). `null` until `unlock()` resolves —
   * Web Audio requires a user gesture before a context exists at all.
   * @returns The engine's `AudioContext`, or `null` before `unlock()` resolves. */
  context(): AudioContext | null;
  /** The shared ducking bus (see `DuckController`). */
  readonly duck: DuckController;
  /** Play a one-shot sound effect by asset id, at channel gain (times an optional per-call
   * override) — used by a VFX's paired sound and UI cues. Does nothing if the device is not
   * yet unlocked (the pending state is NOT queued for one-shots, unlike transport state — a
   * missed UI cue is inconsequential).
   * @param asset Asset id of the sound to play.
   * @param opts Optional channel override (default `"sfx"`) and gain multiplier.
   * @param opts.channel The bus to play through; default `"sfx"`.
   * @param opts.gain Per-call gain multiplier against the channel's own gain; default `1`. */
  playOneShot(asset: string, opts?: {
    /** The bus to play through; default `"sfx"`. */
    channel?: AudioChannelId;
    /** Per-call gain multiplier against the channel's own gain; default `1`. */
    gain?: number;
  }): void;
  /** The calibrated server clock, ms — a thin forwarder to `WsClient.serverNow()`. Drives live
   * playlist-position readouts (a playhead computed from `PlayingTrack.startedAt` against this
   * clock, never `Date.now()` directly).
   * @returns The current calibrated server time, ms. */
  serverNow(): number;
  /** Send a GM-only transport op — a thin forwarder to `WsClient.audioTransport(op)`.
   * Fire-and-forget: success is the broadcast `audio-state` Update echo, a refusal arrives as
   * `onAudioError`.
   * @param op The transport operation to apply. */
  transport(op: WireAudioOp): void;
  /** GM-only: set (or clear) this device's spatial-audio listening token — a preview seam
   * independent of any token this connection owns (`ClientMsg::AudioListenAs`). Fire-and-forget,
   * no correlated reply (mirrors `transport`'s own contract): takes effect on the next
   * `"audibility"` channel push. A non-GM caller is a no-op server-side (the frame is simply
   * ignored — `AudioListenAs` carries no refusal path since it can never disclose anything a
   * GM does not already see).
   * @param token The token to listen as, or `null` to clear the override. */
  listenAs(token: string | null): void;
}
