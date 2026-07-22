// The ONLY file in this codebase permitted to import `dockview-core`. Every
// dockview type/event crosses out of this module already translated into our
// own vocabulary (`LayoutOp`, `DropSite`) — `policy.ts` and everything above
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

/** The dockview group id reserved for the stage (W1). Never collides with a
 * zone-group id, which is always derived from a panel id via `groupIdFor`
 * and namespaced under a different prefix. */
const STAGE_GROUP_ID = "sc-stage-group";

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
  readonly element: HTMLElement;
  #adopted: HTMLElement | null = null;

  constructor(
    private readonly resolve: () => HTMLElement,
    className: string,
  ) {
    this.element = document.createElement("div");
    this.element.className = className;
    this.element.style.height = "100%";
  }

  init(): void {
    if (this.#adopted) return;
    this.#adopted = this.resolve();
    this.element.appendChild(this.#adopted);
  }

  layout(): void {}
  update(): void {}
  toJSON(): object {
    return {};
  }
  focus(): void {}

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
 * otherwise shift every later index (mirrors `tree.ts`'s `dock` op, which
 * resolves a target group's identity before `detach` runs for the same
 * reason). A reordered or emptied-then-refilled group gets a fresh id and
 * is recreated — accepted as a minor churn cost; a finer content-independent
 * diff is future work. */
function groupIdFor(zone: ZoneId, index: number, tabs: readonly string[]): string {
  return `sc-group:${tabs[0] ?? `${zone}:${index}:empty`}`;
}

/** Mounts `PanelMenu` into a small popover positioned under `anchor` (the tab's
 * menu button); returns a `close()` that unmounts it and restores focus to
 * `anchor`. A pointerdown outside the popover, or the menu's own `onClose`
 * (Escape/Tab), closes it — the two paths share this single teardown so
 * neither can leave the other's listener/DOM behind. */
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
 * (`Tab#_element` in `tab.js`, `role="tab"`), which already implements APG
 * roving tabindex (arrow keys move that wrapper's focus without activating;
 * Enter/Space activates) and is not reimplemented here. `getMeta` is read
 * lazily (not snapshotted at construction) so a later `apply()` call's meta
 * map is always current; an `i18n.subscribe` re-renders the label on locale
 * change. */
class PanelTabRenderer implements ITabRenderer {
  readonly element: HTMLElement;
  #iconEl: HTMLElement;
  #labelEl: HTMLElement;
  #menuBtn: HTMLButtonElement;
  #unsubLocale: () => void;
  #closeMenu: (() => void) | null = null;

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
    this.#menuBtn = document.createElement("button");
    this.#menuBtn.type = "button";
    this.#menuBtn.className = "sc-tab-menu-btn";
    this.#menuBtn.setAttribute("aria-haspopup", "menu");
    this.#menuBtn.setAttribute("aria-expanded", "false");
    this.#menuBtn.textContent = "⋮";
    this.element.append(this.#iconEl, this.#labelEl, this.#menuBtn);
    this.#renderLabels();
    this.#unsubLocale = i18n.subscribe(() => this.#renderLabels());
    this.#menuBtn.addEventListener("click", (event) => {
      // Never let a menu-button click bubble into the tab wrapper's own
      // click handling (`Tab#_onTabClick`), which would also activate this
      // tab as a side effect of opening its menu.
      event.stopPropagation();
      this.#toggleMenu();
    });
  }

  // No per-panel state to read from dockview's own init parameters — label/
  // icon come entirely from `getMeta()`, never `params.title`.
  init(_params: TabPartInitParameters): void {}

  #renderLabels(): void {
    const meta = this.getMeta();
    this.#iconEl.textContent = meta?.icon ?? "";
    this.#labelEl.textContent = meta ? i18n.t(meta.labelKey) : this.id;
    this.#menuBtn.setAttribute("aria-label", i18n.t("panels.menu"));
  }

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

  dispose(): void {
    this.#closeMenu?.();
    this.#unsubLocale();
  }
}

/** Gesture contract: classify → veto or redispatch; dockview never self-mutates
 * from a drop. Every will-drop wire (`#handleWillDrop`, fed by both the
 * component-level `api.onWillDrop` and the per-group `group.model.onWillDrop`
 * — see `#groupWillDropSubs`) either vetoes (`preventDefault()`, no further
 * action) or, for an ALLOWED classification, ALSO `preventDefault()`s and
 * instead emits the classified `LayoutOp` to `#opListeners`. dockview's own
 * internal move machinery (`_onMove` → `DockviewComponent#moveGroupOrPanel`,
 * `dockviewGroupPanelModel.ts:1804`) is consequently never reached for a
 * completed same-instance drag: the tree is canonical, and the controller's
 * `apply()` — driven by the op this class just emitted — is the one
 * sanctioned mutation path back into dockview. `onDidDrop` (fired only when
 * a drop's `PanelTransfer.viewId` does not match this instance's own
 * `accessor.id` — i.e. a drag whose data never originated from THIS
 * `DockviewApi`, `dockviewGroupPanelModel.ts:1747-1821`) has no wiring here:
 * this codebase creates exactly one `DockviewApi` per `PanelHost`, and a
 * popped-out group still shares that same instance/`accessor.id` (pop-out
 * moves a panel between groups of ONE api, it never spins up a second one),
 * so no drag reaching this class can ever satisfy that mismatch — the event
 * is unreachable for a real user gesture, not merely unhandled. */
export class DockviewEngine implements EngineAdapter {
  #api: DockviewApi | null = null;
  #opListeners = new Set<(op: LayoutOp) => void>();
  #disposables: { dispose(): void }[] = [];
  #logger: Logger;
  #expanded: ExpandedLayout | null = null;
  // dockview group id -> our (zone, index) for that group, as of the last
  // `apply()`. Used to translate a drop event's target group back into our
  // vocabulary; rebuilt fresh on every `apply()` call.
  #zoneOfGroup = new Map<string, { zone: ZoneId; index: number }>();
  // True for the synchronous duration of `apply()` — panel/group removals
  // dockview fires WHILE we are diffing are OUR OWN reconciliation, not a
  // user gesture, so `onDidRemovePanel`/`onDidActivePanelChange` must not
  // re-emit them as ops (that would fight the reducer that just drove this
  // very `apply()` call).
  #applying = false;
  // Reentrancy guard for W3: `#restoreStage` itself adds a panel; without
  // this, the model's own remove/add bookkeeping could recurse back in.
  #restoringStage = false;
  // One live `onDidDimensionsChange` subscription per managed (non-stage)
  // group, keyed by dockview group id. Added the moment `apply()` creates a
  // group, disposed the moment `apply()` removes it (and on `destroy()`) —
  // a group's whole lifetime is bracketed by exactly one subscription.
  #groupResizeSubs = new Map<string, { dispose(): void }>();
  // One live `group.model.onWillDrop` subscription per managed (non-stage)
  // group — same add/dispose lifecycle as `#groupResizeSubs` above. Required
  // because `DockviewApi.onWillDrop` (subscribed once in `init()`, below)
  // NEVER fires for a drop targeting an existing group: the component only
  // forwards a group model's `onWillDrop` through `_advancedDnDService?.
  // dispatchWillDrop(event)` (`dockviewComponent.ts:4592-4594`), and this
  // codebase ships no `advancedDnDService` module (`allModules.ts:16-24`), so
  // that optional chain is a permanent no-op — `#handleWillDrop` is
  // otherwise unreachable for any group-target drop (header, tab, or
  // content). `IDockviewGroupPanelModel.onWillDrop` (`dockviewGroupPanelModel.
  // ts:186` interface, emitter at `:274-275`) is a public event on the SAME
  // `group.model` this class already reaches via `api.getGroup(groupId)`,
  // fires with the identical `DockviewWillDropEvent` shape as the
  // component-level event (`dockviewGroupPanelModel.ts:1731-1741`), and gates
  // `_onMove`/`_onDidDrop` on `defaultPrevented` exactly like the root path
  // (`handleDropEvent`, `dockviewGroupPanelModel.ts:1741-1811`) — so binding
  // `#handleWillDrop` to it directly closes the group-onto-group veto bypass
  // AND lets an ALLOWED group-target drop be intercepted-and-redispatched the
  // same way as a root/edge drop, no new method needed.
  #groupWillDropSubs = new Map<string, { dispose(): void }>();
  // Last EMITTED px dimensions per zone/group, to skip dockview's frequent
  // sub-pixel dimension churn (every layout pass fires this event, not just
  // a user's splitter drag) — avoids feedback-loop op spam.
  #lastZonePx = new Map<ZoneId, number>();
  #lastGroupPx = new Map<string, number>();
  // Meta snapshot from the last `apply()` call — read by `PanelTabRenderer`'s
  // `getMeta` closure (icon/labelKey are effectively static per panel once
  // registered, so a live reference here is sufficient; no per-render diffing).
  #meta: ReadonlyMap<string, PanelMeta> = new Map();
  // Invoking element (the menu button) for each floating panel created via a
  // "float" menu command, so `#teardownFloatingA11y` can return focus to it
  // on close. Absent for any future non-menu float path — that path simply
  // degrades to no focus-return, never a crash.
  #floatInvokers = new Map<string, HTMLElement>();
  // One live Escape-to-close keydown listener per floating panel's dialog
  // element, disposed the moment that panel leaves the model (mirrors
  // `#groupResizeSubs`'s per-group add/dispose lifecycle).
  #floatingEscapeSubs = new Map<string, () => void>();
  // Ids currently mid-transition from docked into floating WITHIN THE SAME
  // `apply()` call (the floating loop's remove+re-add-under-same-id branch,
  // mirroring the zone loop's cross-group-move pattern). `api.removePanel`
  // fires `onDidRemovePanel` synchronously — by the time that listener runs,
  // dockview has already detached the outgoing tab's DOM (confirmed: a
  // `document.contains` check inside the listener already reads false), so
  // `#teardownFloatingA11y` never gets to invoke `invoker.focus()` here
  // regardless of this guard. What it WOULD otherwise do, wrongly, is
  // `#floatInvokers.delete(id)` — discarding the very entry `apply()`'s
  // floating loop is about to hand to the panel's OWN new floating dialog a
  // few lines later. Gating the delete on this set is what actually fixes
  // Finding 1: the invoker entry now survives the transient churn intact,
  // available to a LATER, real close of the floating panel.
  #floatTransitionIds = new Set<string>();
  // Last rect (px, relative to the dockview root) `apply()` last placed a
  // floating panel at OR this class last emitted via `resizeFloating` —
  // whichever happened most recently. Read by `#handleFloatingLayoutChange`
  // to detect a real user-driven drift and to suppress re-emitting a rect
  // the tree already knows about. Deleted alongside `#floatingEscapeSubs`/
  // `#floatInvokers` in `#teardownFloatingA11y` (mirrors their per-id
  // add/dispose lifecycle) and cleared wholesale in `destroy()`.
  #lastFloatingRect = new Map<string, Rect>();
  #noticeListeners = new Set<(key: string) => void>();
  // Popout group id -> the panel ids it hosts, recorded when a pop-out succeeds
  // so `onDidRemovePopoutGroup` (window closed by the user) can translate a
  // group id back into `popIn` ops without depending on the group's live panel
  // membership at fire time (dockview's teardown may have already moved them).
  #poppedOutGroupPanels = new Map<string, string[]>();
  // Ids with a pop-out request currently in flight (driver called, promise not
  // yet settled). dockview's `addPopoutGroup` is wrapped in `mutation()`, whose
  // `finally` fires the instant the function RETURNS its pending promise — not
  // when it settles — so it serializes only the synchronous portion, leaving the
  // async `window.open` → re-parent gap unguarded. Without this set, a second
  // "Pop out" click on the SAME id inside that gap opens a second window and
  // re-keys `#poppedOutGroupPanels` to the first (now-orphaned) group id,
  // corrupting the tree on the eventual window close. Refuse a duplicate until
  // the first request settles.
  #pendingPopouts = new Set<string>();
  // Popped-out panel id -> the group id it lived in BEFORE pop-out (its ORIGIN
  // group). dockview keeps that group alive-but-hidden (`setVisible(false)`) and
  // its window-close path (`disposePopoutWindow`) hands the panel back to that
  // exact group object; the `popOut` op meanwhile detaches the group from the
  // persisted tree, so `apply()` seeds these ids into `seenGroupIds` to keep the
  // orphan-group loop from destroying the group dockview still depends on.
  #poppedOutOriginGroups = new Map<string, string>();
  // Gesture-time popout invoker. Defaults to dockview's native popout (verified
  // same-heap, content re-parented, stylesheets cloned — M12a-0 spike +
  // popoutWindow.js:136). Injectable so unit tests exercise the async-result →
  // op translation without a real `window.open` (jsdom has none).
  #popoutDriver: (panel: IDockviewPanel) => Promise<boolean>;

  constructor(logger?: Logger, popoutDriver?: (panel: IDockviewPanel) => Promise<boolean>) {
    this.#logger = logger ?? consoleLogger();
    // `/popout.html` is the same-origin loader document dockview's popout
    // window navigates to; passed explicitly to document the dependency
    // rather than relying on dockview's own default drifting.
    // `assertSameOriginPopoutUrl` (popoutWindow.js) rejects
    // `about:blank`/cross-origin, so this URL is load-bearing.
    this.#popoutDriver = popoutDriver ?? ((panel) => this.#api!.addPopoutGroup(panel, { popoutUrl: "/popout.html" }));
  }

  init(host: HTMLElement, slotFor: (id: string) => HTMLElement, stageEl: HTMLElement): void {
    host.classList.add("sc-dockview-root");

    const api = createDockview(host, {
      createComponent: (options: CreateComponentOptions) =>
        options.name === "sc-stage"
          ? new AdoptingContentRenderer(() => stageEl, "sc-dockview-stage-content")
          : new AdoptingContentRenderer(() => slotFor(options.id), "sc-dockview-panel-content"),
      // `DockviewPanelModel.createTabComponent` (dockviewPanelModel.js) only
      // calls `options.createTabComponent` at all when `componentName ??
      // defaultTabComponent` is truthy — no per-panel `tabComponent` is ever
      // set on `addPanel`, so this MUST be a truthy string or `createTabComponent`
      // below is never reached and every tab silently falls back to
      // dockview's own `DefaultTab`. The value itself is arbitrary; `options.id`
      // (not `options.name`, which is always this same string) is what the
      // factory below branches on.
      defaultTabComponent: "sc-tab",
      // The stage's own group is headerless (`hideHeader: true`, W1), so this
      // is never actually invoked for the stage id — the `undefined` fallback
      // (dockview's `DefaultTab`) is defense-in-depth only, matching `apply`/
      // `focus`'s belt-and-suspenders STAGE_ID guards elsewhere in this class.
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
   * (policy.ts, dockview-free) maps it to, and emits it through the SAME
   * `#opListeners` channel a drag gesture uses — the controller cannot tell
   * the two apart, and per the parity requirement, it doesn't need to: both
   * paths produce identical `LayoutOp` shapes for the equivalent move.
   * `float` additionally records `invoker` (the tab's own menu button,
   * handed in by `PanelTabRenderer`) as this panel's focus-return target
   * (see `#floatInvokers`). Refuses `STAGE_ID` in TWO independent layers:
   * the early return below (mirrors `focus()`'s existing W2 guard) and
   * `opForMenuCommand`'s own veto (mirrors `classifyDrop`'s stage veto) —
   * belt-and-suspenders alongside `createTabComponent`'s `STAGE_ID` branch
   * (`init()`) and W1's headerless stage group, which never gives the stage
   * a `PanelTabRenderer`/menu button to invoke this with in the first place;
   * neither guard here is the sole line of defense. */
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
   * group for close-translation). Block/throw ⇒ spec §10 fallback: `float` +
   * a `panels.popoutBlocked` notice. */
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

  /** W1: mounts the stage into its own dedicated group — headerless (no tab
   * strip, so no close/drag affordance exists at all: `hideHeader: true`
   * sets `header.hidden = true`, and `header` IS the group's `TabsContainer`
   * instance, whose `hidden` setter sets the whole tabs-and-actions
   * element's `display: none`) and locked to `'no-drop-target'` (the
   * model's own drop handler returns before a drop event is even
   * constructed against a group locked this way). Also used by
   * `#restoreStage` (W3) to remount after an unexpected removal. */
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

  /** W3: fail-safe invariant guard. If the stage panel ever leaves the
   * model (a bug elsewhere, a dockview behaviour change, or a wrapper-API
   * gap), remount it immediately and log — the stage must never simply
   * vanish. */
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

  /** W1/D4: floating groups are non-modal dialogs (dockview's own `Overlay`
   * sets `role="dialog"`/`aria-modal="false"` — see `overlay.js`); this adds
   * the label + focus management the brief requires that dockview doesn't
   * supply itself: `aria-label` = the panel's own label, DOM focus moves
   * into the dialog the moment it appears, and Escape (bubbled from
   * anywhere inside it) closes it via the same op channel a menu/drag
   * gesture uses. Called once per floating-panel CREATION (`apply()`'s
   * floating loop) — `panel.group.element` is a descendant of the dialog's
   * `Overlay#_element`, so `.closest()` finds it without reaching into any
   * dockview-internal field. */
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
   * or a dock transition moving the panel out of its floating window). */
  #teardownFloatingA11y(id: string): void {
    this.#floatingEscapeSubs.get(id)?.();
    this.#floatingEscapeSubs.delete(id);
    const invoker = this.#floatInvokers.get(id);
    this.#floatInvokers.delete(id);
    this.#lastFloatingRect.delete(id);
    if (invoker && document.contains(invoker)) invoker.focus();
  }

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
   * `#handleActivePanelChange`), so re-emitting would replay a stale op. */
  #handleRemovePopoutGroup(event: { id: string; group: IDockviewGroupPanel }): void {
    const ids = this.#poppedOutGroupPanels.get(event.id) ?? event.group.model.panels.map((p) => p.id);
    this.#poppedOutGroupPanels.delete(event.id);
    // Origin-group tracking clears unconditionally (like `#poppedOutGroupPanels`
    // above), for both a user window-close and our own `apply()`-driven pop-in:
    // once the panel leaves the popped-out state its origin group is a normal
    // tree-owned group again.
    for (const id of ids) this.#poppedOutOriginGroups.delete(id);
    if (this.#applying) return;
    for (const id of ids) {
      if (id === STAGE_ID) continue;
      for (const cb of this.#opListeners) cb({ op: "popIn", id });
    }
  }

  /** W2 + intercept-and-redispatch: the ONLY place a dockview drag event is
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
   * an entire group, per `groupDragSource.ts`) is translated into one `dock`
   * op per tab of the dragged group (`#handleGroupWillDrop`), classified
   * against the group's FIRST tab as a representative subject — the same
   * `classifyDrop` rules apply, so a whole-group drop targeting the
   * container's TOP edge still vetoes (no "top" `ZoneId` exists, W1/D4) exactly
   * as a single-tab edge drop would.
   *
   * Two independent wires feed this SAME method: `init()`'s `api.onWillDrop`
   * (fires only for root/edge drops — `dockviewComponent.ts`'s
   * `rootDropTarget.onWillShowOverlay`/`onDrop` wiring is the only path that
   * calls `this._onWillDrop.fire(...)` directly) and, per managed group,
   * `group.model.onWillDrop` (subscribed in `apply()` — see
   * `#groupWillDropSubs`'s doc comment for why the component-level event
   * alone does NOT cover group-target drops). Together they cover every
   * drop this engine can receive: container-edge drops via the component
   * path, group-onto-group drops via the per-group path — and since BOTH
   * now `preventDefault()` every allowed drop too, dockview's own `_onMove`/
   * internal move machinery is never reached from either wire. */
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
   * `groupId` = the SOURCE group's own dockview id, per `groupDragSource.ts`/
   * `tabGroups.js`) into one `dock` `LayoutOp` per tab of the dragged group,
   * preserving their relative order, with a SINGLE `preventDefault()` for the
   * whole transfer. Classifies against the group's FIRST tab as a
   * representative subject — every tab in the dragged group moves to the
   * SAME target, so one `classifyDrop` call settles the target zone/group/
   * position for all of them (`#expandGroupDockOp` fans that single result
   * out per-tab). Fails closed (veto, single `preventDefault()`) when the
   * source group can't be resolved (empty tab list — e.g. a stale/foreign
   * group id) or the representative classification itself vetoes, exactly
   * like the single-tab path in `#handleWillDrop`. */
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
   * tab count, when the drop was a "content" drop with no explicit index). */
  #expandGroupDockOp(
    op: Extract<LayoutOp, { op: "dock" }>,
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
   * approximation branch is needed here anymore. */
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
          // W3 hardening: a tree group whose tabs are entirely the stage id
          // (after filtering) has no real content to dock. Creating it
          // anyway would removePanel the LIVE stage panel out of its own
          // locked group to "move" it here; W3's `#restoreStage` remounts it
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
            // Same W3 hardening as above, per-tab: never let the tree
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

  /** Finding 3 (buddy-check): translates a managed group's live
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
   * belongs to, not stale size snapshots. */
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

  /** F3: translates a live re-drag or re-resize of an ALREADY-floating panel
   * into a `resizeFloating` op, mirroring `#handleGroupDimensionsChange`'s role
   * for docked zones. Bound to `DockviewApi.onDidLayoutChange` rather than a
   * per-panel `onDidDimensionsChange` subscription, for two reasons found by
   * tracing the vendored source (`overlay.js`, `floatingGroupService.js`):
   * a floating group's `onDidDimensionsChange` only ever carries width/height
   * (`panelApi.js`'s `_onDidDimensionChange`), so a pure re-POSITION drag with
   * no size change never fires it at all; `onDidLayoutChange` is what
   * `Overlay#onDidChangeEnd` (fired once per completed drag OR resize gesture,
   * not per pointermove) actually feeds, via `floatingGroupService.js`'s
   * `overlay.onDidChangeEnd(() => host.fireLayoutChange())`.
   *
   * Deliberately NOT gated by `#applying`, unlike every other handler in this
   * class: `DockviewApi.onDidLayoutChange` is dockview's own `AsapEvent`
   * (`events.js`), which defers every listener to the NEXT microtask via
   * `queueMicrotask` — by the time this fires, `apply()`'s synchronous
   * `finally { this.#applying = false }` has already run, so `#applying` would
   * always read `false` here regardless of cause and provide no real
   * protection (a false sense of one is worse than none). Self-caused churn is
   * instead suppressed by diffing against `#lastFloatingRect`, which `apply()`'s
   * floating loop snapshots to the TREE's own rect on every reconcile: a
   * `resizeFloating` op's round trip back through the reducer and into a later
   * `apply()` call re-snapshots the identical rect, so this handler's diff
   * against that same value reads as unchanged and emits nothing, with no
   * dependency on `apply()`'s synchronous window at all. */
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
   * internals directly (e.g. the W3 guard test calls `debugApi.removePanel`
   * on the stage panel the way an external bug or a future dockview version
   * might). Never used by production callers — the `EngineAdapter` seam
   * above never reaches for it. */
  get debugApi(): DockviewApi | null {
    return this.#api;
  }

  /** Test helper: read-only view of `#poppedOutGroupPanels`, so a test can
   * assert `#handleRemovePopoutGroup` actually clears the tracked entry for a
   * closed popout group rather than merely inferring it from emitted ops.
   * Never used by production callers. */
  get debugPoppedOutGroupPanels(): ReadonlyMap<string, string[]> {
    return this.#poppedOutGroupPanels;
  }

  /** Test helper: read-only view of `#poppedOutOriginGroups`, mirroring
   * `debugPoppedOutGroupPanels` above. Never used by production callers. */
  get debugPoppedOutOriginGroups(): ReadonlyMap<string, string> {
    return this.#poppedOutOriginGroups;
  }

  onOp(cb: (op: LayoutOp) => void): () => void {
    this.#opListeners.add(cb);
    return () => this.#opListeners.delete(cb);
  }

  onNotice(cb: (key: string) => void): () => void {
    this.#noticeListeners.add(cb);
    return () => this.#noticeListeners.delete(cb);
  }

  #emitNotice(key: string): void {
    for (const cb of this.#noticeListeners) cb(key);
  }

  focus(id: string): void {
    if (id === STAGE_ID) return; // W2 defense-in-depth: never a normal focus subject
    this.#api?.getPanel(id)?.api.setActive();
  }

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
