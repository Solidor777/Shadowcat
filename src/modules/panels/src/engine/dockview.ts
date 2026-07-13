// The ONLY file in this codebase permitted to import `dockview-core`. Every
// dockview type/event crosses out of this module already translated into our
// own vocabulary (`LayoutOp`, `DropSite`) — `policy.ts` and everything above
// the `EngineAdapter` seam stays engine-free.
import { createDockview } from "dockview-core";
import type {
  CreateComponentOptions,
  DockviewApi,
  DockviewActivePanelChangeEvent,
  DockviewDidDropEvent,
  DockviewWillDropEvent,
  IDockviewGroupPanel,
  IContentRenderer,
  IDockviewPanel,
} from "dockview-core";
import { consoleLogger, type Logger, type PanelMeta, type ZoneId } from "@shadowcat/core";
import type { EngineAdapter } from "./adapter";
import type { ExpandedLayout, LayoutOp } from "../layout/tree";
import { classifyDrop, STAGE_ID, type DropSite } from "./policy";
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
  // Last EMITTED px dimensions per zone/group, to skip dockview's frequent
  // sub-pixel dimension churn (every layout pass fires this event, not just
  // a user's splitter drag) — avoids feedback-loop op spam.
  #lastZonePx = new Map<ZoneId, number>();
  #lastGroupPx = new Map<string, number>();

  constructor(logger?: Logger) {
    this.#logger = logger ?? consoleLogger();
  }

  init(host: HTMLElement, slotFor: (id: string) => HTMLElement, stageEl: HTMLElement): void {
    host.classList.add("sc-dockview-root");

    const api = createDockview(host, {
      createComponent: (options: CreateComponentOptions) =>
        options.name === "sc-stage"
          ? new AdoptingContentRenderer(() => stageEl, "sc-dockview-stage-content")
          : new AdoptingContentRenderer(() => slotFor(options.id), "sc-dockview-panel-content"),
    });
    this.#api = api;

    this.#mountStage(api);

    this.#disposables.push(
      api.onWillDrop((event) => this.#handleWillDrop(event)),
      api.onDidDrop((event) => this.#handleDidDrop(event)),
      api.onDidRemovePanel((panel) => this.#handleDidRemovePanel(panel)),
      api.onDidActivePanelChange((event) => this.#handleActivePanelChange(event)),
    );
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

  /** W2: the drop veto. `classifyDrop` is pure/engine-free — this is the
   * ONLY place a dockview drag event is translated into `DropSite` and fed
   * to it. A veto calls `event.preventDefault()`, which dockview honours
   * before performing the actual DOM move (`DockviewWillDropEvent` fires
   * before the model applies anything and checks `defaultPrevented`).
   * Fails CLOSED on anything this translation layer cannot classify: a null
   * layout (pre-first-`apply()`) or a payload `#toDropSite` cannot resolve
   * into a `DropSite` (notably a whole-GROUP transfer — `PanelTransfer`'s
   * `panelId` is null for a titlebar drag of an entire group, per
   * `groupDragSource.ts`) are vetoed outright rather than let through
   * unpoliced. A whole-group drop targeting the container's TOP edge would
   * otherwise land ABOVE the stage (W1/D4 violation) since `classifyDrop`
   * never runs against it; vetoing the whole gesture class in v1 also means
   * a completed group drop never needs a `LayoutOp` translation (see
   * `#handleDidDrop`). */
  #handleWillDrop(event: DockviewWillDropEvent): void {
    const layout = this.#expanded;
    if (!layout) {
      event.preventDefault();
      return;
    }
    const site = this.#toDropSite(event, event.kind);
    if (!site) {
      // Whole-group transfers and any other unclassifiable payload shape —
      // vetoed rather than silently let through (see doc comment above).
      event.preventDefault();
      this.#logger.warn("panels: vetoed drop (unclassifiable payload, e.g. a whole-group transfer)");
      return;
    }
    const result = classifyDrop(site, layout);
    if ("veto" in result) {
      event.preventDefault();
      this.#logger.warn(`panels: vetoed drop (${result.reason})`, site);
    }
  }

  /** Translates a completed drop into the `LayoutOp` the reducer needs to
   * keep our tree in sync with what dockview just did to its own DOM. A
   * completed drop necessarily already passed `#handleWillDrop`'s veto, so
   * `classifyDrop` re-running here should never veto in practice — if it
   * somehow does (a translation edge case: `DockviewDidDropEvent` carries no
   * `kind`, unlike `DockviewWillDropEvent`, so it is approximated below),
   * the op is dropped and logged rather than risking a stage-well-violating
   * op reaching the reducer. */
  #handleDidDrop(event: DockviewDidDropEvent): void {
    if (this.#applying) return;
    const layout = this.#expanded;
    // A real completed drop can never reach here with a null layout:
    // `#handleWillDrop` now `preventDefault()`s every drop in that window
    // (see its doc comment), so dockview never lets one complete. Kept as a
    // defense-in-depth bail, not a reachable production path.
    if (!layout) return;
    const site = this.#toDropSite(event, undefined);
    if (!site) return;
    const result = classifyDrop(site, layout);
    if ("veto" in result) {
      this.#logger.warn(`panels: a completed drop reclassified as a veto (${result.reason}); ignoring`, site);
      return;
    }
    for (const cb of this.#opListeners) cb(result);
  }

  /** Shared translator for both `onWillDrop` (has a real `kind`) and
   * `onDidDrop` (does not — `kind` is passed as `undefined` and approximated
   * from `position`/`group` below). */
  #toDropSite(
    event: DockviewWillDropEvent | DockviewDidDropEvent,
    kind: DockviewWillDropEvent["kind"] | undefined,
  ): DropSite | null {
    const data = event.getData();
    const id = data?.panelId;
    // Whole-group drags (`PanelTransfer.panelId === null`, a titlebar drag of
    // an entire group) carry no single subject id — `classifyDrop` has no
    // vocabulary for a group-as-subject, so this returns null. The caller
    // (`#handleWillDrop`) vetoes every null-site result outright; this is
    // NOT "unpoliced", just policed one level up.
    if (!id) return null;
    const targetGroupId = event.group?.id;
    const stageGroup = targetGroupId === STAGE_GROUP_ID;

    if (kind === "edge" || (kind === undefined && !event.group)) {
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

  apply(expanded: ExpandedLayout, _meta: ReadonlyMap<string, PanelMeta>): void {
    const api = this.#api;
    if (!api) return;
    this.#expanded = expanded;
    this.#applying = true;
    try {
      const seenPanelIds = new Set<string>([STAGE_ID]);
      const seenGroupIds = new Set<string>([STAGE_GROUP_ID]);
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
        if (!api.getPanel(f.id)) {
          api.addPanel({
            id: f.id,
            component: "sc-panel",
            floating: { x: f.rect.x, y: f.rect.y, width: f.rect.w, height: f.rect.h },
          });
        }
        // Position/size sync of an ALREADY-floating panel is deferred — see
        // TODO below.
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

  /** Test helper: the underlying dockview API, for driving/asserting engine
   * internals directly (e.g. the W3 guard test calls `debugApi.removePanel`
   * on the stage panel the way an external bug or a future dockview version
   * might). Never used by production callers — the `EngineAdapter` seam
   * above never reaches for it. */
  get debugApi(): DockviewApi | null {
    return this.#api;
  }

  onOp(cb: (op: LayoutOp) => void): () => void {
    this.#opListeners.add(cb);
    return () => this.#opListeners.delete(cb);
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
    this.#lastZonePx.clear();
    this.#lastGroupPx.clear();
    this.#api?.dispose();
    this.#api = null;
    this.#expanded = null;
    this.#zoneOfGroup.clear();
    this.#opListeners.clear();
  }
}

// TODO: floating-panel position/size sync in `apply()` for an
// ALREADY-floating panel (creation is handled; live re-drag/resize of an
// existing floating window is not yet mirrored back into the tree).
// TODO: `#toDropSite`'s fallback branches (target group outside our zone
// bookkeeping; `onDidDrop`'s missing `kind`) are best-effort approximations,
// not exhaustively verified against every dockview drag path — recommend a
// manual browser QA pass over live drag-and-drop before shipping.
// Whole-GROUP drag transfers (`PanelTransfer.panelId === null`) are vetoed
// outright in v1 (see `#handleWillDrop`'s doc comment).
// TODO: Translate whole-group transfers into per-tab dock ops to re-enable
// the group-drag gesture.
