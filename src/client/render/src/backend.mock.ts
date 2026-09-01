import type { DisplayBackend, BackgroundSpec } from "./backend";
import type { LineSeg, CameraTransform, VisibilityInput, TokenNodeSpec, ShapeNodeSpec, Point } from "./types";
import type { LightingFrame } from "./lighting";
import type { PingRing } from "./ping-view";

/** A recording DisplayBackend for unit tests — never touches Pixi/GL. */
export class MockBackend implements DisplayBackend {
  /** Last `ensureLayers` z-order, recorded verbatim — see `ensureLayers`'s doc for the
   * shrinking-set divergence from `PixiBackend`. */
  layers: string[] = [];
  /** Last `setBackground` spec, recorded verbatim; `null` before the first call, or after a
   * clear (`setBackground(null)`). */
  background: BackgroundSpec | null = null;
  /** Count of lines passed to the last `drawGrid` call — the geometry itself is discarded. */
  gridLineCount = 0;
  /** Color passed to the last `drawGrid` call, `0xRRGGBB`. */
  gridColor: number | null = null;
  /** Last `setCameraTransform` value, recorded verbatim. */
  camera: CameraTransform | null = null;
  /** Last applied visibility mask — either the last `setVisibility` input, or the
   * snapped-to endpoint of the last `setVisibilityBlend` call; `null` before either has ever
   * been called (the fog layer has no default mask to fall back on). */
  visibility: VisibilityInput | null = null;
  /** Last `setVisibilityBlend` call recorded verbatim (from/to/factor), for asserting the
   * cross-fade advances 0→1 across a sample interval. */
  visibilityBlend: {
    /** See `DisplayBackend.setVisibilityBlend`'s `from` param. */
    from: VisibilityInput;
    /** See `DisplayBackend.setVisibilityBlend`'s `to` param. */
    to: VisibilityInput;
    /** See `DisplayBackend.setVisibilityBlend`'s `factor` param. */
    factor: number;
  } | null = null;
  /** Last `resize` call, recorded verbatim. */
  size: {
    /** See `DisplayBackend.resize`'s `width` param. */
    width: number;
    /** See `DisplayBackend.resize`'s `height` param. */
    height: number;
  } | null = null;
  /** Every `addLayerFilter` registration not yet disposed, in call order. */
  filters: Array<{
    /** Target core-layer id, as given (not validated). */
    layerId: string;
    /** The opaque filter value passed to `addLayerFilter`. */
    filter: unknown;
  }> = [];
  /** Every token render node, keyed by document id, reflecting the last `setToken` spec. */
  tokens = new Map<string, TokenNodeSpec>();
  /** Every shape render node, keyed by document id, reflecting the last `setShape` spec. */
  shapes = new Map<string, ShapeNodeSpec>();
  /** Last `drawOverlay` shapes, recorded verbatim (empty after `clearOverlay`). */
  overlay: Omit<ShapeNodeSpec, "layer">[] = [];
  /** Last `drawMeasure` call, recorded verbatim (`null` after `clearMeasure`). */
  measure: {
    /** See `DisplayBackend.drawMeasure`'s `from` param. */
    from: Point;
    /** See `DisplayBackend.drawMeasure`'s `to` param. */
    to: Point;
    /** See `DisplayBackend.drawMeasure`'s `label` param. */
    label: string;
  } | null = null;
  /** Last `drawPings` rings, recorded verbatim. */
  pings: PingRing[] = [];
  /** Last `setLighting` frame, recorded verbatim. */
  lighting: LightingFrame | null = null;
  /** The callback recorded by `startTicker`, driven manually via `runTicker` — see
   * `startTicker`'s doc for the overwrite-on-repeat-call divergence from `PixiBackend`. */
  tick: ((dtMs: number) => void) | undefined;
  /** Set `true` by `destroy` — see `destroy`'s doc for the idempotent-vs-throwing divergence
   * from `PixiBackend`. */
  destroyed = false;

  /** `DisplayBackend.ensureLayers`: records the requested z-order verbatim into `this.layers`,
   * replacing any previous value. Unlike `PixiBackend`, which idempotently creates/re-parents real
   * Containers, this is a plain overwrite with no notion of "already exists" — a test asserts the
   * layer order by reading `this.layers` directly. **Divergence on a SHRINKING set:** a repeat
   * call that omits an id present in an earlier call makes that id vanish entirely from
   * `this.layers` here, whereas `PixiBackend.ensureLayers` never removes an omitted layer's
   * Container — it survives, un-reparented, and sinks to the bottom of the real z-stack instead
   * of disappearing. A test asserting "an omitted layer is gone" would pass against this mock and
   * fail against the real backend (which would show the layer still present, just misordered).
   * @param orderedIds The z-order (bottom to top) of core layer ids to record.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.ensureLayers(["background", "grid", "tokens"]);
   * backend.layers; // ["background", "grid", "tokens"]
   * ```
   */
  ensureLayers(orderedIds: string[]): void {
    this.layers = [...orderedIds];
  }
  /** `DisplayBackend.setBackground`: records `spec` verbatim into `this.background`. Unlike
   * `PixiBackend`, which loads the texture asynchronously (guarded by a monotonic load token) and
   * skips a same-`url` reload, this is a synchronous, unconditional overwrite — a test observes the
   * requested spec immediately, with no load-in-flight state to model.
   * @param spec The background image `url` to record, or `null` to clear it.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.setBackground({ url: "https://example.test/map.png" });
   * backend.background; // { url: "https://example.test/map.png" }
   * ```
   */
  setBackground(spec: BackgroundSpec | null): void {
    this.background = spec;
  }
  /** `DisplayBackend.drawGrid`: records only `lines.length` (as `gridLineCount`) and `color` — NOT
   * the line geometry itself. Unlike `PixiBackend`, which strokes every segment's exact
   * coordinates, a test using this mock can assert line COUNT and color but cannot assert the
   * drawn geometry; a test needing exact grid-line positions must exercise `PixiBackend` (or
   * `Grid.lines()` directly, upstream of the backend).
   * @param lines The grid line segments (only the count is retained).
   * @param color The stroke color, packed `0xRRGGBB`.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.drawGrid([{ x1: 0, y1: 0, x2: 100, y2: 0 }], 0x3a3a4a);
   * backend.gridLineCount; // 1
   * ```
   */
  drawGrid(lines: LineSeg[], color: number): void {
    this.gridLineCount = lines.length;
    this.gridColor = color;
  }
  /** `DisplayBackend.setCameraTransform`: records `t` verbatim into `this.camera`.
   * @param t The camera transform: translation `(x,y)` plus uniform `scale`.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.setCameraTransform({ x: 0, y: 0, scale: 1 });
   * backend.camera; // { x: 0, y: 0, scale: 1 }
   * ```
   */
  setCameraTransform(t: CameraTransform): void {
    this.camera = t;
  }
  /** `DisplayBackend.setVisibility`: records `input` verbatim into `this.visibility` and clears
   * `this.visibilityBlend` — mirroring `PixiBackend.setVisibility`'s contract that a plain apply
   * ends any in-flight cross-fade, without modeling the GPU-texture teardown that entails on the
   * real backend.
   * @param input The visibility mask to record.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.setVisibility({ mode: "all", visible: [], explored: [], perceived: [] });
   * backend.visibility; // { mode: "all", visible: [], explored: [], perceived: [] }
   * ```
   */
  setVisibility(input: VisibilityInput): void {
    this.visibility = input;
    this.visibilityBlend = null;
  }
  /** `DisplayBackend.setVisibilityBlend`: records the `{from, to, factor}` triple verbatim into
   * `this.visibilityBlend` (so a test can assert the exact blend inputs). **Divergence from
   * `PixiBackend`:** this does NOT model a cross-fade — `this.visibility` is set to a hard SNAP at
   * `from` or `to` (whichever `factor` is nearer, `factor < 0.5 ? from : to`), never an
   * interpolated/blended value. A test reading `this.visibility` mid-blend sees one endpoint
   * exactly, not the two-texture alpha blend `PixiBackend.setVisibilityBlend` actually renders —
   * assert the recorded `visibilityBlend` triple (or `factor`) directly when the blend progression
   * itself is under test.
   * @param from The outgoing sample's visibility mask.
   * @param to The incoming sample's visibility mask.
   * @param factor Blend position in `[0,1]`; `< 0.5` snaps `this.visibility` to `from`, otherwise
   * to `to`.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.setVisibilityBlend(
   *   { mode: "masked", visible: [], explored: [], perceived: [] },
   *   { mode: "masked", visible: [], explored: [], perceived: [] },
   *   0.5,
   * );
   * backend.visibilityBlend?.factor; // 0.5
   * ```
   */
  setVisibilityBlend(from: VisibilityInput, to: VisibilityInput, factor: number): void {
    this.visibilityBlend = { from, to, factor };
    this.visibility = factor < 0.5 ? from : to;
  }
  /** `DisplayBackend.addLayerFilter`: records `{layerId, filter}` into `this.filters`
   * unconditionally. **Divergence from `PixiBackend` (mock LOOSER):** this never validates
   * `layerId` — an unknown id is recorded just like a real one, whereas `PixiBackend.addLayerFilter`
   * silently no-ops (returns a no-op dispose, never touching `this.filters`-equivalent state) for a
   * `layerId` that names no layer Container. A test asserting filter registration against an
   * invalid layer id would pass here and fail against the real backend. **Divergence #2
   * (mock STRICTER, the opposite direction):** dispose here removes only THIS registration's own
   * `entry` object (by identity, via `indexOf`), whereas `PixiBackend.addLayerFilter`'s dispose
   * removes every filter-list entry `=== filter` — registering the same filter value twice on one
   * layer and disposing either one strips both on the real backend, but only the disposed one
   * here. A test asserting "disposing one registration leaves a duplicate-value registration
   * intact" would pass against this mock and fail against the real backend.
   * @param layerId The target core-layer id (recorded as given, not validated).
   * @param filter An opaque filter value.
   * @returns A dispose function that removes exactly this registration's entry from
   * `this.filters`, by object identity — see the stricter-than-`PixiBackend` note above.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * const dispose = backend.addLayerFilter("tokens", { kind: "blur" });
   * dispose();
   * ```
   */
  addLayerFilter(layerId: string, filter: unknown): () => void {
    const entry = { layerId, filter };
    this.filters.push(entry);
    return () => {
      const i = this.filters.indexOf(entry);
      if (i >= 0) this.filters.splice(i, 1);
    };
  }
  /** `DisplayBackend.setToken`: upserts `spec` verbatim into `this.tokens`, keyed by `id`. Unlike
   * `PixiBackend.setToken`, this does not simulate texture loading, rotation, badge diffing, or the
   * perceived-flag re-parenting — `this.tokens.get(id)` always reflects the LAST spec passed,
   * immediately, including its `perceived` flag.
   * @param id The token document id.
   * @param spec The resolved token render spec to record.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.setToken("00000000-0000-0000-0000-000000000001", {
   *   x: 0, y: 0, w: 70, h: 70, rotation: 0,
   *   visual: { kind: "image", url: "https://example.test/token.png" },
   *   borderColor: null, badges: [], shape: "square", perceived: false,
   * });
   * ```
   */
  setToken(id: string, spec: TokenNodeSpec): void {
    this.tokens.set(id, spec);
  }
  /** `DisplayBackend.removeToken`: deletes `id` from `this.tokens`. A no-op for an unknown `id`.
   * @param id The token document id to remove.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.removeToken("00000000-0000-0000-0000-000000000001");
   * ```
   */
  removeToken(id: string): void {
    this.tokens.delete(id);
  }
  /** `DisplayBackend.tickTokenAnimations`: an intentional no-op. `MockBackend` records
   * `TokenNodeSpec.visual` verbatim (via `setToken`); animated-frame-advance state (the current
   * frame index, elapsed time) is owned entirely by `PixiBackend`'s real `AnimatedSprite` objects,
   * which this mock never creates, so there is nothing here to advance.
   * @param _dtMs Milliseconds elapsed since the previous tick (unused).
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.tickTokenAnimations(16); // no-op
   * ```
   */
  tickTokenAnimations(_dtMs: number): void {
    // MockBackend records TokenNodeSpec.visual verbatim; frame-advance is real-AnimatedSprite
    // state owned by PixiBackend only, so this is an intentional no-op in tests.
  }
  /** `DisplayBackend.setShape`: upserts `spec` verbatim into `this.shapes`, keyed by `id`.
   * @param id The shape document id.
   * @param spec The resolved shape spec to record.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.setShape("00000000-0000-0000-0000-000000000001", {
   *   layer: "drawings", points: [0, 0, 10, 0, 10, 10, 0, 10], closed: true,
   *   stroke: null, fill: null,
   * });
   * ```
   */
  setShape(id: string, spec: ShapeNodeSpec): void {
    this.shapes.set(id, spec);
  }
  /** `DisplayBackend.removeShape`: deletes `id` from `this.shapes`. A no-op for an unknown `id`.
   * @param id The shape document id to remove.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.removeShape("00000000-0000-0000-0000-000000000001");
   * ```
   */
  removeShape(id: string): void {
    this.shapes.delete(id);
  }
  /** `DisplayBackend.drawOverlay`: replaces `this.overlay` with `shapes` verbatim (a reference
   * assignment, not a paint-and-accumulate like `PixiBackend`'s shared-Graphics implementation).
   * @param shapes The preview shapes to record.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.drawOverlay([{ points: [0, 0, 10, 0, 10, 10, 0, 10], closed: true, stroke: null, fill: null }]);
   * ```
   */
  drawOverlay(shapes: Omit<ShapeNodeSpec, "layer">[]): void {
    this.overlay = shapes;
  }
  /** `DisplayBackend.clearOverlay`: resets `this.overlay` to an empty array.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.clearOverlay();
   * ```
   */
  clearOverlay(): void {
    this.overlay = [];
  }
  /** `DisplayBackend.drawMeasure`: records `{from, to, label}` verbatim into `this.measure`.
   * @param from The segment's start point (scene coords).
   * @param to The segment's end point (scene coords).
   * @param label The distance label text.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.drawMeasure({ x: 0, y: 0 }, { x: 10, y: 0 }, "2");
   * ```
   */
  drawMeasure(from: Point, to: Point, label: string): void {
    this.measure = { from, to, label };
  }
  /** `DisplayBackend.clearMeasure`: resets `this.measure` to `null`.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.clearMeasure();
   * ```
   */
  clearMeasure(): void {
    this.measure = null;
  }
  /** `DisplayBackend.drawPings`: records `rings` verbatim into `this.pings`.
   * @param rings The current ping rings to record.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.drawPings([{ x: 0, y: 0, radius: 20, alpha: 0.8 }]);
   * ```
   */
  drawPings(rings: PingRing[]): void {
    this.pings = rings;
  }
  /** `DisplayBackend.setLighting`: records `frame` verbatim into `this.lighting`.
   * @param frame The resolved per-cell lighting to record.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.setLighting({ cell: 70, cells: [] });
   * ```
   */
  setLighting(frame: LightingFrame): void {
    this.lighting = frame;
  }
  /** `DisplayBackend.startTicker`: records `cb` into `this.tick` for manual driving. **Divergence
   * from `PixiBackend`:** the real backend hooks Pixi's own `app.ticker` and calls `cb` on every
   * render frame automatically; this mock never calls `cb` on its own — a test must explicitly
   * invoke it, either directly (`backend.tick!(dtMs)`) or via the {@link runTicker} helper.
   * **Second divergence, on a REPEAT call:** here, a second `startTicker` call silently OVERWRITES
   * `this.tick` — only the latest registration ever fires. `PixiBackend.startTicker` instead
   * ACCUMULATES: every call adds a new, never-removed listener to `app.ticker`, so on the real
   * backend a second call means BOTH callbacks fire every frame, not just the latest. A test
   * asserting "the first-registered callback no longer fires after a second `startTicker` call"
   * would pass against this mock and fail against the real backend.
   * @param cb The per-frame render callback to record.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.startTicker((dtMs) => {});
   * backend.runTicker(16); // manually drive one frame
   * ```
   */
  startTicker(cb: (dtMs: number) => void): void {
    this.tick = cb;
  }
  /** `DisplayBackend.resize`: records `{width, height}` verbatim into `this.size`.
   * @param width The new viewport width, in CSS pixels.
   * @param height The new viewport height, in CSS pixels.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.resize(1280, 720);
   * ```
   */
  resize(width: number, height: number): void {
    this.size = { width, height };
  }
  /** `DisplayBackend.destroy`: sets `this.destroyed = true`. Does not release any resources (this
   * mock never allocates GPU state) — a test asserts teardown was requested by reading
   * `this.destroyed`. **Divergence from `PixiBackend` (mock LOOSER, sharpest divergence in this
   * pair — crash vs. silent success):** this is idempotent — a second call just re-sets
   * `this.destroyed = true` with no error — and it does NOT clear `this.tick`/`this.tokens`/any
   * other recorded state, so a stale `runTicker()` or `lastTokenX()` call after `destroy()` still
   * works here. `PixiBackend.destroy()` is single-use: its second call THROWS (Pixi's
   * `Application.destroy` nulls `stage`/`renderer`, and the second call dereferences the null). A
   * test double-destroying a backend, or calling a mock method after `destroy()`, passes green
   * against this mock and would throw against the real backend.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.destroy();
   * backend.destroyed; // true
   * ```
   */
  destroy(): void {
    this.destroyed = true;
  }

  /** Test helper: drive the ticker by `ms` milliseconds in one shot — calls the callback recorded
   * by `startTicker`. Throws (non-null assertion) if called before `startTicker`.
   * @param ms Milliseconds to advance in this one call.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.startTicker((dtMs) => {});
   * backend.runTicker(16);
   * ```
   */
  runTicker(ms: number): void {
    this.tick!(ms);
  }

  /** Test helper: read the current recorded x of a token. Throws (non-null assertion) if `id`
   * has never been passed to `setToken`.
   * @param id The token document id.
   * @returns The token's last-recorded `x` coordinate.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.setToken("00000000-0000-0000-0000-000000000001", {
   *   x: 42, y: 0, w: 70, h: 70, rotation: 0,
   *   visual: { kind: "image", url: "https://example.test/token.png" },
   *   borderColor: null, badges: [], shape: "square", perceived: false,
   * });
   * backend.lastTokenX("00000000-0000-0000-0000-000000000001"); // 42
   * ```
   */
  lastTokenX(id: string): number {
    return this.tokens.get(id)!.x;
  }
}
