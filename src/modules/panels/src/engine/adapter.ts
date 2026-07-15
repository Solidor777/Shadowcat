// The narrow seam a real docking engine implements to reconcile the
// panel-manager's layout tree onto its own widget tree. PanelHost owns layout
// state and slot elements; every method here is imperative DOM/state
// reconciliation — no engine implementation holds layout truth of its own,
// and none of them may destroy a slot element (ownership of panel components
// stays with PanelHost's staging container for the component's entire
// lifetime).
import type { PanelMeta } from "@shadowcat/core";
import type { ExpandedLayout, LayoutOp } from "../layout/tree";

export interface EngineAdapter {
  /** Called once at mount. `slotFor(id)` resolves a panel id to its persistent,
   * already-mounted slot element (owned by PanelHost's staging container) —
   * the engine adopts it via `appendChild`, never re-creates or destroys it.
   * `stageEl` is the canvas/stage element some real engines reserve layout
   * space around (opaque to this adapter; passed through as-is). */
  init(host: HTMLElement, slotFor: (id: string) => HTMLElement, stageEl: HTMLElement): void;
  /** Reconciles the engine's widget tree to match `expanded` (docked zones +
   * floating windows). Called whenever the layout changes while the host is
   * in expanded presentation. Idempotent — repeat calls with the same input
   * must be safe (a host may call this defensively on every layout change). */
  apply(expanded: ExpandedLayout, meta: ReadonlyMap<string, PanelMeta>): void;
  /** Subscribes to user gestures (drag/resize/close/float/dock) normalized to
   * `LayoutOp`s the host reduces onto its layout tree; returns an unsubscribe. */
  onOp(cb: (op: LayoutOp) => void): () => void;
  /** Subscribes to user-facing engine notices (spec §10) — a stable i18n key
   * the host resolves + surfaces (live region / toast). Optional: engines with
   * no notice source (`FakeEngine`) omit it. Returns an unsubscribe. */
  onNotice?(cb: (key: string) => void): () => void;
  /** Brings a panel's group/window to the foreground. */
  focus(id: string): void;
  /** Releases all DOM/listeners the engine created. Slot elements themselves
   * are NOT destroyed — ownership returns to PanelHost's staging container. */
  destroy(): void;
}
