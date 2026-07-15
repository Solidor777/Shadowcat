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
    // width containment" defect (docs/OPEN_BUGS.md): a zone `<div>` with no
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
   * an empty zone reserves no layout space. */
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

  onOp(cb: (op: LayoutOp) => void): () => void {
    this.#opListeners.add(cb);
    return () => this.#opListeners.delete(cb);
  }

  /** Test/bespoke-fallback helper: simulates a user gesture normalized to a
   * `LayoutOp`, exactly as a real engine would emit through `onOp`. */
  emitOp(op: LayoutOp): void {
    for (const cb of this.#opListeners) cb(op);
  }

  focus(id: string): void {
    this.#focused = id;
  }

  /** Test helper: the last id passed to `focus`. */
  get focused(): string | null {
    return this.#focused;
  }

  /** Test helper: the DOM element hosting a given zone/group's tabs (or null). */
  groupEl(zone: Zone, index: number): HTMLElement | null {
    return this.#groupEls.get(`${zone}:${index}`) ?? null;
  }

  /** Test helper: the zone container itself (or null before `init`). */
  zoneEl(zone: Zone): HTMLElement | null {
    return this.#zoneEls.get(zone) ?? null;
  }

  /** Test helper: the DOM element hosting a floating panel (or null). */
  floatEl(id: string): HTMLElement | null {
    return this.#floatEls.get(id) ?? null;
  }

  /** Test helper: the center-well container the `stageEl` passed to `init` was adopted into. */
  centerEl(): HTMLElement | null {
    return this.#centerEl;
  }

  /** Test helper: the `stageEl` passed to `init` (or null before init/after destroy). */
  get stageEl(): HTMLElement | null {
    return this.#stageEl;
  }

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
