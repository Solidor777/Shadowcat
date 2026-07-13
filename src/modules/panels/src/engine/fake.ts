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

  init(host: HTMLElement, slotFor: (id: string) => HTMLElement): void {
    this.#host = host;
    this.#slotFor = slotFor;
    for (const zone of ZONE_IDS) {
      const el = document.createElement("div");
      el.dataset.zone = zone;
      host.appendChild(el);
      this.#zoneEls.set(zone, el);
    }
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
      expanded.zones[zone].groups.forEach((group, i) => {
        const groupEl = document.createElement("div");
        groupEl.dataset.group = String(i);
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
    // positioned from its `Rect`.
    const floatIds = new Set(expanded.floating.map((f) => f.id));
    for (const [id, el] of [...this.#floatEls]) {
      if (!floatIds.has(id)) {
        el.remove();
        this.#floatEls.delete(id);
      }
    }
    for (const f of expanded.floating) {
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

  /** Test helper: the DOM element hosting a floating panel (or null). */
  floatEl(id: string): HTMLElement | null {
    return this.#floatEls.get(id) ?? null;
  }

  destroy(): void {
    this.#zoneEls.clear();
    this.#groupEls.clear();
    this.#floatEls.clear();
    this.#opListeners.clear();
    this.#host = null;
    this.#slotFor = null;
  }
}
