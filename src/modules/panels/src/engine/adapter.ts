// The narrow seam a real docking engine implements to reconcile the
// panel-manager's layout tree onto its own widget tree. PanelHost owns layout
// state and slot elements; every method here is imperative DOM/state
// reconciliation — no engine implementation holds layout truth of its own,
// and none of them may destroy a slot element (ownership of panel components
// stays with PanelHost's staging container for the component's entire
// lifetime).
import type { PanelMeta } from "@shadowcat/core";
import type { ExpandedLayout, LayoutOp } from "../layout/tree";

/** The seam a docking engine implements to reconcile the panel-manager's layout tree onto
 * its own widget tree. This is the statement of record for what each member guarantees —
 * `DockviewEngine`/`FakeEngine` members document only what diverges from, or is not covered
 * by, the contract stated here. */
export interface EngineAdapter {
  /** Called once at mount.
   * @param host The container element the engine may build its own widget tree inside.
   * @param slotFor Resolves a panel id to its persistent, already-mounted slot element
   * (owned by PanelHost's staging container) — the engine adopts it via `appendChild`,
   * never re-creates or destroys it.
   * @param stageEl The canvas/stage element some real engines reserve layout space around;
   * opaque to this adapter, passed through as-is.
   */
  init(host: HTMLElement, slotFor: (id: string) => HTMLElement, stageEl: HTMLElement): void;
  /** Reconciles the engine's widget tree to match `expanded` (docked zones + floating
   * windows). Called whenever the layout changes while the host is in expanded
   * presentation. Idempotent — repeat calls with the same input must be safe (a host may
   * call this defensively on every layout change).
   * @param expanded The layout to reconcile the engine's widget tree to.
   * @param meta Per-panel display metadata (title, icon) keyed by panel id.
   */
  apply(expanded: ExpandedLayout, meta: ReadonlyMap<string, PanelMeta>): void;
  /** Subscribes to user gestures (drag/resize/close/float/dock) normalized to `LayoutOp`s
   * the host reduces onto its layout tree.
   * @param cb Called with each classified `LayoutOp` as gestures occur.
   * @returns An unsubscribe function.
   */
  onOp(cb: (op: LayoutOp) => void): () => void;
  /** Subscribes to user-facing engine notices — a stable i18n key the host resolves +
   * surfaces (live region / toast). Optional: engines with no notice source (`FakeEngine`)
   * omit it.
   * @param cb Called with each notice's i18n key as it occurs.
   * @returns An unsubscribe function.
   */
  onNotice?(cb: (key: string) => void): () => void;
  /** Brings a panel's group/window to the foreground.
   * @param id The panel id to focus.
   */
  focus(id: string): void;
  /** Releases all DOM/listeners the engine created. Slot elements themselves are NOT
   * destroyed — ownership returns to PanelHost's staging container.
   */
  destroy(): void;
}
