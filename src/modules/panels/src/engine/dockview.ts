// The ONLY file in this codebase permitted to import `dockview-core`. Every
// dockview type/event crosses out of this module already translated into our
// own vocabulary (`LayoutOp`, `DropSite`) — the `policy` module and everything above
// the `EngineAdapter` seam stays engine-free.
import { createDockview } from "dockview-core";
import type {
  CreateComponentOptions,
  DockviewApi,
  DockviewActivePanelChangeEvent,
  DockviewWillDropEvent,
  IDockviewGroupPanel,
  IContentRenderer,
  IDockviewPanel,
  ITabRenderer,
  TabPartInitParameters,
} from "dockview-core";
import { mount, unmount } from "svelte";
import { consoleLogger, type Logger, type PanelMeta, type ZoneId } from "@shadowcat/core";
import { i18n } from "@shadowcat/ui-kit";
import type { EngineAdapter } from "./adapter";
import type { ExpandedLayout, LayoutOp, Rect } from "../layout/tree";
import { classifyDrop, opForMenuCommand, MENU_FLOAT_RECT, STAGE_ID, type DropSite, type MenuCommand } from "./policy";
import PanelMenu from "../PanelMenu.svelte";
// Minimal wiring: imports dockview's base stylesheet + our token overrides —
// only paid for when a real docking engine is actually instantiated, never
// by `FakeEngine`-only hosts.
import "../panels.scss";

/** The dockview group id reserved for the stage. Never collides with a
 * zone-group id, which is always derived from a panel id via `groupIdFor`
 * and namespaced under a different prefix. */
const STAGE_GROUP_ID = "sc-stage-group";

/** The three dock zones this engine reconciles, in the fixed order `apply()`
 * walks them — an iteration order, not a priority; every zone is always
 * present (see `ExpandedLayout`'s own doc comment). */
const ZONE_IDS: readonly ZoneId[] = ["right", "bottom", "left"];

/** The edge direction `addGroup` splits off the stage group in, for the
 * FIRST group created in each zone. Every later group in the same zone
 * stacks below the previous one instead (see `apply`). */
const ZONE_EDGE_DIRECTION: Record<ZoneId, "left" | "right" | "below"> = {
  right: "right",
  left: "left",
  bottom: "below",
};

/** Content renderer that adopts an externally-owned element (a panel slot,
 * or the shared stage element) into dockview's panel content container.
 * `resolve` is called once, lazily, at `init()` — matching every other
 * engine's adoption timing (never before the renderer is asked to render).
 * `dispose` detaches rather than destroys: ownership of the adopted element
 * returns to its original owner (PanelHost's staging container, or — for
 * the stage — the adapter's own cached reference), per the `EngineAdapter`
 * contract. */
class AdoptingContentRenderer implements IContentRenderer {
  /** This renderer's own wrapper element, adopted into dockview's panel content container. */
  readonly element: HTMLElement;
  /** The externally-owned element adopted into `element` by `init`; `null` before `init`
   * runs or after `dispose`. */
  #adopted: HTMLElement | null = null;

  /** Stashes `resolve` for `init()` to call (never invoked here — see the class
   * doc comment) and builds this renderer's own wrapper `element`, tagged with
   * `className` so `panels.scss` can style the stage's content container
   * distinctly from an ordinary panel's.
   * @param resolve Lazily resolves the element to adopt — a panel slot, or the
   * shared stage element. Called exactly once, from `init()`.
   * @param className CSS class applied to this renderer's own wrapper element
   * (`sc-dockview-panel-content` or `sc-dockview-stage-content`).
   * @example
   * ```
   * // private class; not part of the public API — constructed only by
   * // DockviewEngine's own `createComponent` factory
   * declare const stageEl: HTMLElement;
   * new AdoptingContentRenderer(() => stageEl, "sc-dockview-stage-content");
   * ```
   */
  constructor(
    private readonly resolve: () => HTMLElement,
    className: string,
  ) {
    this.element = document.createElement("div");
    this.element.className = className;
    this.element.style.height = "100%";
  }

  /** Dockview's first (and only meaningful) render call for this content: pulls
   * the element `resolve()` returns and adopts it via `appendChild`. Guarded by
   * `#adopted` so a defensive repeat call is a no-op — dockview does not itself
   * call `init()` twice for the same renderer.
   * @example
   * ```
   * // private method; not part of the public API — invoked only by
   * // dockview-core's own render lifecycle (IContentRenderer.init)
   * declare const renderer: AdoptingContentRenderer;
   * renderer.init();
   * ```
   */
  init(): void {
    if (this.#adopted) return;
    this.#adopted = this.resolve();
    this.element.appendChild(this.#adopted);
  }

  /** No-op: the adopted element's own CSS (`height: 100%` on `element`, set at
   * construction) sizes it, not dockview's grid layout pass.
   * @example
   * ```
   * // private method; not part of the public API
   * declare const renderer: AdoptingContentRenderer;
   * renderer.layout();
   * ```
   */
  layout(): void {}

  /** No-op: this renderer forwards no per-update params of its own — the
   * adopted element's content is owned and re-rendered by whatever mounted it,
   * never by dockview's `IPanel.update` callback.
   * @example
   * ```
   * // private method; not part of the public API
   * declare const renderer: AdoptingContentRenderer;
   * renderer.update();
   * ```
   */
  update(): void {}

  /** No serializable state of its own to contribute to dockview's layout JSON.
   * @returns An empty object.
   * @example
   * ```
   * // private method; not part of the public API
   * declare const renderer: AdoptingContentRenderer;
   * renderer.toJSON();
   * ```
   */
  toJSON(): object {
    return {};
  }

  /** No-op: DOM focus for the adopted element is directed by its own owner
   * (PanelHost's staging container, or the stage), never by dockview's
   * tab-activation focus callback.
   * @example
   * ```
   * // private method; not part of the public API
   * declare const renderer: AdoptingContentRenderer;
   * renderer.focus();
   * ```
   */
  focus(): void {}

  /** Detaches (never destroys) the adopted element, returning it to its
   * original owner — see the class doc comment's ownership contract.
   * @example
   * ```
   * // private method; not part of the public API — invoked only by
   * // dockview-core when this content renderer is torn down
   * declare const renderer: AdoptingContentRenderer;
   * renderer.dispose();
   * ```
   */
  dispose(): void {
    if (this.#adopted && this.#adopted.parentElement === this.element) {
      this.element.removeChild(this.#adopted);
    }
    this.#adopted = null;
  }
}

/** Derives a dockview group id from the zone-group's CONTENT (its first tab
 * id) rather than its positional index — a group's dockview identity then
 * survives a sibling's insert/removal in the same zone, which would
 * otherwise shift every later index (mirrors `applyOp`'s `dock` case, which
 * resolves a target group's identity before `detach` runs for the same
 * reason). A reordered or emptied-then-refilled group gets a fresh id and
 * is recreated — accepted as a minor churn cost; a finer content-independent
 * diff is future work.
 * @param zone The dock zone the group belongs to.
 * @param index The group's positional index within `zone`, used only as a
 * fallback suffix when `tabs` is empty.
 * @param tabs The group's current tab ids, in order; `tabs[0]` is the id this
 * function keys on.
 * @returns A stable dockview group id for this content, or an index-suffixed
 * fallback id when the group has no tabs yet.
 * @example
 * ```
 * // private function; not part of the public API
 * groupIdFor("right", 0, ["chat"]); // "sc-group:chat"
 * ```
 */
function groupIdFor(zone: ZoneId, index: number, tabs: readonly string[]): string {
  return `sc-group:${tabs[0] ?? `${zone}:${index}:empty`}`;
}

/** Mounts `PanelMenu` into a small popover positioned under `anchor` (the tab's
 * menu button); returns a `close()` that unmounts it and restores focus to
 * `anchor`. A pointerdown outside the popover, or the menu's own `onClose`
 * (Escape/Tab), closes it — the two paths share this single teardown so
 * neither can leave the other's listener/DOM behind.
 * @param anchor The tab's own menu button; the popover is positioned under it
 * and, on close, refocused (unless a Tab-driven close asked to skip that).
 * @param onCommand Called with the chosen `MenuCommand` once, right before the
 * popover closes itself.
 * @returns A `close(returnFocus?)` teardown: unmounts the popover and, unless
 * `returnFocus` is explicitly `false`, returns DOM focus to `anchor`.
 * @example
 * ```
 * // private function; not part of the public API
 * declare const menuButton: HTMLElement;
 * declare function handleCommand(cmd: MenuCommand): void;
 * const close = mountPanelMenu(menuButton, (cmd) => handleCommand(cmd));
 * close(); // later, or from the menu's own onClose/pointerdown-outside paths
 * ```
 */
function mountPanelMenu(
  anchor: HTMLElement,
  onCommand: (cmd: MenuCommand) => void,
): (returnFocus?: boolean) => void {
  const container = document.createElement("div");
  container.className = "sc-panel-menu-popover";
  const rect = anchor.getBoundingClientRect();
  container.style.position = "fixed";
  container.style.top = `${rect.bottom}px`;
  container.style.left = `${rect.left}px`;
  document.body.appendChild(container);

  let closed = false;
  // `returnFocus` (default true) lets a Tab-driven close (APG Menu Button
  // pattern) skip forcing focus back onto `anchor`, so native Tab traversal
  // can proceed to the next tabbable element instead of being bounced.
  const close = (returnFocus = true): void => {
    if (closed) return;
    closed = true;
    document.removeEventListener("pointerdown", onDocPointerDown, true);
    unmount(app);
    container.remove();
    if (returnFocus) anchor.focus();
  };
  const onDocPointerDown = (event: PointerEvent): void => {
    if (!container.contains(event.target as Node)) close();
  };
  const app = mount(PanelMenu, { target: container, props: { onCommand, onClose: close } });
  document.addEventListener("pointerdown", onDocPointerDown, true);
  // First menu item takes focus (WAI-ARIA Menu pattern: opening a menu moves
  // focus onto it). Deferred a tick: `mount()` has already appended the
  // buttons synchronously, but queuing avoids fighting the click event that
  // opened this same menu (still bubbling) for focus.
  queueMicrotask(() => container.querySelector<HTMLButtonElement>("button")?.focus());
  return close;
}

/** Renders a tab's visible chrome — icon, `i18n.t(labelKey)` label, and a
 * command-menu button — as the CONTENT of dockview's own tab wrapper
 * (`Tab#_element`, `role="tab"`), which already implements APG
 * roving tabindex (arrow keys move that wrapper's focus without activating;
 * Enter/Space activates) and is not reimplemented here. `getMeta` is read
 * lazily (not snapshotted at construction) so a later `apply()` call's meta
 * map is always current; an `i18n.subscribe` re-renders the label on locale
 * change. */
class PanelTabRenderer implements ITabRenderer {
  /** This tab's chrome wrapper, rendered as the content of dockview's own tab element. */
  readonly element: HTMLElement;
  /** The tab's icon element. */
  #iconEl: HTMLElement;
  /** The tab's label element; text kept current with `i18n.subscribe` and `getMeta()`. */
  #labelEl: HTMLElement;
  /** The tab's badge count element; visibility/text driven by `#unsubBadge`'s callback. */
  #badgeEl: HTMLElement;
  /** The tab's command-menu button. */
  #menuBtn: HTMLButtonElement;
  /** Unsubscribes this renderer's `i18n.subscribe` locale-change listener. */
  #unsubLocale: () => void;
  // `PanelMeta.badge`'s own subscribe, bound once at construction: unlike
  // icon/label (read live through `getMeta()` on every render, since the
  // meta MAP can be rebuilt), the badge object itself is stable for a
  // panel's whole lifetime (registered once at module install), and its
  // count changes independently of any `apply()` cycle — see `PanelBadge`'s
  // doc comment.
  /** Unsubscribes this tab's `PanelMeta.badge` count listener; `null` when the panel's meta
   * carries no badge. */
  #unsubBadge: (() => void) | null = null;
  /** Closes the currently-open command-menu popover; `null` when no popover is open. */
  #closeMenu: (() => void) | null = null;

  /** Builds this tab's chrome (icon/label/badge/menu-button elements) and wires
   * its live subscriptions (`i18n.subscribe` for locale changes, the panel's
   * own `PanelMeta.badge` for count changes) — see the class doc comment for
   * why label/icon stay live through `getMeta()` while the badge subscription
   * is bound once here.
   * @param id This tab's panel id — used as the label fallback when `getMeta()`
   * resolves to nothing, and passed back through `onCommand`.
   * @param getMeta Resolves this tab's current `PanelMeta` (icon/label/badge),
   * read live on every render rather than snapshotted at construction.
   * @param onCommand Called with `(id, cmd, invoker)` when this tab's menu
   * button chooses a command — `invoker` is always this tab's own menu button.
   * @example
   * ```
   * // private class; not part of the public API — constructed only by
   * // DockviewEngine's own `createTabComponent` factory
   * declare const meta: PanelMeta | undefined;
   * new PanelTabRenderer("chat", () => meta, (id, cmd, invoker) => {});
   * ```
   */
  constructor(
    private readonly id: string,
    private readonly getMeta: () => PanelMeta | undefined,
    private readonly onCommand: (id: string, cmd: MenuCommand, invoker: HTMLElement) => void,
  ) {
    this.element = document.createElement("div");
    this.element.className = "sc-tab";
    this.#iconEl = document.createElement("span");
    this.#iconEl.className = "sc-tab-icon";
    this.#iconEl.setAttribute("aria-hidden", "true");
    this.#labelEl = document.createElement("span");
    this.#labelEl.className = "sc-tab-label";
    this.#badgeEl = document.createElement("span");
    this.#badgeEl.className = "sc-tab-badge";
    this.#menuBtn = document.createElement("button");
    this.#menuBtn.type = "button";
    this.#menuBtn.className = "sc-tab-menu-btn";
    this.#menuBtn.setAttribute("aria-haspopup", "menu");
    this.#menuBtn.setAttribute("aria-expanded", "false");
    this.#menuBtn.textContent = "⋮";
    this.element.append(this.#iconEl, this.#labelEl, this.#badgeEl, this.#menuBtn);
    this.#renderLabels();
    this.#renderBadge();
    this.#unsubLocale = i18n.subscribe(() => this.#renderLabels());
    this.#unsubBadge = this.getMeta()?.badge?.subscribe(() => this.#renderBadge()) ?? null;
    this.#menuBtn.addEventListener("click", (event) => {
      // Never let a menu-button click bubble into the tab wrapper's own
      // click handling (`Tab#_onTabClick`), which would also activate this
      // tab as a side effect of opening its menu.
      event.stopPropagation();
      this.#toggleMenu();
    });
  }

  /** No-op: no per-panel state to read from dockview's own init parameters —
   * label/icon come entirely from `getMeta()`, never `params.title`.
   * @param _params Dockview's tab-part init parameters; unused.
   * @example
   * ```
   * // private method; not part of the public API — invoked only by
   * // dockview-core's own render lifecycle (ITabRenderer.init)
   * declare const renderer: PanelTabRenderer;
   * declare const params: TabPartInitParameters;
   * renderer.init(params);
   * ```
   */
  init(_params: TabPartInitParameters): void {}

  /** Re-renders the icon/label from a fresh `getMeta()` read, and the menu
   * button's `aria-label`. Called at construction and again on every
   * `i18n.subscribe` locale-change notification, so a live locale switch
   * relabels every open tab without a remount.
   * @example
   * ```
   * // private method; not part of the public API
   * this.#renderLabels();
   * ```
   */
  #renderLabels(): void {
    const meta = this.getMeta();
    this.#iconEl.textContent = meta?.icon ?? "";
    this.#labelEl.textContent = meta ? i18n.t(meta.labelKey) : this.id;
    this.#menuBtn.setAttribute("aria-label", i18n.t("panels.menu"));
  }

  /** Re-renders the badge count from `PanelMeta.badge`, hiding the badge
   * element entirely at a count of `0` rather than rendering a visible "0".
   * Called at construction and again on every `#unsubBadge` notification.
   * @example
   * ```
   * // private method; not part of the public API
   * this.#renderBadge();
   * ```
   */
  #renderBadge(): void {
    const count = this.getMeta()?.badge?.get() ?? 0;
    if (count > 0) {
      this.#badgeEl.textContent = String(count);
      this.#badgeEl.hidden = false;
    } else {
      this.#badgeEl.textContent = "";
      this.#badgeEl.hidden = true;
    }
  }

  /** Opens the command popover on a closed menu, or closes it via the SAME
   * `#closeMenu` teardown `mountPanelMenu` returned on the alternate call —
   * mirrors the popover's own pointerdown-outside/Escape close paths, so no
   * path can leave the other's listener/DOM behind (see `mountPanelMenu`'s
   * doc comment).
   * @example
   * ```
   * // private method; not part of the public API — invoked only by this
   * // tab's own menu-button click handler
   * this.#toggleMenu();
   * ```
   */
  #toggleMenu(): void {
    if (this.#closeMenu) {
      this.#closeMenu();
      return;
    }
    const close = mountPanelMenu(this.#menuBtn, (cmd) => {
      close();
      this.onCommand(this.id, cmd, this.#menuBtn);
    });
    this.#menuBtn.setAttribute("aria-expanded", "true");
    this.#closeMenu = () => {
      close();
      this.#menuBtn.setAttribute("aria-expanded", "false");
      this.#closeMenu = null;
    };
  }

  /** Tears down every live subscription this tab holds: an open menu popover
   * (if any), the locale subscription, and the badge subscription.
   * @example
   * ```
   * // private method; not part of the public API — invoked only by
   * // dockview-core when this tab renderer is torn down
   * declare const renderer: PanelTabRenderer;
   * renderer.dispose();
   * ```
   */
  dispose(): void {
    this.#closeMenu?.();
    this.#unsubLocale();
    this.#unsubBadge?.();
  }
}

/** Gesture contract: classify → veto or redispatch; dockview never self-mutates
 * from a drop. Every will-drop wire (`#handleWillDrop`, fed by both the
 * component-level `api.onWillDrop` and the per-group `group.model.onWillDrop`
 * — see `#groupWillDropSubs`) either vetoes (`preventDefault()`, no further
 * action) or, for an ALLOWED classification, ALSO `preventDefault()`s and
 * instead emits the classified `LayoutOp` to `#opListeners`. dockview's own
 * internal move machinery (`_onMove` → `DockviewComponent.moveGroupOrPanel`,
 * reached from the `onMove` subscription in `DockviewComponent.createGroup`)
 * is consequently never reached for a
 * completed same-instance drag: the tree is canonical, and the controller's
 * `apply()` — driven by the op this class just emitted — is the one
 * sanctioned mutation path back into dockview. `onDidDrop` fires in
 * `DockviewGroupPanelModel.handleDropEvent`'s `else` branch,
 * taken when the drop's data is either ABSENT entirely (a drag whose payload
 * carries no `PanelTransfer` at all — e.g. an external/OS drag) or present
 * with a `viewId` that doesn't match `this.accessor.id` — not only the
 * viewId-mismatch case. It has no wiring here regardless of which of those
 * two shapes a drop takes, because `#handleWillDrop` (and its
 * `#handleGroupWillDrop` delegate) `preventDefault()`s on EVERY path it can
 * take — including when `#toDropSite` resolves no subject at all (`!id`)
 * — which trips `handleDropEvent`'s own
 * `if (willDropEvent.defaultPrevented) return;`
 * before the model ever reaches the
 * data/viewId branch quoted above. So `onDidDrop` is unreachable here not
 * because a mismatched `viewId` can't occur, but because this class's
 * veto-or-redispatch contract never lets `handleDropEvent` get that far.
 * (Separately, every `PanelTransfer` is stamped with its constructing
 * accessor's own id and consumed identically wherever dockview reads one, so
 * a PANEL drag actually originating in THIS codebase's one `DockviewApi`
 * per `PanelHost` could never present a mismatched `viewId` in the first
 * place, even if this class's own veto did not already foreclose the
 * question.) */
export class DockviewEngine implements EngineAdapter {
  /** The live dockview API; `null` before `init` runs or after `destroy`. */
  #api: DockviewApi | null = null;
  /** Subscribers registered via `onOp`. */
  #opListeners = new Set<(op: LayoutOp) => void>();
  /** Every dockview subscription this engine owns, disposed in bulk by `destroy`. */
  #disposables: {
    /** Tears down this one subscription. */
    dispose(): void;
  }[] = [];
  /** Diagnostic sink for vetoed gestures and recoverable failures. */
  #logger: Logger;
  /** The layout last passed to `apply`; `null` before the first `apply` call. */
  #expanded: ExpandedLayout | null = null;
  /** dockview group id -> our (zone, index) for that group, as of the last `apply()`. Used
   * to translate a drop event's target group back into our vocabulary; rebuilt fresh on
   * every `apply()` call. */
  #zoneOfGroup = new Map<
    string,
    {
      /** The zone this dockview group corresponds to. */
      zone: ZoneId;
      /** This group's positional index within `zone`, as of the last `apply()`. */
      index: number;
    }
  >();
  /** True for the synchronous duration of `apply()` — panel/group removals dockview fires
   * WHILE we are diffing are OUR OWN reconciliation, not a user gesture, so
   * `onDidRemovePanel`/`onDidActivePanelChange` must not re-emit them as ops (that would
   * fight the reducer that just drove this very `apply()` call). */
  #applying = false;
  /** Reentrancy guard: `#restoreStage` itself adds a panel; without this, the model's own
   * remove/add bookkeeping could recurse back in. */
  #restoringStage = false;
  /** One live `onDidDimensionsChange` subscription per managed (non-stage) group, keyed by
   * dockview group id. Added the moment `apply()` creates a group, disposed the moment
   * `apply()` removes it (and on `destroy()`) — a group's whole lifetime is bracketed by
   * exactly one subscription. */
  #groupResizeSubs = new Map<
    string,
    {
      /** Tears down this group's dimension-change subscription. */
      dispose(): void;
    }
  >();
  /** One live `group.model.onWillDrop` subscription per managed (non-stage) group — same
   * add/dispose lifecycle as `#groupResizeSubs` above. Required because
   * `DockviewApi.onWillDrop` (subscribed once in `init()`, below) NEVER fires for a drop
   * targeting an existing group: the component only forwards a group model's `onWillDrop`
   * through `_advancedDnDService?.dispatchWillDrop(event)` (in
   * `DockviewComponent.createGroup`'s `onWillDrop` wiring), and this codebase ships no
   * `advancedDnDService` module (`AllModules` lists none), so that optional chain is a
   * permanent no-op — `#handleWillDrop` is otherwise unreachable for any group-target drop
   * (header, tab, or content). `IDockviewGroupPanelModel.onWillDrop` is a public event on
   * the SAME `group.model` this class already reaches via `api.getGroup(groupId)`, fires
   * with the identical `DockviewWillDropEvent` shape as the component-level event (where
   * `handleDropEvent` constructs the event), and gates `_onMove`/`_onDidDrop` on
   * `defaultPrevented` exactly like the root path (`DockviewGroupPanelModel.handleDropEvent`)
   * — so binding `#handleWillDrop` to it directly closes the group-onto-group veto bypass
   * AND lets an ALLOWED group-target drop be intercepted-and-redispatched the same way as a
   * root/edge drop, no new method needed. */
  #groupWillDropSubs = new Map<
    string,
    {
      /** Tears down this group's `onWillDrop` subscription. */
      dispose(): void;
    }
  >();
  /** Last EMITTED px dimension per zone, to skip dockview's frequent sub-pixel dimension
   * churn (every layout pass fires this event, not just a user's splitter drag) — avoids
   * feedback-loop op spam. */
  #lastZonePx = new Map<ZoneId, number>();
  /** Last EMITTED px dimension per group, keyed by dockview group id; same purpose as
   * `#lastZonePx`. */
  #lastGroupPx = new Map<string, number>();
  /** Meta snapshot from the last `apply()` call — read by `PanelTabRenderer`'s `getMeta`
   * closure (icon/labelKey are effectively static per panel once registered, so a live
   * reference here is sufficient; no per-render diffing). */
  #meta: ReadonlyMap<string, PanelMeta> = new Map();
  /** Invoking element (the menu button) for each floating panel created via a "float" menu
   * command, so `#teardownFloatingA11y` can return focus to it on close. Absent for any
   * future non-menu float path — that path simply degrades to no focus-return, never a
   * crash. */
  #floatInvokers = new Map<string, HTMLElement>();
  /** One live Escape-to-close keydown listener per floating panel's dialog element,
   * disposed the moment that panel leaves the model (mirrors `#groupResizeSubs`'s per-group
   * add/dispose lifecycle). */
  #floatingEscapeSubs = new Map<string, () => void>();
  /** Ids currently mid-transition from docked into floating WITHIN THE SAME `apply()` call
   * (the floating loop's remove+re-add-under-same-id branch, mirroring the zone loop's
   * cross-group-move pattern). `api.removePanel` fires `onDidRemovePanel` synchronously — by
   * the time that listener runs, dockview has already detached the outgoing tab's DOM
   * (confirmed: a `document.contains` check inside the listener already reads false), so
   * `#teardownFloatingA11y` never gets to invoke `invoker.focus()` here regardless of this
   * guard. What it WOULD otherwise do, wrongly, is `#floatInvokers.delete(id)` — discarding
   * the very entry `apply()`'s floating loop is about to hand to the panel's OWN new
   * floating dialog a few lines later. Gating the delete on this set keeps the invoker
   * entry intact across the transient churn, available to a LATER, real close of the
   * floating panel. */
  #floatTransitionIds = new Set<string>();
  /** Last rect (px, relative to the dockview root) `apply()` last placed a floating panel
   * at OR this class last emitted via `resizeFloating` — whichever happened most recently.
   * Read by `#handleFloatingLayoutChange` to detect a real user-driven drift and to
   * suppress re-emitting a rect the tree already knows about. Deleted alongside
   * `#floatingEscapeSubs`/`#floatInvokers` in `#teardownFloatingA11y` (mirrors their per-id
   * add/dispose lifecycle) and cleared wholesale in `destroy()`. */
  #lastFloatingRect = new Map<string, Rect>();
  /** Subscribers registered via `onNotice`. */
  #noticeListeners = new Set<(key: string) => void>();
  /** Popout group id -> the panel ids it hosts, recorded when a pop-out succeeds so
   * `onDidRemovePopoutGroup` (window closed by the user) can translate a group id back into
   * `popIn` ops without depending on the group's live panel membership at fire time
   * (dockview's teardown may have already moved them). */
  #poppedOutGroupPanels = new Map<string, string[]>();
  /** Ids with a pop-out request currently in flight (driver called, promise not yet
   * settled). dockview's `addPopoutGroup` is wrapped in `mutation()`, whose `finally` fires
   * the instant the function RETURNS its pending promise — not when it settles — so it
   * serializes only the synchronous portion, leaving the async `window.open` → re-parent
   * gap unguarded. Without this set, a second "Pop out" click on the SAME id inside that
   * gap opens a second window and re-keys `#poppedOutGroupPanels` to the first
   * (now-orphaned) group id, corrupting the tree on the eventual window close. Refuse a
   * duplicate until the first request settles. */
  #pendingPopouts = new Set<string>();
  /** Popped-out panel id -> the group id it lived in BEFORE pop-out (its ORIGIN group).
   * dockview keeps that group alive-but-hidden (`setVisible(false)`) and its window-close
   * path (`disposePopoutWindow`) hands the panel back to that exact group object; the
   * `popOut` op meanwhile detaches the group from the persisted tree, so `apply()` seeds
   * these ids into `seenGroupIds` to keep the orphan-group loop from destroying the group
   * dockview still depends on. */
  #poppedOutOriginGroups = new Map<string, string>();
  /** Gesture-time popout invoker. Defaults to dockview's native popout (verified same-heap,
   * content re-parented, stylesheets cloned, via `PopoutWindow.open`). Injectable so unit
   * tests exercise the async-result → op translation without a real `window.open` (jsdom
   * has none). */
  #popoutDriver: (panel: IDockviewPanel) => Promise<boolean>;

  /** Builds an engine instance with no dockview API yet (that is created by
   * `init()`, which a `PanelHost` calls once at mount). `popoutDriver` is a
   * seam for unit tests, not a production configuration point. `PanelHost`
   * itself never constructs a `DockviewEngine` — it receives `engine` as an
   * optional prop, defaulting to `new FakeEngine()` when absent
   * (`PanelHost`'s `engine` prop); the real production construction is the
   * panels module's `panels.register()`, with ONE argument
   * (`props: { engine: new DockviewEngine(consoleLogger()) }`).
   * @param logger Diagnostic sink for vetoed gestures and recoverable
   * failures (defaults to `consoleLogger()`).
   * @param popoutDriver Replaces dockview's native `addPopoutGroup` call for
   * pop-out requests; defaults to the real driver. Injectable so a test can
   * exercise the async-result → op translation without a real `window.open`
   * (jsdom has none).
   * @example
   * ```ts
   * import { DockviewEngine } from "@shadowcat/module-panels";
   *
   * const engine = new DockviewEngine();
   * ```
   */
  constructor(logger?: Logger, popoutDriver?: (panel: IDockviewPanel) => Promise<boolean>) {
    this.#logger = logger ?? consoleLogger();
    // `/popout.html` is the same-origin loader document dockview's popout
    // window navigates to; passed explicitly to document the dependency
    // rather than relying on dockview's own default drifting.
    // `assertSameOriginPopoutUrl` rejects
    // `about:blank`/cross-origin, so this URL is load-bearing.
    this.#popoutDriver = popoutDriver ?? ((panel) => this.#api!.addPopoutGroup(panel, { popoutUrl: "/popout.html" }));
  }

  /** `EngineAdapter.init`: creates the underlying `DockviewApi` against `host`,
   * registers the content/tab component factories (`createComponent`,
   * `createTabComponent` — branching on `options.name`/`options.id` per the
   * inline comments below), mounts the stage (`#mountStage`), and
   * subscribes every component-level event this class translates into ops or
   * DOM/focus teardown (`onWillDrop`, `onDidRemovePanel`,
   * `onDidActivePanelChange`, `onDidRemovePopoutGroup`, `onDidLayoutChange`).
   * Called exactly once, at `PanelHost` mount.
   * @param host The dockview root element; classed `sc-dockview-root` and
   * handed to `createDockview` unmodified otherwise.
   * @param slotFor Resolves a panel id to its persistent, already-mounted slot
   * element — adopted, never re-created, by this engine's content renderers.
   * @param stageEl The shared canvas/stage element, adopted into its own
   * dedicated headerless group.
   * @example
   * ```ts
   * import { DockviewEngine } from "@shadowcat/module-panels";
   *
   * const engine = new DockviewEngine();
   * declare const host: HTMLElement;
   * declare const stageEl: HTMLElement;
   * declare const slots: Map<string, HTMLElement>;
   * engine.init(host, (id) => slots.get(id)!, stageEl);
   * ```
   */
  init(host: HTMLElement, slotFor: (id: string) => HTMLElement, stageEl: HTMLElement): void {
    host.classList.add("sc-dockview-root");

    const api = createDockview(host, {
      createComponent: (options: CreateComponentOptions) =>
        options.name === "sc-stage"
          ? new AdoptingContentRenderer(() => stageEl, "sc-dockview-stage-content")
          : new AdoptingContentRenderer(() => slotFor(options.id), "sc-dockview-panel-content"),
      // `DockviewPanelModel.createTabComponent` only
      // calls `options.createTabComponent` at all when `componentName ??
      // defaultTabComponent` is truthy — no per-panel `tabComponent` is ever
      // set on `addPanel`, so this MUST be a truthy string or `createTabComponent`
      // below is never reached and every tab silently falls back to
      // dockview's own `DefaultTab`. The value itself is arbitrary; `options.id`
      // (not `options.name`, which is always this same string) is what the
      // factory below branches on.
      defaultTabComponent: "sc-tab",
      // The stage's own group is headerless (`hideHeader: true`, set by
      // `#mountStage`), so this is never actually invoked for the stage id —
      // the `undefined` fallback (dockview's `DefaultTab`) is defense-in-depth
      // only, matching `apply`/`focus`'s belt-and-suspenders STAGE_ID guards
      // elsewhere in this class.
      createTabComponent: (options: CreateComponentOptions) =>
        options.id === STAGE_ID
          ? undefined
          : new PanelTabRenderer(
              options.id,
              () => this.#meta.get(options.id),
              (id, cmd, invoker) => this.#handleMenuCommand(id, cmd, invoker),
            ),
    });
    this.#api = api;

    this.#mountStage(api);

    this.#disposables.push(
      api.onWillDrop((event) => this.#handleWillDrop(event)),
      api.onDidRemovePanel((panel) => this.#handleDidRemovePanel(panel)),
      api.onDidActivePanelChange((event) => this.#handleActivePanelChange(event)),
      api.onDidRemovePopoutGroup((event) => this.#handleRemovePopoutGroup(event)),
      api.onDidLayoutChange(() => this.#handleFloatingLayoutChange()),
    );
  }

  /** Translates a `PanelMenu` command into the `LayoutOp` `opForMenuCommand`
   * (dockview-free) maps it to, and emits it through the SAME
   * `#opListeners` channel a drag gesture uses — the controller cannot tell
   * the two apart, and per the parity requirement, it doesn't need to: both
   * paths produce identical `LayoutOp` shapes for the equivalent move.
   * `float` additionally records `invoker` (the tab's own menu button,
   * handed in by `PanelTabRenderer`) as this panel's focus-return target
   * (see `#floatInvokers`). Refuses `STAGE_ID` in TWO independent layers:
   * the early return below (mirrors `focus()`'s STAGE_ID defense-in-depth
   * guard) and `opForMenuCommand`'s own veto (mirrors `classifyDrop`'s stage
   * veto) — belt-and-suspenders alongside `createTabComponent`'s `STAGE_ID`
   * branch (`init()`) and `#mountStage`'s headerless stage group, which never
   * gives the stage a `PanelTabRenderer`/menu button to invoke this with in
   * the first place; neither guard here is the sole line of defense.
   * @param id The panel id the command targets.
   * @param cmd The chosen `MenuCommand`.
   * @param invoker The tab's own menu button — recorded as the focus-return
   * target for a `"float"` command (see `#floatInvokers`).
   * @example
   * ```
   * // private method; not part of the public API — invoked only by
   * // PanelTabRenderer's own menu-command callback
   * declare const menuButton: HTMLElement;
   * this.#handleMenuCommand("chat", "float", menuButton);
   * ```
   */
  #handleMenuCommand(id: string, cmd: MenuCommand, invoker: HTMLElement): void {
    if (id === STAGE_ID) return;
    const result = opForMenuCommand(cmd, id);
    if ("veto" in result) {
      this.#logger.warn(`panels: vetoed menu command (${result.reason})`, { id, cmd });
      return;
    }
    if (cmd === "float") this.#floatInvokers.set(id, invoker);
    if (cmd === "popOut") {
      // Pop-out is imperative + gesture-timed: `window.open` (inside
      // addPopoutGroup, synchronous before its first await) must run in THIS
      // click's tick or the browser blocks it. The tree op is emitted only
      // after the async open resolves — unlike every other menu command, which
      // reduces declaratively through `apply()`. Same gesture-timing reason
      // `#floatInvokers` is captured synchronously above.
      this.#requestPopOut(id);
      return;
    }
    for (const cb of this.#opListeners) cb(result);
  }

  /** Gesture-time pop-out: drives dockview's native `addPopoutGroup`
   * synchronously (preserving the user gesture), then translates the async
   * result into a tree op. Success ⇒ `popOut` (records the id + its live popout
   * group for close-translation). Block/throw ⇒ falls back to `float` + a
   * `panels.popoutBlocked` notice.
   * @param id The panel id to pop out.
   * @example
   * ```
   * // private method; not part of the public API — invoked only from
   * // #handleMenuCommand's "popOut" branch
   * this.#requestPopOut("chat");
   * ```
   */
  #requestPopOut(id: string): void {
    const api = this.#api;
    if (!api) return;
    const panel = api.getPanel(id);
    if (!panel) return;
    // In-flight guard: dockview's `mutation()` bracket around `addPopoutGroup`
    // does not span the async window.open → re-parent gap (see `#pendingPopouts`),
    // so a duplicate request in that gap is refused here, mirroring the
    // logged-warning-on-reentry style of the menu-command veto path.
    if (this.#pendingPopouts.has(id)) {
      this.#logger.warn("panels: pop-out already in flight; ignoring duplicate request", { id });
      return;
    }
    this.#pendingPopouts.add(id);
    // Origin group captured BEFORE the driver re-parents the panel: after a
    // successful pop-out `panel.group.id` is the POPOUT group's own id, so the
    // origin must be read now (see `#poppedOutOriginGroups`).
    const originGroupId = panel.group.id;
    this.#popoutDriver(panel)
      .then((ok) => {
        this.#pendingPopouts.delete(id);
        if (ok) {
          this.#poppedOutOriginGroups.set(id, originGroupId);
          const gid = api.getPanel(id)?.group.id;
          if (gid) this.#poppedOutGroupPanels.set(gid, [id]);
          for (const cb of this.#opListeners) cb({ op: "popOut", id });
        } else {
          for (const cb of this.#opListeners) cb({ op: "float", id, rect: MENU_FLOAT_RECT });
          this.#emitNotice("panels.popoutBlocked");
        }
      })
      .catch((err) => {
        this.#pendingPopouts.delete(id);
        this.#logger.warn("panels: pop-out failed; falling back to floating", { id, err });
        for (const cb of this.#opListeners) cb({ op: "float", id, rect: MENU_FLOAT_RECT });
        this.#emitNotice("panels.popoutBlocked");
      });
  }

  /** Mounts the stage into its own dedicated group — headerless (no tab
   * strip, so no close/drag affordance exists at all: `hideHeader: true`
   * sets `header.hidden = true`, and `header` IS the group's `TabsContainer`
   * instance, whose `hidden` setter sets the whole tabs-and-actions
   * element's `display: none`) and locked to `'no-drop-target'` (the
   * model's own drop handler returns before a drop event is even
   * constructed against a group locked this way). Also used by
   * `#restoreStage` to remount after an unexpected removal.
   * @param api The live `DockviewApi` to mount the stage group/panel into.
   * @returns The stage's dedicated dockview group (created, or the existing
   * one re-locked, on a repeat call).
   * @example
   * ```
   * // private method; not part of the public API — invoked only from
   * // init() and #restoreStage()
   * declare const api: DockviewApi;
   * this.#mountStage(api);
   * ```
   */
  #mountStage(api: DockviewApi): IDockviewGroupPanel {
    let stageGroup = api.getGroup(STAGE_GROUP_ID);
    if (!stageGroup) {
      stageGroup = api.addGroup({
        id: STAGE_GROUP_ID,
        // Arbitrary: this is the very first group added to an empty grid,
        // so it fills the whole container regardless of `direction`.
        direction: "right",
        hideHeader: true,
        locked: "no-drop-target",
      });
    } else {
      stageGroup.locked = "no-drop-target";
    }
    if (!api.getPanel(STAGE_ID)) {
      api.addPanel({
        id: STAGE_ID,
        component: "sc-stage",
        position: { referenceGroup: stageGroup.id, direction: "within" },
      });
    }
    return stageGroup;
  }

  /** Fail-safe invariant guard. If the stage panel ever leaves the
   * model (a bug elsewhere, a dockview behaviour change, or a wrapper-API
   * gap), remount it immediately and log — the stage must never simply
   * vanish.
   * @example
   * ```
   * // private method; not part of the public API — invoked only from
   * // #handleDidRemovePanel when the stage panel itself was removed
   * this.#restoreStage();
   * ```
   */
  #restoreStage(): void {
    if (this.#restoringStage) return;
    const api = this.#api;
    if (!api) return;
    this.#restoringStage = true;
    try {
      this.#logger.error("panels: stage panel left the dockview model; restoring it");
      this.#mountStage(api);
    } finally {
      this.#restoringStage = false;
    }
  }

  /** `DockviewApi.onDidRemovePanel` handler: runs floating-a11y teardown and
   * the `#restoreStage` stage-restore guard for EVERY panel removal regardless of cause,
   * then — unless the removal is our own `apply()` reconciliation, or the
   * transient docked→floating transition `#floatTransitionIds` brackets —
   * redispatches it as a `close` `LayoutOp`. See the inline comments below for
   * each branch's reasoning.
   * @param panel The panel dockview just removed.
   * @example
   * ```
   * // private method; not part of the public API — invoked only by
   * // dockview-core's own onDidRemovePanel event
   * declare const panel: IDockviewPanel;
   * this.#handleDidRemovePanel(panel);
   * ```
   */
  #handleDidRemovePanel(panel: IDockviewPanel): void {
    // Always runs, even for an `#applying`-driven removal (e.g. a floating
    // panel docked or closed via the reducer) — this is a DOM/focus
    // teardown, not a user-gesture translation, so it does not share the
    // `#applying` guard below. EXCEPT for the docked→floating transient
    // remove+re-add this panel's own `apply()` call is mid-way through (see
    // `#floatTransitionIds`): that removal is not a real close, and running
    // teardown here would discard the invoker entry `apply()`'s floating
    // loop is about to reuse for this SAME id's new floating dialog.
    if (!this.#floatTransitionIds.has(panel.id)) {
      this.#teardownFloatingA11y(panel.id);
    }
    if (panel.id === STAGE_ID) {
      this.#restoreStage();
      return;
    }
    // A removal driven by our own `apply()` diff is not a user gesture —
    // the reducer that produced the tree this `apply()` reconciles from
    // already knows about it; re-emitting here would just replay a stale op.
    if (this.#applying) return;
    for (const cb of this.#opListeners) cb({ op: "close", id: panel.id });
  }

  /** Floating groups are non-modal dialogs (dockview's own `Overlay`
   * sets `role="dialog"`/`aria-modal="false"` — see `Overlay`'s constructor); this adds
   * the label + focus management dockview doesn't supply itself:
   * `aria-label` = the panel's own label, DOM focus moves
   * into the dialog the moment it appears, and Escape (bubbled from
   * anywhere inside it) closes it via the same op channel a menu/drag
   * gesture uses. Called once per floating-panel CREATION (`apply()`'s
   * floating loop) — `panel.group.element` is a descendant of the dialog's
   * `Overlay#_element`, so `.closest()` finds it without reaching into any
   * dockview-internal field.
   * @param id The panel id that just became a floating group.
   * @param meta The panel's `PanelMeta`, for the dialog's `aria-label` — a
   * missing meta leaves the label unset rather than throwing.
   * @example
   * ```
   * // private method; not part of the public API — invoked only from
   * // apply()'s floating loop, once per newly-created floating panel
   * declare const meta: PanelMeta | undefined;
   * this.#wireFloatingA11y("chat", meta);
   * ```
   */
  #wireFloatingA11y(id: string, meta: PanelMeta | undefined): void {
    const panel = this.#api?.getPanel(id);
    if (!panel) return;
    const dialogEl = panel.group.element.closest<HTMLElement>('[role="dialog"]');
    if (!dialogEl) return;
    dialogEl.tabIndex = -1;
    if (meta) dialogEl.setAttribute("aria-label", i18n.t(meta.labelKey));
    const onKeydown = (event: KeyboardEvent): void => {
      if (event.key !== "Escape") return;
      event.stopPropagation();
      for (const cb of this.#opListeners) cb({ op: "close", id });
    };
    dialogEl.addEventListener("keydown", onKeydown);
    this.#floatingEscapeSubs.set(id, () => dialogEl.removeEventListener("keydown", onKeydown));
    dialogEl.focus();
  }

  /** Reverses `#wireFloatingA11y` for `id` — disposes its Escape listener and
   * returns DOM focus to whatever menu button opened it (`#floatInvokers`),
   * when that element is still attached. A no-op (safe, idempotent) for any
   * id that was never floating. Called from `#handleDidRemovePanel`, which
   * fires for every panel removal regardless of cause (menu close, Escape,
   * or a dock transition moving the panel out of its floating window).
   * @param id The panel id to tear down floating a11y state for.
   * @example
   * ```
   * // private method; not part of the public API — invoked only from
   * // #handleDidRemovePanel
   * this.#teardownFloatingA11y("chat");
   * ```
   */
  #teardownFloatingA11y(id: string): void {
    this.#floatingEscapeSubs.get(id)?.();
    this.#floatingEscapeSubs.delete(id);
    const invoker = this.#floatInvokers.get(id);
    this.#floatInvokers.delete(id);
    this.#lastFloatingRect.delete(id);
    if (invoker && document.contains(invoker)) invoker.focus();
  }

  /** `DockviewApi.onDidActivePanelChange` handler: redispatches a USER-driven
   * tab activation (`event.origin === "user"` — an `'api'` origin is this
   * class's own `setActive()` call from `apply()`/`focus()`) as an `activeTab`
   * `LayoutOp`, resolving the panel's zone/group from `#zoneOfGroup`. No-ops
   * for the stage panel, an unresolvable group, or during `#applying` (our own
   * reconciliation, not a user gesture).
   * @param event The active-panel-change event; `event.panel` is `undefined`
   * when the active group becomes empty.
   * @example
   * ```
   * // private method; not part of the public API — invoked only by
   * // dockview-core's own onDidActivePanelChange event
   * declare const event: DockviewActivePanelChangeEvent;
   * this.#handleActivePanelChange(event);
   * ```
   */
  #handleActivePanelChange(event: DockviewActivePanelChangeEvent): void {
    if (this.#applying) return;
    if (event.origin !== "user") return; // 'api' origin = our own `setActive()` calls (apply/focus)
    const panel = event.panel;
    if (!panel || panel.id === STAGE_ID) return;
    const zoneInfo = this.#zoneOfGroup.get(panel.group.id);
    if (!zoneInfo) return;
    for (const cb of this.#opListeners) {
      cb({ op: "activeTab", zone: zoneInfo.zone, group: zoneInfo.index, id: panel.id });
    }
  }

  /** A popout window closed — by the user (OS window close, funneled through
   * dockview's single removal path), or by our own reconcile (`apply()` moving
   * a panel out of its popout because the tree no longer marks it popped-out).
   * The map cleanup runs unconditionally; the `popIn` redispatch is suppressed
   * during `#applying` — that removal is our own reconciliation of a tree the
   * reducer already updated (identical reasoning to `#handleDidRemovePanel`/
   * `#handleActivePanelChange`), so re-emitting would replay a stale op.
   * @param event The popout-group-removed event.
   * @param event.id The dockview id of the popout group that was removed.
   * @param event.group The removed group itself, whose `model.panels` is the
   * fallback panel-id source when `#poppedOutGroupPanels` has no entry for
   * `event.id`.
   * @example
   * ```
   * // private method; not part of the public API — invoked only by
   * // dockview-core's own onDidRemovePopoutGroup event
   * declare const event: { id: string; group: IDockviewGroupPanel };
   * this.#handleRemovePopoutGroup(event);
   * ```
   */
  #handleRemovePopoutGroup(event: {
    /** The dockview id of the popout group that was removed. */
    id: string;
    /** The removed group itself; see this method's own doc for its fallback role. */
    group: IDockviewGroupPanel;
  }): void {
    const ids = this.#poppedOutGroupPanels.get(event.id) ?? event.group.model.panels.map((p) => p.id);
    this.#poppedOutGroupPanels.delete(event.id);
    // Origin-group tracking clears unconditionally (like `#poppedOutGroupPanels`
    // above), for both a user window-close and our own `apply()`-driven pop-in:
    // once the panel leaves the popped-out state its origin group is a normal
    // tree-owned group again.
    for (const id of ids) this.#poppedOutOriginGroups.delete(id);
    if (this.#applying) return;
    for (const id of ids) {
      // STAGE_ID veto, belt-and-suspenders alongside every other STAGE_ID guard
      // in this class: the stage never enters `poppedOut`, so this only
      // ever matters for the `event.group.model.panels` fallback above — skip
      // it rather than emit a `popIn` for an id that was never a valid subject.
      if (id === STAGE_ID) continue;
      for (const cb of this.#opListeners) cb({ op: "popIn", id });
    }
  }

  /** Intercept-and-redispatch: the ONLY place a dockview drag event is
   * translated into `DropSite` and fed to `classifyDrop` (pure/engine-free).
   * `event.preventDefault()` is now called UNCONDITIONALLY once a layout and
   * a classifiable site exist — for a veto, that is the whole story (dockview
   * performs no DOM move; see the class doc comment); for an ALLOWED
   * classification, `preventDefault()` ALSO runs (dockview must never
   * self-mutate from this gesture), and the classified `LayoutOp` is emitted
   * to `#opListeners` instead. The controller applies that op to the tree and
   * re-enters through `apply()`, which is the one sanctioned mutation path
   * back into dockview (cross-group moves: see `apply()`'s
   * remove+re-add-under-groupId branch). Fails CLOSED on anything this
   * translation layer cannot classify: a null layout (pre-first-`apply()`) or
   * a payload `#toDropSite`/`#handleGroupWillDrop` cannot resolve into a
   * `DropSite` are vetoed outright rather than let through unpoliced. A
   * whole-GROUP transfer (`PanelTransfer.panelId` null for a titlebar drag of
   * an entire group, per `GroupDragSource`) is translated into one `dock`
   * op per tab of the dragged group (`#handleGroupWillDrop`), classified
   * against the group's FIRST tab as a representative subject — the same
   * `classifyDrop` rules apply, so a whole-group drop targeting the
   * container's TOP edge still vetoes (no `"top"` `ZoneId` variant exists)
   * exactly as a single-tab edge drop would.
   *
   * Two independent wires feed this SAME method: `init()`'s `api.onWillDrop`
   * (fires only for root/edge drops — `DockviewComponent`'s constructor's
   * `rootDropTarget.onWillShowOverlay`/`onDrop` wiring is the only path that
   * calls `this._onWillDrop.fire(...)` directly) and, per managed group,
   * `group.model.onWillDrop` (subscribed in `apply()` — see
   * `#groupWillDropSubs`'s doc comment for why the component-level event
   * alone does NOT cover group-target drops). Together they cover every
   * drop this engine can receive: container-edge drops via the component
   * path, group-onto-group drops via the per-group path — and since BOTH
   * now `preventDefault()` every allowed drop too, dockview's own `_onMove`/
   * internal move machinery is never reached from either wire.
   * @param event The will-drop event, from either of the two wires described
   * above.
   * @example
   * ```
   * // private method; not part of the public API — invoked by both
   * // dockview-core's component-level onWillDrop and, per group, the
   * // group model's own onWillDrop
   * declare const event: DockviewWillDropEvent;
   * this.#handleWillDrop(event);
   * ```
   */
  #handleWillDrop(event: DockviewWillDropEvent): void {
    const layout = this.#expanded;
    if (!layout) {
      event.preventDefault();
      return;
    }
    const data = event.getData();
    if (data && data.panelId === null) {
      this.#handleGroupWillDrop(event, data.groupId, layout);
      return;
    }
    const site = this.#toDropSite(event, data?.panelId);
    if (!site) {
      // Any other unclassifiable payload shape — vetoed rather than silently
      // let through (see doc comment above).
      event.preventDefault();
      this.#logger.warn("panels: vetoed drop (unclassifiable payload)");
      return;
    }
    const result = classifyDrop(site, layout);
    event.preventDefault();
    if ("veto" in result) {
      this.#logger.warn(`panels: vetoed drop (${result.reason})`, site);
      return;
    }
    // Allowed: redispatch as our own op rather than letting dockview
    // complete the move internally (see class doc comment).
    for (const cb of this.#opListeners) cb(result);
  }

  /** Translates a whole-group transfer (`PanelTransfer.panelId === null`,
   * `groupId` = the SOURCE group's own dockview id, per `GroupDragSource`/
   * `TabGroupManager`) into one `dock` `LayoutOp` per tab of the dragged group,
   * preserving their relative order, with a SINGLE `preventDefault()` for the
   * whole transfer. Classifies against the group's FIRST tab as a
   * representative subject — every tab in the dragged group moves to the
   * SAME target, so one `classifyDrop` call settles the target zone/group/
   * position for all of them (`#expandGroupDockOp` fans that single result
   * out per-tab). Fails closed (veto, single `preventDefault()`) when the
   * source group can't be resolved (empty tab list — e.g. a stale/foreign
   * group id) or the representative classification itself vetoes, exactly
   * like the single-tab path in `#handleWillDrop`.
   * @param event The will-drop event carrying the whole-group transfer.
   * @param sourceGroupId The dragged group's own dockview id.
   * @param layout The tree `classifyDrop` classifies the representative tab
   * against.
   * @example
   * ```
   * // private method; not part of the public API — invoked only from
   * // #handleWillDrop when a drop's transfer data has a null panelId
   * declare const event: DockviewWillDropEvent;
   * declare const layout: ExpandedLayout;
   * this.#handleGroupWillDrop(event, "sc-group:chat", layout);
   * ```
   */
  #handleGroupWillDrop(event: DockviewWillDropEvent, sourceGroupId: string, layout: ExpandedLayout): void {
    const tabs = this.#api
      ?.getGroup(sourceGroupId)
      ?.model.panels.map((p) => p.id)
      .filter((id) => id !== STAGE_ID);
    if (!tabs || tabs.length === 0) {
      event.preventDefault();
      this.#logger.warn("panels: vetoed drop (unclassifiable payload, e.g. an unresolvable whole-group transfer)");
      return;
    }
    const site = this.#toDropSite(event, tabs[0]);
    if (!site) {
      event.preventDefault();
      this.#logger.warn("panels: vetoed drop (unclassifiable payload, e.g. an unresolvable whole-group transfer)");
      return;
    }
    const result = classifyDrop(site, layout);
    event.preventDefault();
    if ("veto" in result) {
      this.#logger.warn(`panels: vetoed drop (${result.reason})`, site);
      return;
    }
    if (result.op !== "dock") {
      // `classifyDrop` only ever returns a "dock" op for the "edge"/"group"
      // `DropSite` kinds `#toDropSite` produces (the only kinds a drag event
      // can construct) — defensive only, never reached in practice.
      for (const cb of this.#opListeners) cb(result);
      return;
    }
    for (const op of this.#expandGroupDockOp(result, tabs, event, layout)) {
      for (const cb of this.#opListeners) cb(op);
    }
  }

  /** Fans a single representative `dock` op out into one op per tab of the
   * dragged group, preserving relative order. `group: "new"` (an edge drop)
   * creates a fresh group for the FIRST tab only — every op after that
   * targets that SAME new group by numeric index (`group: "new"` a second
   * time would instead create ANOTHER new group), computed as the target
   * zone's CURRENT group count in `layout` (the tree as it stood before any
   * of these ops apply). This assumes the dragged group is not already a
   * member of the target zone (the overwhelmingly common real gesture — a
   * titlebar drag to a DIFFERENT zone's edge); a same-zone whole-group
   * reorder is a residual approximation, matching this file's existing
   * documented fidelity limits (see `#toDropSite`'s target-group-outside-our-
   * zone-bookkeeping approximation). An existing-group target (`group` a
   * number already) instead reuses that same numeric index for every tab,
   * inserting each at a consecutive `tabIndex` starting from the
   * representative op's own resolved position (or the target group's current
   * tab count, when the drop was a "content" drop with no explicit index).
   * @param op The single representative `dock` op `classifyDrop` returned for
   * the dragged group's first tab.
   * @param tabs The dragged group's tab ids, in their original relative order.
   * @param event The will-drop event, for its `event.group` fallback tab count
   * on an existing-group "content" drop.
   * @param layout The tree the target zone's current group count is read from
   * (only consulted for a `group: "new"` op).
   * @returns One `dock` `LayoutOp` per entry in `tabs`.
   * @example
   * ```
   * // private method; not part of the public API — invoked only from
   * // #handleGroupWillDrop
   * declare const op: Extract<LayoutOp, { op: "dock" }>;
   * declare const tabs: readonly string[];
   * declare const event: DockviewWillDropEvent;
   * declare const layout: ExpandedLayout;
   * this.#expandGroupDockOp(op, tabs, event, layout);
   * ```
   */
  #expandGroupDockOp(
    op: Extract<
      LayoutOp,
      {
        /** Selects the `dock` variant of `LayoutOp` for this type-level filter. */
        op: "dock";
      }
    >,
    tabs: readonly string[],
    event: DockviewWillDropEvent,
    layout: ExpandedLayout,
  ): LayoutOp[] {
    if (op.group === "new") {
      const newGroupIndex = layout.zones[op.zone].groups.length;
      return tabs.map((id, i) =>
        i === 0
          ? { op: "dock", id, zone: op.zone, group: "new" }
          : { op: "dock", id, zone: op.zone, group: newGroupIndex, tabIndex: i },
      );
    }
    const targetGroup = op.group;
    const baseIndex = op.tabIndex ?? event.group?.model.panels.length ?? 0;
    return tabs.map((id, i) => ({ op: "dock", id, zone: op.zone, group: targetGroup, tabIndex: baseIndex + i }));
  }

  /** Translator for `onWillDrop`'s `DropSite`, given an explicit subject
   * `id` (the dragged panel's own id for a single-tab drag, or a whole
   * dragged group's representative first tab for `#handleGroupWillDrop`).
   * `onDidDrop` is unwired (see the class doc comment), so `kind` is always
   * the real value dockview supplies on the will-drop event — no
   * approximation branch is needed here anymore.
   * @param event The will-drop event to translate.
   * @param id The drop's subject panel id, or `null`/`undefined` for a payload
   * this translation cannot classify.
   * @returns The translated `DropSite`, or `null` when `id` is absent.
   * @example
   * ```
   * // private method; not part of the public API — invoked only from
   * // #handleWillDrop and #handleGroupWillDrop
   * declare const event: DockviewWillDropEvent;
   * this.#toDropSite(event, "chat");
   * ```
   */
  #toDropSite(event: DockviewWillDropEvent, id: string | null | undefined): DropSite | null {
    if (!id) return null;
    const targetGroupId = event.group?.id;
    const stageGroup = targetGroupId === STAGE_GROUP_ID;
    const kind = event.kind;

    if (kind === "edge" || !event.group) {
      return { kind: "edge", id, position: event.position, stageGroup };
    }

    const zoneInfo = targetGroupId ? this.#zoneOfGroup.get(targetGroupId) : undefined;
    if (!zoneInfo) {
      // A target group outside our own zone bookkeeping — approximated as a
      // fresh edge-zone dock (see class-level doc comment on translation
      // fidelity limits).
      return { kind: "edge", id, position: event.position, stageGroup };
    }

    const tabIndex =
      kind === "tab" && event.panel
        ? event.group?.model.panels.findIndex((p) => p.id === event.panel?.id)
        : undefined;

    return {
      kind: "group",
      id,
      position: event.position,
      stageGroup,
      zone: zoneInfo.zone,
      group: zoneInfo.index,
      ...(tabIndex !== undefined && tabIndex >= 0 ? { tabIndex } : {}),
    };
  }

  /** `EngineAdapter.apply`: reconciles dockview's live widget tree to match
   * `expanded` — walks every zone's groups/tabs (creating/moving dockview
   * groups and panels as needed, subscribing each new group's resize/drop
   * events), then every floating entry (creating or repositioning as needed,
   * wiring floating a11y for a newly-created one), then removes any dockview
   * panel/group the tree no longer names. Popped-out ids and their origin
   * groups are seeded into the "seen" sets first so this pass leaves a live
   * popout untouched (see the inline comments below for why). A no-op before
   * `init()` has run (`this.#api` still `null`).
   * @param expanded The docked-zone + floating layout to reconcile onto.
   * @param meta Per-panel icon/label/badge metadata, read by
   * `PanelTabRenderer`'s tabs and `#wireFloatingA11y`'s dialog label.
   * @example
   * ```ts
   * import { DockviewEngine, type ExpandedLayout } from "@shadowcat/module-panels";
   * import type { PanelMeta } from "@shadowcat/core";
   *
   * const engine = new DockviewEngine();
   * declare const expanded: ExpandedLayout;
   * declare const meta: ReadonlyMap<string, PanelMeta>;
   * engine.apply(expanded, meta);
   * ```
   */
  apply(expanded: ExpandedLayout, meta: ReadonlyMap<string, PanelMeta>): void {
    const api = this.#api;
    if (!api) return;
    this.#expanded = expanded;
    this.#meta = meta;
    this.#applying = true;
    try {
      // Popped-out ids stay in dockview's model (a same-heap popout group), so
      // seed them here — otherwise the orphan-removal loop below tears the live
      // popout panel out of its window. The zone/floating placement loops never
      // list a popped-out id, so `apply()` leaves the popout untouched (hands-
      // off). A menu dock/float on a popped-out panel drops the id from
      // `poppedOut` first, so it is NOT seeded and the placement loops move it
      // back (removePanel of the popout's last panel disposes the window — the
      // resulting onDidRemovePopoutGroup is `#applying`-suppressed).
      const seenPanelIds = new Set<string>([STAGE_ID, ...expanded.poppedOut]);
      const seenGroupIds = new Set<string>([STAGE_GROUP_ID]);
      // A popped-out panel's ORIGIN group stays alive-but-hidden in dockview's
      // model, but the `popOut` op has already stripped it from the persisted
      // tree — so the zone walk below never re-lists it. Seed it (mirroring
      // `seenPanelIds`'s `poppedOut` seed) so the orphan-group loop does not
      // destroy the group dockview's own window-close path hands the panel back
      // to. Untracked ids (e.g. a pop-out that never went through this engine)
      // simply contribute nothing.
      for (const id of expanded.poppedOut) {
        const originGroupId = this.#poppedOutOriginGroups.get(id);
        if (originGroupId) seenGroupIds.add(originGroupId);
      }
      this.#zoneOfGroup.clear();

      for (const zone of ZONE_IDS) {
        const zoneNode = expanded.zones[zone];
        let previousGroupIdInZone: string | null = null;

        zoneNode.groups.forEach((groupNode, index) => {
          // Stage-relocation guard: a tree group whose tabs are entirely the
          // stage id (after filtering) has no real content to dock. Creating
          // it anyway would removePanel the LIVE stage panel out of its own
          // locked group to "move" it here; `#restoreStage` remounts it
          // synchronously, so the loop's own `addPanel("stage")` below would
          // then throw on a duplicate id and abort the whole `apply()`. Skip
          // the group entirely rather than let the tree relocate the stage.
          if (groupNode.tabs.every((t) => t === STAGE_ID)) return;

          const groupId = groupIdFor(zone, index, groupNode.tabs);
          seenGroupIds.add(groupId);
          this.#zoneOfGroup.set(groupId, { zone, index });

          let group = api.getGroup(groupId);
          if (!group) {
            group = previousGroupIdInZone
              ? api.addGroup({ id: groupId, referenceGroup: previousGroupIdInZone, direction: "below" })
              : api.addGroup({ id: groupId, referenceGroup: STAGE_GROUP_ID, direction: ZONE_EDGE_DIRECTION[zone] });
            this.#groupResizeSubs.set(
              groupId,
              group.api.onDidDimensionsChange(() => this.#handleGroupDimensionsChange(groupId)),
            );
            this.#groupWillDropSubs.set(
              groupId,
              group.model.onWillDrop((event) => this.#handleWillDrop(event)),
            );
          }
          previousGroupIdInZone = groupId;

          groupNode.tabs.forEach((tabId, tabIndex) => {
            // Same stage-relocation guard as above, per-tab: never let the tree
            // relocate the real stage panel into a zone group.
            if (tabId === STAGE_ID) return;
            seenPanelIds.add(tabId);
            const existing = api.getPanel(tabId);
            if (!existing) {
              api.addPanel({
                id: tabId,
                component: "sc-panel",
                position: { referenceGroup: groupId, direction: "within", index: tabIndex },
              });
            } else if (existing.group.id !== groupId) {
              // Cross-group move: remove + re-add under the same id. The
              // content renderer's `dispose` only detaches the adopted slot
              // (never destroys it), and the new renderer re-adopts the
              // SAME slot node — the mounted Svelte component instance
              // survives; only the dockview wrapper widget churns.
              api.removePanel(existing);
              api.addPanel({
                id: tabId,
                component: "sc-panel",
                position: { referenceGroup: groupId, direction: "within", index: tabIndex },
              });
            }
          });

          const activePanel = api.getPanel(groupNode.active);
          if (activePanel && group.model.activePanel?.id !== groupNode.active) {
            activePanel.api.setActive();
          }
        });
      }

      for (const f of expanded.floating) {
        seenPanelIds.add(f.id);
        const existing = api.getPanel(f.id);
        // A panel already present but NOT in a floating group is transitioning
        // INTO floating (e.g. the menu's "Float" command on a docked tab) —
        // mirrors the zone loop's cross-group-move branch just above: remove
        // + re-add under the same id rather than leaving it stranded in its
        // old (now tree-orphaned) group. `location.type` is dockview's own
        // public discriminant for a group's placement (`'grid' | 'floating' |
        // 'popout' | 'edge'`), read off the panel's CURRENT group.
        if (!existing || existing.group.api.location.type !== "floating") {
          if (existing) {
            // Bracket the transient removal so `#handleDidRemovePanel` skips
            // `#teardownFloatingA11y` for it — see `#floatTransitionIds`'s
            // doc comment.
            this.#floatTransitionIds.add(f.id);
            try {
              api.removePanel(existing);
            } finally {
              this.#floatTransitionIds.delete(f.id);
            }
          }
          api.addPanel({
            id: f.id,
            component: "sc-panel",
            floating: { x: f.rect.x, y: f.rect.y, width: f.rect.w, height: f.rect.h },
          });
          this.#wireFloatingA11y(f.id, meta.get(f.id));
        }
        // Snapshot the tree's own rect as the baseline `#handleFloatingLayoutChange`
        // diffs against, whether this iteration created/relocated the panel or
        // left an already-floating one untouched — see that method's doc comment
        // for why this snapshot (not `#applying`) is what suppresses self-caused
        // churn from a `resizeFloating` op's own round trip back through `apply()`.
        this.#lastFloatingRect.set(f.id, f.rect);
      }

      // Panels/groups no longer named by the tree are removed. Minimized
      // ids are never dockview panels at all — PanelHost relocates their
      // slot to its own staging container directly, independent of this
      // adapter (see the `EngineAdapter` doc comment).
      for (const panel of [...api.panels]) {
        if (!seenPanelIds.has(panel.id)) api.removePanel(panel);
      }
      for (const group of [...api.groups]) {
        if (group.id !== STAGE_GROUP_ID && !seenGroupIds.has(group.id) && group.model.panels.length === 0) {
          api.removeGroup(group);
          this.#groupResizeSubs.get(group.id)?.dispose();
          this.#groupResizeSubs.delete(group.id);
          this.#groupWillDropSubs.get(group.id)?.dispose();
          this.#groupWillDropSubs.delete(group.id);
          this.#lastGroupPx.delete(group.id);
        }
      }
    } finally {
      this.#applying = false;
    }
  }

  /** Translates a managed group's live
   * `onDidDimensionsChange` into `resizeZone`/`resizeGroup` ops. Guarded by
   * `#applying` — dockview's own layout pass fires this event while `apply()`
   * itself is adding/removing groups, and that churn is our own reconciliation,
   * not a user drag (same reasoning as `#handleDidRemovePanel`).
   *
   * Every managed zone stacks its groups vertically (`apply()` always joins
   * a same-zone sibling with `direction: "below"`), so the STACKING axis is
   * always a group's HEIGHT regardless of zone id — `resizeGroup.size` is
   * therefore always `group.height / Σ(zone's groups' heights)`. The zone's
   * own FACING dimension differs by zone id: right/left zones are columns of
   * fixed WIDTH (the axis perpendicular to the stack), while the bottom
   * zone's facing dimension is its stacked groups' total HEIGHT (the axis
   * the stack grows along). Both read the group/zone dimensions live off
   * the engine — `#zoneOfGroup` only tracks which zone/index each group
   * belongs to, not stale size snapshots.
   * @param groupId The dockview group id whose dimensions just changed.
   * @example
   * ```
   * // private method; not part of the public API — invoked only by the
   * // per-group onDidDimensionsChange subscription apply() creates
   * this.#handleGroupDimensionsChange("sc-group:chat");
   * ```
   */
  #handleGroupDimensionsChange(groupId: string): void {
    if (this.#applying) return;
    const api = this.#api;
    if (!api) return;
    const zoneInfo = this.#zoneOfGroup.get(groupId);
    if (!zoneInfo) return; // subscription outlived this group's zone membership; removal disposes it, but a same-tick race is defended here too
    const group = api.getGroup(groupId);
    if (!group) return;

    let sumHeights = 0;
    for (const [gid, info] of this.#zoneOfGroup) {
      if (info.zone !== zoneInfo.zone) continue;
      const sibling = api.getGroup(gid);
      if (sibling) sumHeights += sibling.api.height;
    }
    if (sumHeights <= 0) return; // no real dimensions yet (e.g. pre-layout) — nothing sane to emit

    const zonePx = zoneInfo.zone === "bottom" ? sumHeights : group.api.width;
    const lastZonePx = this.#lastZonePx.get(zoneInfo.zone);
    if (lastZonePx === undefined || Math.abs(lastZonePx - zonePx) >= 1) {
      this.#lastZonePx.set(zoneInfo.zone, zonePx);
      for (const cb of this.#opListeners) cb({ op: "resizeZone", zone: zoneInfo.zone, size: zonePx });
    }

    const lastGroupPx = this.#lastGroupPx.get(groupId);
    if (lastGroupPx === undefined || Math.abs(lastGroupPx - group.api.height) >= 1) {
      this.#lastGroupPx.set(groupId, group.api.height);
      const fraction = Math.min(1, Math.max(Number.EPSILON, group.api.height / sumHeights));
      for (const cb of this.#opListeners) {
        cb({ op: "resizeGroup", zone: zoneInfo.zone, group: zoneInfo.index, size: fraction });
      }
    }
  }

  /** Translates a live re-drag or re-resize of an ALREADY-floating panel
   * into a `resizeFloating` op, mirroring `#handleGroupDimensionsChange`'s role
   * for docked zones. Bound to `DockviewApi.onDidLayoutChange` rather than a
   * per-panel `onDidDimensionsChange` subscription, for two reasons found by
   * tracing the vendored source (`Overlay`, `FloatingGroupService.add`):
   * a floating group's `onDidDimensionsChange` only ever carries width/height
   * (`PanelApiImpl`'s `_onDidDimensionChange`), so a pure re-POSITION drag with
   * no size change never fires it at all; `onDidLayoutChange` is what
   * `Overlay#onDidChangeEnd` (fired once per completed drag OR resize gesture,
   * not per pointermove) actually feeds, via `FloatingGroupService.add`'s
   * `overlay.onDidChangeEnd(() => host.fireLayoutChange())`.
   *
   * Deliberately NOT gated by `#applying`, unlike every other handler in this
   * class: `DockviewApi.onDidLayoutChange` is dockview's own `AsapEvent`,
   * which defers every listener to the NEXT microtask via
   * `queueMicrotask` — by the time this fires, `apply()`'s synchronous
   * `finally { this.#applying = false }` has already run, so `#applying` would
   * always read `false` here regardless of cause and provide no real
   * protection (a false sense of one is worse than none). Self-caused churn is
   * instead suppressed by diffing against `#lastFloatingRect`, which `apply()`'s
   * floating loop snapshots to the TREE's own rect on every reconcile: a
   * `resizeFloating` op's round trip back through the reducer and into a later
   * `apply()` call re-snapshots the identical rect, so this handler's diff
   * against that same value reads as unchanged and emits nothing, with no
   * dependency on `apply()`'s synchronous window at all.
   * @example
   * ```
   * // private method; not part of the public API — invoked only by
   * // dockview-core's own onDidLayoutChange event
   * this.#handleFloatingLayoutChange();
   * ```
   */
  #handleFloatingLayoutChange(): void {
    const api = this.#api;
    const expanded = this.#expanded;
    if (!api || !expanded) return;
    for (const f of expanded.floating) {
      const panel = api.getPanel(f.id);
      if (!panel || panel.group.api.location.type !== "floating") continue;
      const box = panel.group.api.boundingBox;
      if (!box) continue;
      const rect: Rect = {
        x: Math.round(box.left),
        y: Math.round(box.top),
        w: Math.round(box.width),
        h: Math.round(box.height),
      };
      const last = this.#lastFloatingRect.get(f.id);
      if (
        last &&
        Math.abs(last.x - rect.x) < 1 &&
        Math.abs(last.y - rect.y) < 1 &&
        Math.abs(last.w - rect.w) < 1 &&
        Math.abs(last.h - rect.h) < 1
      ) {
        continue;
      }
      this.#lastFloatingRect.set(f.id, rect);
      for (const cb of this.#opListeners) cb({ op: "resizeFloating", id: f.id, rect });
    }
  }

  /** Test helper: the underlying dockview API, for driving/asserting engine
   * internals directly (e.g. a `#restoreStage` guard test calls `debugApi.removePanel`
   * on the stage panel the way an external bug or a future dockview version
   * might). Never used by production callers — the `EngineAdapter` seam
   * above never reaches for it.
   * @returns The live `DockviewApi`, or `null` before `init()` has run.
   * @example
   * ```ts
   * import { DockviewEngine } from "@shadowcat/module-panels";
   *
   * const engine = new DockviewEngine();
   * const api = engine.debugApi; // test-only; production code never reads this
   * ```
   */
  get debugApi(): DockviewApi | null {
    return this.#api;
  }

  /** Test helper: read-only view of `#poppedOutGroupPanels`, so a test can
   * assert `#handleRemovePopoutGroup` actually clears the tracked entry for a
   * closed popout group rather than merely inferring it from emitted ops.
   * Never used by production callers.
   * @returns A read-only map of popout dockview group id to the panel ids it
   * hosts.
   * @example
   * ```ts
   * import { DockviewEngine } from "@shadowcat/module-panels";
   *
   * const engine = new DockviewEngine();
   * const map = engine.debugPoppedOutGroupPanels; // test-only
   * ```
   */
  get debugPoppedOutGroupPanels(): ReadonlyMap<string, string[]> {
    return this.#poppedOutGroupPanels;
  }

  /** Test helper: read-only view of `#poppedOutOriginGroups`, mirroring
   * `debugPoppedOutGroupPanels` above.
   * @returns A read-only map of popped-out panel id to its pre-pop-out origin
   * dockview group id.
   * @example
   * ```ts
   * import { DockviewEngine } from "@shadowcat/module-panels";
   *
   * const engine = new DockviewEngine();
   * const map = engine.debugPoppedOutOriginGroups; // test-only
   * ```
   */
  get debugPoppedOutOriginGroups(): ReadonlyMap<string, string> {
    return this.#poppedOutOriginGroups;
  }

  /** `EngineAdapter.onOp`: subscribes to every `LayoutOp` this engine emits.
   * Seven sources, all funneled through this same `#opListeners` channel: a
   * drag gesture (`#handleWillDrop`/`#handleGroupWillDrop`); a menu command
   * (`#handleMenuCommand`, including `#requestPopOut`'s async continuation);
   * a resize (`#handleGroupDimensionsChange`/`#handleFloatingLayoutChange`);
   * a popout window closing (`#handleRemovePopoutGroup`); a panel removal
   * (`#handleDidRemovePanel`, a `close` op); a floating dialog's own
   * Escape keydown handler (wired in `#wireFloatingA11y`, also a `close` op);
   * and a user-driven tab activation (`#handleActivePanelChange`, an
   * `activeTab` op).
   * @param cb Called once per emitted op.
   * @returns An unsubscribe function.
   * @example
   * ```ts
   * import { DockviewEngine, type LayoutOp } from "@shadowcat/module-panels";
   *
   * const engine = new DockviewEngine();
   * const unsubscribe = engine.onOp((op: LayoutOp) => console.log(op));
   * unsubscribe();
   * ```
   */
  onOp(cb: (op: LayoutOp) => void): () => void {
    this.#opListeners.add(cb);
    return () => this.#opListeners.delete(cb);
  }

  /** `EngineAdapter.onNotice`: subscribes to this engine's user-facing notice
   * keys — today, only `"panels.popoutBlocked"`, emitted from `#requestPopOut`
   * when a pop-out request is blocked or fails.
   * @param cb Called once per emitted notice key.
   * @returns An unsubscribe function.
   * @example
   * ```ts
   * import { DockviewEngine } from "@shadowcat/module-panels";
   *
   * const engine = new DockviewEngine();
   * const unsubscribe = engine.onNotice((key: string) => console.log(key));
   * unsubscribe();
   * ```
   */
  onNotice(cb: (key: string) => void): () => void {
    this.#noticeListeners.add(cb);
    return () => this.#noticeListeners.delete(cb);
  }

  /** Notifies every `onNotice` subscriber with `key`.
   * @param key The notice's stable i18n key.
   * @example
   * ```
   * // private method; not part of the public API — invoked only from
   * // #requestPopOut's blocked/failed branches
   * this.#emitNotice("panels.popoutBlocked");
   * ```
   */
  #emitNotice(key: string): void {
    for (const cb of this.#noticeListeners) cb(key);
  }

  /** `EngineAdapter.focus`: brings `id`'s dockview group/window to the
   * foreground via `setActive()`. No-ops for the stage id (defense-in-depth
   * — the stage is never a normal focus subject) or an id with no live panel.
   * @param id The panel id to focus.
   * @example
   * ```ts
   * import { DockviewEngine } from "@shadowcat/module-panels";
   *
   * const engine = new DockviewEngine();
   * engine.focus("chat");
   * ```
   */
  focus(id: string): void {
    if (id === STAGE_ID) return; // defense-in-depth: never a normal focus subject
    this.#api?.getPanel(id)?.api.setActive();
  }

  /** `EngineAdapter.destroy`: disposes every subscription this engine created
   * (component-level, per-group resize/drop, per-floating-panel Escape),
   * clears every internal tracking map, and disposes the underlying
   * `DockviewApi` itself. Slot elements are NOT destroyed — ownership returns
   * to `PanelHost`'s staging container, per the `EngineAdapter` contract.
   * @example
   * ```ts
   * import { DockviewEngine } from "@shadowcat/module-panels";
   *
   * const engine = new DockviewEngine();
   * engine.destroy();
   * ```
   */
  destroy(): void {
    for (const d of this.#disposables) d.dispose();
    this.#disposables = [];
    for (const d of this.#groupResizeSubs.values()) d.dispose();
    this.#groupResizeSubs.clear();
    for (const d of this.#groupWillDropSubs.values()) d.dispose();
    this.#groupWillDropSubs.clear();
    for (const dispose of this.#floatingEscapeSubs.values()) dispose();
    this.#floatingEscapeSubs.clear();
    this.#floatInvokers.clear();
    this.#lastZonePx.clear();
    this.#lastGroupPx.clear();
    this.#lastFloatingRect.clear();
    this.#poppedOutGroupPanels.clear();
    this.#pendingPopouts.clear();
    this.#poppedOutOriginGroups.clear();
    this.#noticeListeners.clear();
    this.#api?.dispose();
    this.#api = null;
    this.#expanded = null;
    this.#zoneOfGroup.clear();
    this.#opListeners.clear();
  }
}

// TODO: `#toDropSite`'s target-group-outside-our-zone-bookkeeping fallback is
// a best-effort approximation (falls back to an edge-zone dock). The
// intercept-and-redispatch translation mechanism itself (preventDefault +
// emit + reconcile through `apply()`) is exercised directly by unit tests, not
// approximated — the residual manual-QA item narrows to drop-position
// classification fidelity for real pointer geometry (edge vs center vs
// tab-strip index resolution against an actual drag gesture, which jsdom
// cannot simulate) before shipping.
// TODO: `#expandGroupDockOp`'s "new group" index computation assumes the
// dragged group is not already a member of the target zone — a same-zone
// whole-group reorder is a residual approximation (see that method's doc
// comment), same class of limitation as the target-group-outside-our-zone
// fallback above.
