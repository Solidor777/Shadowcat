<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { DiceEngine, type DieThrowSpec } from "./DiceEngine";
  import { shapeFor, realFaceCountOf } from "./shapes";
  import { readDice3DSettings } from "./settings";
  import {
    createTriggerState, seedFromSnapshot, scanForPlays,
    createRollQueue, enqueueRoll, dequeueRoll, toQueuedRoll,
    type RollQueue, type RollPlay, type QueuedRoll,
  } from "./trigger";
  import { dice3dEnabled, reducedMotionPreferred, antialiasPreferred } from "./performanceSeam";
  import { playThrowSound } from "./audioSeam";
  import { DICE_SETTINGS_DOC_TYPE, type DieRecord, type DiceSettingsEngine } from "@shadowcat/core";

  /** How long an idle (no roll tumbling, no roll queued) engine stays alive before
   * `dispose()` releases its WebGL context: disposed after 60 s idle,
   * re-created on the next roll. */
  const IDLE_DISPOSE_MS = 60_000;

  const ctx = getAppContext();
  let host: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  let engine: DiceEngine | null = null;
  let idleTimer: ReturnType<typeof setTimeout> | null = null;
  let queue: RollQueue = createRollQueue();
  let settledState = $state<"idle" | "tumbling" | "settled">("idle");
  let lastSettledValues = $state("");

  /** This device's resolved die-body and label colors (a named interface rather than an
   * inline object-literal type so both properties can be documented). */
  interface DeviceColors {
    /** The die-body color, a css color string. */
    color: string;
    /** The face-label glyph color, a css color string. */
    labelColor: string;
  }

  /** The face-shape subset `faceLabel` reads (mirrors one `WireDieKind.Faces.faces`
   * entry). */
  interface FaceLabelSource {
    /** Optional numeric reference value for the face. */
    value?: number | null;
    /** Symbol labels printed on this face. */
    symbols: string[];
  }

  /** Resolves a CSS custom property NAME (a design token — e.g. `--accent`, `--text-primary`)
   * to a css color string, by reading the computed `color` off a throwaway probe span — the
   * `Stage.svelte` `readColor` precedent, returning a string here (three's `CanvasTexture`
   * material takes a css color directly) rather than that helper's packed 0xRRGGBB number.
   * @param token The CSS custom property name to resolve.
   * @param fallback The css color returned when resolution fails (no `getComputedStyle`, or
   * `host` not yet mounted).
   * @returns The resolved css color, or `fallback`.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from `resolveDeviceColors`
   * readCssColor("--accent", "#2d6ee8");
   * ```
   */
  function readCssColor(token: string, fallback: string): string {
    if (typeof getComputedStyle !== "function" || !host) return fallback;
    const probe = document.createElement("span");
    probe.style.color = `var(${token})`;
    probe.style.display = "none";
    host.appendChild(probe);
    const rgb = getComputedStyle(probe).color;
    host.removeChild(probe);
    return rgb || fallback;
  }

  /** Resolves this device's die-body and label colors: a per-device override
   * (`readDice3DSettings`) when set, else the active theme's accent/primary-text tokens.
   * @returns The resolved body and label colors.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from `ensureEngine`
   * resolveDeviceColors();
   * ```
   */
  function resolveDeviceColors(): DeviceColors {
    const device = readDice3DSettings();
    return {
      color: device.color || readCssColor("--accent", "#2d6ee8"),
      labelColor: device.labelColor || readCssColor("--text-primary", "#ffffff"),
    };
  }

  /** Lazily constructs (or reuses) the engine, cancelling any pending idle-disposal timer.
   * @returns The live engine, initialized and ready to throw.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from `playQueued`
   * await ensureEngine();
   * ```
   */
  async function ensureEngine(): Promise<DiceEngine> {
    if (idleTimer !== null) {
      clearTimeout(idleTimer);
      idleTimer = null;
    }
    if (!engine) {
      engine = new DiceEngine(canvas, { antialias: antialiasPreferred(), ...resolveDeviceColors() });
      await engine.init();
    }
    return engine;
  }

  /** Schedules disposal after `IDLE_DISPOSE_MS` with nothing tumbling or queued.
   * @example
   * ```
   * // private helper; not part of the public API — invoked when the last roll dismisses
   * scheduleIdleDisposal();
   * ```
   */
  function scheduleIdleDisposal(): void {
    if (idleTimer !== null) clearTimeout(idleTimer);
    idleTimer = setTimeout(() => {
      if (queue.active.length === 0 && queue.pending.length === 0) {
        engine?.dispose();
        engine = null;
      }
    }, IDLE_DISPOSE_MS);
  }

  /** The label a `Faces` face renders: its symbols, else its numeric value.
   * @param f The face to label.
   * @returns The label string (comma-joined symbols, the numeric value, or `""`).
   * @example
   * ```
   * // private helper; not part of the public API — invoked from `throwSpecsFor`
   * faceLabel({ value: 3, symbols: [] }); // "3"
   * ```
   */
  function faceLabel(f: FaceLabelSource): string {
    return f.symbols.length > 0 ? f.symbols.join(",") : String(f.value ?? "");
  }

  /** Derives each die's physical-shape throw spec and up-face target from its `DieRecord`.
   * The remap target is derived from the record's FINAL `value` — never `natural`, which a
   * reroll/explode leaves at the pre-reroll draw.
   * @param records The roll's per-die records (records without a `kind` — a roll stored
   * before the face space became an every-recipient fact — are skipped, fail closed).
   * @returns One throw spec per renderable record.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from `playQueued`
   * throwSpecsFor([]);
   * ```
   */
  function throwSpecsFor(records: DieRecord[]): DieThrowSpec[] {
    return records
      .filter((r) => r.kind != null)
      .map((r) => {
        const kind = r.kind!;
        const shape = shapeFor(realFaceCountOf(kind));
        const allLabels =
          "Numeric" in kind
            ? Array.from({ length: shape.realFaceCount }, (_, i) => String(kind.Numeric.min + i))
            : kind.Faces.faces.map(faceLabel);
        let valueIndex: number;
        if ("Numeric" in kind) {
          valueIndex = r.value - kind.Numeric.min;
        } else {
          const byValue = kind.Faces.faces.findIndex((f) => f.value === r.value);
          valueIndex = byValue >= 0 ? byValue : kind.Faces.faces.findIndex((f) => faceLabel(f) === r.symbols.join(","));
        }
        valueIndex = Math.max(0, Math.min(shape.realFaceCount - 1, valueIndex));
        if (shape.sameLabel) {
          // Value chip: every physical face shows the final value; target face 0 is arbitrary.
          return { kind, labels: Array.from({ length: shape.physicalFaceCount }, () => allLabels[valueIndex] ?? String(r.value)), targetIndex: 0 };
        }
        return { kind, labels: allLabels, targetIndex: valueIndex };
      });
  }

  /** How many of a queued roll's dice actually render (the rest are the "+N" badge on
   * the chat card path).
   * @param q The queued roll.
   * @returns The renderable die count (`records.length - overflow`).
   * @example
   * ```
   * // private helper; not part of the public API — invoked from `playQueued`
   * declare const q: QueuedRoll;
   * specsCap(q);
   * ```
   */
  function specsCap(q: QueuedRoll): number {
    return q.outcome.records.length - q.overflow;
  }

  /** Plays one queued roll: throws its dice, waits for settle, records the settled values,
   * fades, then dequeues (promoting the next pending roll, if any).
   * @param q The queued roll to play.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from `handlePlay` and the
   * // dequeue-promotion path
   * declare const q: QueuedRoll;
   * await playQueued(q);
   * ```
   */
  async function playQueued(q: QueuedRoll): Promise<void> {
    settledState = "tumbling";
    const specs = throwSpecsFor(q.outcome.records.slice(0, specsCap(q)));
    if (reducedMotionPreferred()) {
      // Dice appear already settled (one frame) and fade — no tumble animation.
      settledState = "settled";
      lastSettledValues = q.outcome.records.map((r) => r.value).join(",");
    } else {
      const e = await ensureEngine();
      await e.throwDice(specs, q.rollId);
      settledState = "settled";
      lastSettledValues = q.outcome.records.map((r) => r.value).join(",");
    }
    setTimeout(() => {
      const result = dequeueRoll(queue, q.rollId);
      queue = result.queue;
      if (result.promoted) void playQueued(result.promoted);
      if (queue.active.length === 0) {
        settledState = "idle";
        scheduleIdleDisposal();
      }
    }, reducedMotionPreferred() ? 0 : 2500);
  }

  /** Reads the world's `dice-settings.sound` asset id, or `null` if the singleton doc is
   * absent or carries none — the same fail-closed-to-silent read the server's own
   * `DiceSettingsEngine::default()` implies.
   * @returns The sound asset id, or `null` for silent.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from `handlePlay`
   * diceThrowSound();
   * ```
   */
  function diceThrowSound(): string | null {
    const doc = ctx.documents.query(DICE_SETTINGS_DOC_TYPE)[0];
    const engine = doc?.engine as DiceSettingsEngine | undefined;
    return engine?.sound ?? null;
  }

  /** Handles one newly-triggered play: enqueues it (immediate play, or queued behind the
   * concurrency cap) and plays the sound cue at throw time.
   * @param play The play to handle.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the store scan and the
   * // `ctx.dice3d` bridge attach below
   * declare const play: RollPlay;
   * handlePlay(play);
   * ```
   */
  function handlePlay(play: RollPlay): void {
    if (!dice3dEnabled()) return; // seen-bookkeeping already ran in scanForPlays
    const queued = toQueuedRoll(play);
    const before = queue.active.length;
    queue = enqueueRoll(queue, queued);
    playThrowSound(diceThrowSound()); // audioSeam.ts is a no-op until `AudioApi` is wired in
    if (queue.active.length > before) void playQueued(queued);
  }

  $effect(() => {
    const state = createTriggerState();
    seedFromSnapshot(state, ctx.documents);
    const scan = (): void => {
      for (const play of scanForPlays(state, ctx.documents)) handlePlay(play);
    };
    const unsubscribeStore = ctx.documents.subscribe(scan);
    const detach = ctx.dice3d.attach({
      roll: (outcome, rollId) => handlePlay({ rollId, recalcCount: 0, outcome }),
      clear: () => {
        queue = createRollQueue();
        settledState = "idle";
      },
    });
    const observer = new ResizeObserver(() => {
      engine?.resize(host.clientWidth, host.clientHeight);
    });
    observer.observe(host);
    return () => {
      unsubscribeStore();
      detach();
      observer.disconnect();
      if (idleTimer !== null) clearTimeout(idleTimer);
      engine?.dispose();
      engine = null;
    };
  });

  /** A click on the overlay while dice are visible dismisses them.
   * @example
   * ```
   * // private helper; not part of the public API — bound to the dismiss layer's `onclick`
   * dismiss();
   * ```
   */
  function dismiss(): void {
    ctx.dice3d.clear();
  }
</script>

<div
  bind:this={host}
  class="dice3d-overlay"
  data-dice3d-state={settledState}
  data-dice3d-values={lastSettledValues}
>
  <canvas bind:this={canvas}></canvas>
  {#if settledState !== "idle"}
    <button
      type="button"
      class="dice3d-dismiss"
      aria-label={ctx.t("dice3d.dismiss")}
      onclick={dismiss}
    ></button>
  {/if}
</div>

<style lang="scss">
  .dice3d-overlay {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
  }
  .dice3d-dismiss {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: auto;
    background: transparent;
    border: none;
    cursor: pointer;
  }
</style>
