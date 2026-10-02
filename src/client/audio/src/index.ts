export { AudioEngine, SYNC_TICK_MS, type AudioEngineOpts, type AudioTimer, type VisibilitySource } from "./engine";
export { EmitterPlayer, AUDIBILITY_RAMP_TAU_SECS } from "./emitter-player";
export { TrackPlayer, setMediaElementFactory, SYNC_SEEK_THRESHOLD_SECS, SYNC_RATE_NUDGE, SYNC_RATE_ENGAGE_SECS, SYNC_RATE_RELEASE_SECS, rateForDrift } from "./track-player";
export { OneShotPlayer, ONE_SHOT_CACHE_BUDGET_BYTES } from "./one-shot-player";
export { DuckControllerImpl, approach, DUCK_ATTACK_MS, DUCK_RELEASE_MS, DEFAULT_DUCK_DEPTH } from "./duck-controller";
export { decodeAudioCandidate, decodeCandidates, type DecodePreference } from "./decode";
export { createOggOpusDecoder } from "./wasm";
export type {
  AudioBufferLike,
  AudioContextLike,
  AudioNodeLike,
  AudioParamLike,
  BufferSourceNodeLike,
  GainNodeLike,
  MediaElementLike,
  MediaElementSourceNodeLike,
  PannerNodeLike,
  WasmOpusDecoderLike,
  WasmOpusDecodeResult,
} from "./context";
