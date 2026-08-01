// In-memory EngineAdapter implementation: plain divs per zone/group, honoring
// `apply`'s reconcile contract via real `appendChild` adoption. Doubles as the
// bespoke-fallback engine (a real, if minimal, engine — not a test mock) and
// as the test double for PanelHost: `emitOp` simulates a user gesture through
// the same `onOp` channel a real engine uses; `groupEl`/`floatEl` expose the
// adopted DOM for adoption assertions.
import type { PanelMeta } from "@shadowcat/core";
import type { ExpandedLayout, LayoutOp } from "../layout/tree";
import type { EngineAdapter } from "./adapter";

const ZONE_IDS = ["right", "bottom", "left"] as const;
type Zone = (typeof ZONE_IDS)[number];

/** In-memory `EngineAdapter`: plain divs per zone/group/floating panel,
 * reconciled by `apply` via real `appendChild` adoption — no dockview
 * dependency. Serves two roles: the test double for `PanelHost`'s own tests
 * (`emitOp` simulates a user gesture through the same `onOp` channel a real
 * engine uses), and the fallback engine `PanelHost` mounts by default when
 * constructed WITHOUT an `engine` prop (`PanelHost.svelte`'s own `engine ??
 * new FakeEngine()`) — a seam any bespoke host could rely on, but no
 * production path in THIS codebase reaches it: the shipped `panels` module's
 * `register()` always supplies a `DockviewEngine` instance (`index.ts`), so
 * that default branch is taken only by a caller that mounts `PanelHost`
 * directly without an `engine` prop — today, only the test suite does.
 * @example
 * ```ts
 * import { FakeEngine } from "@shadowcat/module-panels";
 *
 * const engine = new FakeEngine();
 * ```
 */
export class FakeEngine implements EngineAdapter {
  #host: HTMLElement | null = null;
  #slotFor: ((id: string) => HTMLElement) | null = null;
  #zoneEls = new Map<Zone, HTMLElement>();
  #groupEls = new Map<string, HTMLElement>(); // key: `${zone}:${index}`
  #floatEls = new Map<string, HTMLElement>(); // key: panel id
  #opListeners = new Set<(op: LayoutOp) => void>();
  #focused: string | null = null;
  #stageEl: HTMLElement | null = null;
  #centerEl: HTMLElement | null = null;

  /** `EngineAdapter.init`: builds this engine's own zone/center-well DOM
   * inside `host` (left/right side columns flanking the center well, `bottom`
   * full-width below) and adopts `stageEl` into the center well. See the
   * inline comments below for why `host` and the center well each need an
   * explicit flex size chain, and why the row/column nesting exists.
   * @param host The container this engine builds its zone DOM into.
   * @param slotFor Resolves a panel id to its persistent, already-mounted slot
   * element — adopted, never re-created, by `apply`.
   * @param stageEl The shared canvas/stage element, adopted into the center well.
   * @example
   * ```ts
   * import { FakeEngine } from "@shadowcat/module-panels";
   *
   * const engine = new FakeEngine();
   * declare const host: HTMLElement;
   * declare const stageEl: HTMLElement;
   * declare const slots: Map<string, HTMLElement>;
   * engine.init(host, (id) => slots.get(id)!, stageEl);
   * ```
   */
  init(host: HTMLElement, slotFor: (id: string) => HTMLElement, stageEl: HTMLElement): void {
    this.#host = host;
    this.#slotFor = slotFor;
    // Establishes a definite size chain for the DOM this bespoke-fallback
    // engine owns (buddy-check finding 2): `host` is a bare container with no
    // layout of its own, so without a flex context here `centerEl` resolves
    // to `height: auto` and the adopted `.stage` (`height: 100%`, PanelHost.
    // svelte) collapses to its content height. Inline styles are used
    // because no external stylesheet can target JS-created engine internals
    // reliably.
    host.style.display = "flex";
    host.style.flexDirection = "column";
    host.style.height = "100%";
    host.style.minHeight = "0";

    // `row` places "left"/"right" as side columns flanking the center well
    // (matching a real docked layout's geometry); "bottom" stacks below the
    // row at full width. Without this nesting, `host`'s own column flow made
    // every zone a full-width block sibling of the center well — the "loses
    // width containment" defect (docs/CLOSED_BUGS.md, resolved): a zone `<div>` with no
    // width of its own, inside a column flex container, stretches to the
    // container's full cross-size (`align-items: stretch`, the flex default)
    // regardless of how many groups are docked into it.
    const row = document.createElement("div");
    row.style.display = "flex";
    row.style.flexDirection = "row";
    row.style.flex = "1";
    row.style.minWidth = "0";
    row.style.minHeight = "0";
    host.appendChild(row);

    // Adopts the shared stage/canvas element into a dedicated center-well
    // container — faithful to the adapter contract's reserved-layout-space
    // semantics (real engines lay dock zones out around this well).
    this.#stageEl = stageEl;
    const centerEl = document.createElement("div");
    centerEl.dataset.fakeCenter = "";
    centerEl.style.flex = "1";
    centerEl.style.minWidth = "0";
    centerEl.style.minHeight = "0";
    centerEl.appendChild(stageEl);
    this.#centerEl = centerEl;

    const leftEl = this.#makeZoneEl("left");
    row.appendChild(leftEl);
    row.appendChild(centerEl);
    const rightEl = this.#makeZoneEl("right");
    row.appendChild(rightEl);
    const bottomEl = this.#makeZoneEl("bottom");
    host.appendChild(bottomEl);
  }

  /** Builds a zone container with the containment properties `apply` relies
   * on: a fixed flex-basis (never stretched by the row/column flex context),
   * `min-width: 0` so its own intrinsic content can never force it wider than
   * that basis, and `overflow: auto` so oversized group content scrolls
   * WITHIN the zone rather than escaping it. `apply` sets the actual px
   * cross-size (width for right/left, height for bottom) from
   * `ZoneNode.size` on every reconcile — 0 while the zone has no groups, so
   * an empty zone reserves no layout space.
   * @param zone The zone this container will host.
   * @returns The newly-created (and tracked in `#zoneEls`) zone container.
   * @example
   * ```
   * // private method; not part of the public API — invoked only from init()
   * this.#makeZoneEl("right");
   * ```
   */
  #makeZoneEl(zone: Zone): HTMLElement {
    const el = document.createElement("div");
    el.dataset.zone = zone;
    el.style.display = "flex";
    el.style.flexDirection = "column";
    el.style.flex = "0 0 auto";
    el.style.minWidth = "0";
    el.style.minHeight = "0";
    el.style.overflow = "auto";
    this.#zoneEls.set(zone, el);
    return el;
  }

  /** `EngineAdapter.apply`: full-rebuild reconcile (never a diff) — every
   * zone's group wrapper divs are torn down and rebuilt fresh from `expanded`
   * on every call, then every floating entry (including `poppedOut` ids,
   * degraded to floating here — see below) is placed or repositioned.
   * Idempotent per the `EngineAdapter` contract: repeat calls with the same
   * `expanded` rebuild the identical DOM shape each time, a no-op in effect
   * even though the elements themselves are recreated. `meta` is unused —
   * this bespoke-fallback engine renders no tab chrome (icon/label/badge) of
   * its own, unlike `DockviewEngine`'s `PanelTabRenderer`.
   * @param expanded The docked-zone + floating layout to reconcile onto.
   * @param _meta Per-panel metadata; accepted for `EngineAdapter` parity but
   * not read by this engine.
   * @example
   * ```ts
   * import { FakeEngine, type ExpandedLayout } from "@shadowcat/module-panels";
   * import type { PanelMeta } from "@shadowcat/core";
   *
   * const engine = new FakeEngine();
   * declare const expanded: ExpandedLayout;
   * declare const meta: ReadonlyMap<string, PanelMeta>;
   * engine.apply(expanded, meta);
   * ```
   */
  apply(expanded: ExpandedLayout, _meta: ReadonlyMap<string, PanelMeta>): void {
    const host = this.#host;
    const slotFor = this.#slotFor;
    if (!host || !slotFor) return;

    // Zones: rebuild each zone's group wrapper divs fresh, then re-adopt every
    // tab's persistent slot element — all tabs in a group are adopted, only
    // the active one is shown (display), matching a docked tab strip.
    for (const zone of ZONE_IDS) {
      const zoneEl = this.#zoneEls.get(zone);
      if (!zoneEl) continue;
      zoneEl.replaceChildren();
      for (const key of [...this.#groupEls.keys()]) {
        if (key.startsWith(`${zone}:`)) this.#groupEls.delete(key);
      }
      const zoneNode = expanded.zones[zone];
      // Cross-size (width for right/left, height for bottom) from the
      // reducer's own `ZoneNode.size` px basis; 0 while empty so a docked-
      // into-elsewhere zone reserves no layout space. Re-applied on every
      // `apply()` so a later `resizeZone` op (or a group count going 0->1)
      // is reflected without a separate code path.
      const crossSize = zoneNode.groups.length > 0 ? zoneNode.size : 0;
      if (zone === "bottom") {
        zoneEl.style.width = "100%";
        zoneEl.style.height = `${crossSize}px`;
      } else {
        zoneEl.style.width = `${crossSize}px`;
        zoneEl.style.height = "";
      }
      zoneNode.groups.forEach((group, i) => {
        const groupEl = document.createElement("div");
        groupEl.dataset.group = String(i);
        groupEl.style.width = "100%";
        groupEl.style.minWidth = "0";
        zoneEl.appendChild(groupEl);
        this.#groupEls.set(`${zone}:${i}`, groupEl);
        for (const id of group.tabs) {
          const slot = slotFor(id);
          groupEl.appendChild(slot);
          slot.style.display = group.active === id ? "" : "none";
        }
      });
    }

    // Floating: one container per floating panel, adopted directly and
    // positioned from its `Rect`. Popped-out ids are degraded to floating here
    // (this bespoke-fallback engine has no cross-window popout; spec §10) so a
    // slot is never lost and the keep-mounted invariant holds — production
    // pop-out is dockview-only.
    const POPOUT_FALLBACK_BASE = { x: 96, y: 96, w: 420, h: 520 };
    const POPOUT_FALLBACK_STEP = 28;
    // Cascades each fallback rect off its index (mirrors the cascade formula
    // at the other degraded/rehydrated-position sites in this checkpoint —
    // tree.ts's SHEET_CASCADE_BASE/STEP, placeFromPersistedLocation's
    // "popped-out" case, controller.svelte.ts's REHYDRATE_FLOAT_BASE/STEP) so
    // two-or-more simultaneously-popped-out ids don't render fully
    // overlapping at the identical position under this bespoke-fallback engine.
    const maxZ = expanded.floating.reduce((m, f) => Math.max(m, f.z), -1);
    const floatEntries = [
      ...expanded.floating,
      ...expanded.poppedOut.map((id, i) => {
        const off = (i % 6) * POPOUT_FALLBACK_STEP;
        return {
          id,
          rect: {
            x: POPOUT_FALLBACK_BASE.x + off,
            y: POPOUT_FALLBACK_BASE.y + off,
            w: POPOUT_FALLBACK_BASE.w,
            h: POPOUT_FALLBACK_BASE.h,
          },
          z: maxZ + 1 + i,
        };
      }),
    ];
    const floatIds = new Set(floatEntries.map((f) => f.id));
    for (const [id, el] of [...this.#floatEls]) {
      if (!floatIds.has(id)) {
        el.remove();
        this.#floatEls.delete(id);
      }
    }
    for (const f of floatEntries) {
      let el = this.#floatEls.get(f.id);
      if (!el) {
        el = document.createElement("div");
        el.dataset.floating = f.id;
        host.appendChild(el);
        this.#floatEls.set(f.id, el);
      }
      el.style.left = `${f.rect.x}px`;
      el.style.top = `${f.rect.y}px`;
      el.style.width = `${f.rect.w}px`;
      el.style.height = `${f.rect.h}px`;
      el.style.zIndex = String(f.z);
      const slot = slotFor(f.id);
      el.appendChild(slot);
      slot.style.display = "";
    }
  }

  /** `EngineAdapter.onOp`: subscribes to ops this engine emits. The only
   * emission source is `emitOp` — this engine has no real drag/resize
   * gestures of its own to translate (see the class doc comment for which
   * callers actually reach this engine).
   * @param cb Called once per emitted op.
   * @returns An unsubscribe function.
   * @example
   * ```ts
   * import { FakeEngine, type LayoutOp } from "@shadowcat/module-panels";
   *
   * const engine = new FakeEngine();
   * const unsubscribe = engine.onOp((op: LayoutOp) => console.log(op));
   * unsubscribe();
   * ```
   */
  onOp(cb: (op: LayoutOp) => void): () => void {
    this.#opListeners.add(cb);
    return () => this.#opListeners.delete(cb);
  }

  /** Test/bespoke-fallback helper: simulates a user gesture normalized to a
   * `LayoutOp`, exactly as a real engine would emit through `onOp`.
   * @param op The op to emit to every `onOp` subscriber.
   * @example
   * ```ts
   * import { FakeEngine } from "@shadowcat/module-panels";
   *
   * const engine = new FakeEngine();
   * engine.emitOp({ op: "minimize", id: "chat" });
   * ```
   */
  emitOp(op: LayoutOp): void {
    for (const cb of this.#opListeners) cb(op);
  }

  /** `EngineAdapter.focus`: records `id` as the last-focused panel — read
   * back via the `focused` test getter. No visible DOM effect (this
   * bespoke-fallback engine renders no focus chrome of its own). Unlike
   * `DockviewEngine.focus`, this has no `STAGE_ID` guard — a sibling
   * divergence with no observed production impact today, since no caller in
   * this package invokes `EngineAdapter.focus` at all (see `PanelsController.
   * focus`'s doc comment).
   * @param id The panel id to focus.
   * @example
   * ```ts
   * import { FakeEngine } from "@shadowcat/module-panels";
   *
   * const engine = new FakeEngine();
   * engine.focus("chat");
   * ```
   */
  focus(id: string): void {
    this.#focused = id;
  }

  /** Test helper: the last id passed to `focus`.
   * @returns The last-focused panel id, or `null` if `focus` was never called.
   */
  get focused(): string | null {
    return this.#focused;
  }

  /** Test helper: the DOM element hosting a given zone/group's tabs (or null).
   * @param zone The zone the group lives in.
   * @param index The group's positional index within `zone`.
   * @returns The group's container element, or `null` if no such group exists.
   * @example
   * ```ts
   * import { FakeEngine } from "@shadowcat/module-panels";
   *
   * const engine = new FakeEngine();
   * const el = engine.groupEl("right", 0);
   * ```
   */
  groupEl(zone: Zone, index: number): HTMLElement | null {
    return this.#groupEls.get(`${zone}:${index}`) ?? null;
  }

  /** Test helper: the zone container itself (or null before `init`).
   * @param zone The zone to look up.
   * @returns The zone's own container element, or `null` before `init` has run.
   * @example
   * ```ts
   * import { FakeEngine } from "@shadowcat/module-panels";
   *
   * const engine = new FakeEngine();
   * const el = engine.zoneEl("right");
   * ```
   */
  zoneEl(zone: Zone): HTMLElement | null {
    return this.#zoneEls.get(zone) ?? null;
  }

  /** Test helper: the DOM element hosting a floating panel (or null).
   * @param id The floating (or popped-out — see `apply`'s degradation) panel id.
   * @returns The floating container element, or `null` if `id` is not floating.
   * @example
   * ```ts
   * import { FakeEngine } from "@shadowcat/module-panels";
   *
   * const engine = new FakeEngine();
   * const el = engine.floatEl("chat");
   * ```
   */
  floatEl(id: string): HTMLElement | null {
    return this.#floatEls.get(id) ?? null;
  }

  /** Test helper: the center-well container the `stageEl` passed to `init` was adopted into.
   * @returns The center-well container, or `null` before `init` has run.
   * @example
   * ```ts
   * import { FakeEngine } from "@shadowcat/module-panels";
   *
   * const engine = new FakeEngine();
   * const el = engine.centerEl();
   * ```
   */
  centerEl(): HTMLElement | null {
    return this.#centerEl;
  }

  /** Test helper: the `stageEl` passed to `init` (or null before init/after destroy).
   * @returns The stage element, or `null` before `init`/after `destroy`.
   */
  get stageEl(): HTMLElement | null {
    return this.#stageEl;
  }

  /** `EngineAdapter.destroy`: clears every internal tracking map/listener set
   * and drops this engine's element references. Does NOT remove the zone/
   * group/floating container elements it built inside `host` from the DOM —
   * the caller (`PanelHost`, on its own unmount) discards `host` itself, so
   * there is nothing left for this method to detach on its own. Slot elements
   * are untouched either way, per the `EngineAdapter` contract.
   * @example
   * ```ts
   * import { FakeEngine } from "@shadowcat/module-panels";
   *
   * const engine = new FakeEngine();
   * engine.destroy();
   * ```
   */
  destroy(): void {
    this.#zoneEls.clear();
    this.#groupEls.clear();
    this.#floatEls.clear();
    this.#opListeners.clear();
    this.#host = null;
    this.#slotFor = null;
    this.#stageEl = null;
    this.#centerEl = null;
  }
}
