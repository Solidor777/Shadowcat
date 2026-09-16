import { VadEngine, VAD_PROCESSOR_NAME, VAD_FRAME_MS } from "./micVad";

/** The worklet node's construction options — the ambient `AudioWorkletNodeOptions` shape
 * narrowed to the one field this file reads. Named (rather than an inline type literal) so
 * each field carries its own doc comment once, shared by the base declaration and
 * `VadProcessor`'s own narrower override below. */
interface WorkletProcessorOptions {
  /** The options bag the constructing `AudioWorkletNode` was given, opaque at this base type
   * (`VadProcessorOptions.processorOptions` narrows it for `VadProcessor`). */
  processorOptions?: unknown;
}

/** Ambient shape of the AudioWorkletGlobalScope's per-processor base class — not part of the
 * default `dom` lib (that belongs to `@types/audioworklet`'s separate global environment,
 * which this project does not otherwise pull in), so this file declares only the minimal
 * surface it calls. This is a TYPE-ONLY declaration (`declare class` emits no runtime code);
 * the identifier `AudioWorkletProcessor` it names resolves to the REAL global that exists
 * inside an AudioWorkletGlobalScope at runtime — this file is loaded there via
 * `audioContext.audioWorklet.addModule(new URL("./vad.worklet.ts", import.meta.url))`, never
 * imported directly by test code (which is why `micVad.test.ts` tests `VadEngine` alone). */
declare class AudioWorkletProcessor {
  /** The port this processor exchanges messages with its owning `AudioWorkletNode` over. */
  readonly port: MessagePort;
  /**
   * Constructs the base processor from the worklet node's construction options.
   * @param options The worklet node's construction options.
   * @example
   * ```
   * // ambient declaration; never constructed directly — VadProcessor extends it
   * class VadProcessor extends AudioWorkletProcessor {}
   * ```
   */
  constructor(options?: WorkletProcessorOptions);
  /**
   * The AudioWorklet render-quantum callback.
   * @param inputs The node's input channels for this render quantum.
   * @param outputs The node's output channels for this render quantum.
   * @param parameters The node's AudioParam values for this render quantum.
   * @returns Whether to keep the processor alive for the node's lifetime.
   * @example
   * ```
   * // ambient declaration; VadProcessor overrides this method
   * declare const processor: { process(i: Float32Array[][], o: Float32Array[][], p: Record<string, Float32Array>): boolean };
   * declare const inputs: Float32Array[][];
   * declare const outputs: Float32Array[][];
   * declare const parameters: Record<string, Float32Array>;
   * processor.process(inputs, outputs, parameters);
   * ```
   */
  process(
    inputs: Float32Array[][],
    outputs: Float32Array[][],
    parameters: Record<string, Float32Array>,
  ): boolean;
}
/**
 * Ambient `registerProcessor`, present only inside an AudioWorkletGlobalScope.
 * @param name The processor name the worklet node constructs by.
 * @param processorCtor The `AudioWorkletProcessor` subclass to register under `name`.
 * @example
 * ```
 * registerProcessor(VAD_PROCESSOR_NAME, VadProcessor);
 * ```
 */
declare function registerProcessor(name: string, processorCtor: unknown): void;
/** Ambient per-worklet sample-rate global. */
declare const sampleRate: number;

/** `VadProcessor`'s narrowed `processorOptions` shape — the sensitivity multiplier
 * `MicVadSource.enable()` passes through `AudioWorkletNodeOptions.processorOptions`. */
interface VadProcessorOptions {
  /** The sensitivity-carrying options bag. */
  processorOptions?: {
    /** Seeds the underlying `VadEngine`'s sensitivity multiplier (defaults to 2.5 if absent). */
    sensitivity?: number;
  };
}

/**
 * Runs entirely inside the AudioWorkletGlobalScope: accumulates input samples into 20 ms
 * frames, computes each frame's RMS, feeds it to a `VadEngine`, and posts ONLY the resulting
 * boolean over `port` — never a sample, buffer, or RMS value (the privacy invariant: the
 * audio itself never leaves the worklet).
 */
class VadProcessor extends AudioWorkletProcessor {
  /** The adaptive VAD state machine, constructed from `processorOptions.sensitivity`. */
  private readonly engine: VadEngine;
  /** Samples accumulated toward the next `frameSamples`-sized frame. */
  private buffer: number[] = [];
  /** Frame size in samples at this worklet's `sampleRate`. */
  private readonly frameSamples: number;

  /**
   * Constructs the processor from the worklet node's `processorOptions`.
   * @param options The worklet node's construction options.
   * @example
   * ```
   * // registered below via registerProcessor(); never constructed directly by this module
   * registerProcessor(VAD_PROCESSOR_NAME, VadProcessor);
   * ```
   */
  constructor(options?: VadProcessorOptions) {
    super(options);
    const sensitivity = options?.processorOptions?.sensitivity ?? 2.5;
    this.engine = new VadEngine({ sensitivity, frameMs: VAD_FRAME_MS });
    this.frameSamples = Math.round((sampleRate * VAD_FRAME_MS) / 1000);
  }

  /**
   * The AudioWorklet render-quantum callback: accumulates samples and posts a boolean demand
   * once per 20 ms frame.
   * @param inputs The node's input channels for this render quantum.
   * @returns `true` to keep the processor alive for the node's lifetime.
   * @example
   * ```
   * // invoked by the AudioWorklet runtime, never called directly by this module
   * declare const processor: VadProcessor;
   * declare const inputs: Float32Array[][];
   * processor.process(inputs);
   * ```
   */
  process(inputs: Float32Array[][]): boolean {
    const channel = inputs[0]?.[0];
    if (channel) {
      for (const sample of channel) {
        this.buffer.push(sample);
        if (this.buffer.length >= this.frameSamples) {
          const rms = computeRms(this.buffer);
          this.buffer = [];
          this.port.postMessage(this.engine.pushFrame(rms) === 1);
        }
      }
    }
    return true; // keep the processor alive for the node's lifetime
  }
}

/**
 * Root-mean-square amplitude of `samples`.
 * @param samples The accumulated frame's samples.
 * @returns The frame's RMS amplitude.
 * @example
 * ```
 * const rms = computeRms([0.1, -0.2, 0.05]);
 * ```
 */
function computeRms(samples: number[]): number {
  const sumSquares = samples.reduce((sum, s) => sum + s * s, 0);
  return Math.sqrt(sumSquares / samples.length);
}

// Only present inside an AudioWorkletGlobalScope; `typeof` on an undeclared global safely
// evaluates to `"undefined"` (never throws) in every other environment, including this
// module never being imported by a Vitest test in the first place.
if (typeof registerProcessor === "function") {
  registerProcessor(VAD_PROCESSOR_NAME, VadProcessor);
}
