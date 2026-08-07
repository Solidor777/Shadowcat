// Pure layout tree + reducer for the panel-manager host. Engine-agnostic: no Svelte,
// no dockview, no ui-kit — a host component maps this state onto whatever docking widget
// it renders (dockview-core, in this codebase). All mutating functions return a NEW object;
// unchanged-input calls return the SAME reference so a host can cheaply skip a re-render.
import type { ZoneId, DefaultPlacement } from "@shadowcat/core";

/** A floating panel's position and size, in CSS px relative to the panel host's own
 * container (not the viewport). Carried on `floating` entries and on the `float`/
 * `resizeFloating` ops; the reducer never reads a viewport, so these stay whatever the
 * caller supplied. `x`/`y` may be negative (a panel legitimately dragged partway
 * off-screen); `w`/`h` are never negative (see `isRect`, which enforces
 * this on decode). */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** One tab strip within a zone. `size` is this group's fraction (0..1) of the zone. */
export interface GroupNode {
  tabs: string[];
  active: string;
  size: number;
}

/** A dock zone. `size` is the zone's own px basis (independent of its groups' fractions). */
export interface ZoneNode {
  groups: GroupNode[];
  size: number;
}

/** The "expanded" (non-compact) view's full layout state: the three dock zones, floating
 * panels, minimized panels, and popped-out ids. `locate` treats these four locations plus
 * "closed" as mutually exclusive and exhaustive for a given panel id (see `detach`'s own doc
 * comment). */
export interface ExpandedLayout {
  // All three ZoneId keys are always present, even with empty groups — callers never
  // guard a missing zone.
  zones: Record<ZoneId, ZoneNode>;
  floating: { id: string; rect: Rect; z: number }[];
  minimized: string[];
  // Ids currently rendered in a same-heap child window (dockview popout). PERSISTED
  // as ids only — the live `Window` handle is dockview's, never serialized. A page
  // load cannot reopen a popup (no user gesture), so a persisted entry rehydrates to
  // floating at controller construction; during a live session an id lands here only
  // after a gesture-time `addPopoutGroup` succeeds.
  poppedOut: string[];
}

/** The narrow-viewport (<48rem) view's layout state: a single-column `order` of panel ids
 * and which one is currently shown. Independent of `ExpandedLayout` — a panel's docked/
 * floating/minimized/popped-out location does not change when the host switches views. */
export interface CompactLayout {
  activeView: string | null;
  order: string[];
}

/** The persisted root: a fixed schema `version` tag (for future migrations) plus the
 * expanded and compact views' independent state. The sole value type every reducer
 * function (`applyOp`, `prune`, `placeNewRegistrations`) and the persistence codec
 * (`encodeLayout`/`decodeLayout`) operate on. */
export interface PanelLayoutV1 {
  version: 1;
  expanded: ExpandedLayout;
  compact: CompactLayout;
}

/** Every state change `applyOp` can make. A host never mutates `PanelLayoutV1` directly —
 * it translates a gesture (engine event, menu command, persisted rehydration) into one of
 * these and dispatches it through `applyOp`. */
export type LayoutOp =
  | { op: "open"; id: string; placement?: DefaultPlacement }
  | { op: "close"; id: string }
  | { op: "dock"; id: string; zone: ZoneId; group: number | "new"; tabIndex?: number }
  | { op: "float"; id: string; rect: Rect }
  | { op: "minimize"; id: string }
  | { op: "restore"; id: string }
  | { op: "activeTab"; zone: ZoneId; group: number; id: string }
  | { op: "resizeZone"; zone: ZoneId; size: number }
  | { op: "resizeGroup"; zone: ZoneId; group: number; size: number }
  | { op: "resizeFloating"; id: string; rect: Rect }
  | { op: "compactView"; id: string }
  | { op: "popOut"; id: string }
  | { op: "popIn"; id: string };

/** Where a single panel id currently lives, as returned by `locate`. Mutually exclusive
 * and exhaustive: a panel is in exactly one of these five states at any time (see
 * `detach`'s own doc comment for the invariant this relies on). */
export type PanelLocation =
  | { where: "docked"; zone: ZoneId; group: number; tabIndex: number }
  | { where: "floating"; index: number }
  | { where: "minimized" }
  | { where: "popped-out" }
  | { where: "closed" };

// The fixed dock-zone set every `ExpandedLayout.zones` record always has an entry for
// (see `ExpandedLayout`'s own doc comment) — an iteration order for the loops in this file,
// not a priority. Mirrored (not shared) by `DockviewEngine`'s own `ZONE_IDS`, which walks the
// same three zones for its dockview-side reconciliation.
const ZONE_IDS: readonly ZoneId[] = ["right", "bottom", "left"];

// Px-basis defaults for a fresh zone. Arbitrary but stable; a host persists real sizes
// via `resizeZone` once the user drags a splitter.
const ZONE_DEFAULT_SIZE: Record<ZoneId, number> = { right: 320, bottom: 240, left: 320 };

// Floating-sheet cascade: a fixed base rect, offset a step per already-floating panel,
// wrapping every 6 so a burst of sheets never marches off-screen. Deterministic + pure
// so the reducer stays unit-testable (no viewport read). Value aligned with
// `PanelsController`'s own `REHYDRATE_FLOAT_BASE`/`REHYDRATE_FLOAT_STEP` (kept as a separate constant, not a
// shared import, so the two modules stay decoupled) — the same logical operation
// (reload -> float a persisted popout) must land at the same screen position
// regardless of which of the two call sites handles a given panel's registration timing.
// Nothing in either file's types enforces this pairing; the two constants can drift
// silently unless something exercises both call sites and compares their output. That
// enforcement is the "cascade parity at index %i: a floating placement and
// a rehydrated popout land on the identical rect" test, not a runtime assertion here.
const SHEET_CASCADE_BASE: Rect = { x: 96, y: 96, w: 420, h: 520 };
const SHEET_CASCADE_STEP = 28;

/** Builds the zones sub-tree for a brand-new `PanelLayoutV1` — every zone
 * present with no groups, at its `ZONE_DEFAULT_SIZE` px basis.
 * @returns A fresh `right`/`bottom`/`left` record of empty `ZoneNode`s.
 * @example
 * ```
 * // private function; not part of the public API — used only by
 * // defaultLayout's initial-layout construction
 * emptyZones();
 * ```
 */
function emptyZones(): Record<ZoneId, ZoneNode> {
  return {
    right: { groups: [], size: ZONE_DEFAULT_SIZE.right },
    bottom: { groups: [], size: ZONE_DEFAULT_SIZE.bottom },
    left: { groups: [], size: ZONE_DEFAULT_SIZE.left },
  };
}

/** Equal-share renormalization of a zone's groups after a structural insert/remove.
 * Manual `resizeGroup` calls are NOT renormalized — only insert/remove touches sizes.
 * @param groups The zone's groups AFTER the structural insert/remove, in their final
 * membership.
 * @returns `groups` itself (the same reference) only for an empty input; for any non-empty
 * `groups`, `Array.prototype.map` always allocates a new array here, even when every member's
 * `size` already equals `1 / groups.length` — in that case only the individual `GroupNode`
 * elements are reused (`g.size === size ? g : ...`), not the outer array.
 * @example
 * ```
 * // private function; not part of the public API — called only by detach,
 * // placeByPlacement, dock, and prune after a group is added or removed
 * renormalize([{ tabs: ["chat"], active: "chat", size: 1 }]);
 * ```
 */
function renormalize(groups: GroupNode[]): GroupNode[] {
  if (groups.length === 0) return groups;
  const size = 1 / groups.length;
  return groups.map((g) => (g.size === size ? g : { ...g, size }));
}

/** Reassigns floating z to a contiguous 0..n-1 range (ascending by current z), bounding
 * growth — repeated `open`/`float` focus-bumps never inflate z without limit.
 * @param floating The floating entries to reassign z for, in any order.
 * @returns A copy sorted ascending by z with each entry's z rewritten to its position in
 * that sort; an entry already at its correct z is reused untouched (not a whole-array
 * same-reference contract — the returned array is always a new one when reordering, but
 * unaffected entries keep their own object reference).
 * @example
 * ```
 * // private function; not part of the public API — called after every op that adds,
 * // removes, or re-orders a floating panel
 * compactZ([{ id: "chat", rect: { x: 0, y: 0, w: 1, h: 1 }, z: 3 }]);
 * ```
 */
function compactZ(
  floating: ExpandedLayout["floating"],
): ExpandedLayout["floating"] {
  return [...floating]
    .sort((a, b) => a.z - b.z)
    .map((f, i) => (f.z === i ? f : { ...f, z: i }));
}

/** Finds `id`'s current location in `l`. Every mutating op that relocates a panel calls
 * this first, then passes the result to `detach`.
 * @param l The layout to search.
 * @param id The panel id to locate.
 * @returns `id`'s location: `docked` (zone/group index/tabIndex), `floating` (index into
 * `expanded.floating`), `minimized`, `popped-out`, or `closed` when `id` is absent from
 * every one of those.
 * @example
 * ```ts
 * import { locate, defaultLayout } from "@shadowcat/module-panels";
 *
 * const layout = defaultLayout([]);
 * locate(layout, "chat"); // { where: "closed" }
 * ```
 */
export function locate(l: PanelLayoutV1, id: string): PanelLocation {
  for (const zone of ZONE_IDS) {
    const groups = l.expanded.zones[zone].groups;
    for (let g = 0; g < groups.length; g++) {
      const tabIndex = groups[g].tabs.indexOf(id);
      if (tabIndex !== -1) return { where: "docked", zone, group: g, tabIndex };
    }
  }
  const fi = l.expanded.floating.findIndex((f) => f.id === id);
  if (fi !== -1) return { where: "floating", index: fi };
  if (l.expanded.minimized.includes(id)) return { where: "minimized" };
  if (l.expanded.poppedOut.includes(id)) return { where: "popped-out" };
  return { where: "closed" };
}

/** Removes `id` from wherever it currently lives (INVARIANT: at most one location holds
 * it, so this is exhaustive). Used by every mutating op that relocates a panel; total —
 * a "closed" location is a no-op that returns the SAME reference.
 * @param l The layout to detach `id` from.
 * @param id The panel id to detach.
 * @returns A tuple of the resulting layout (`l` itself if `id` was already closed) and the
 * `PanelLocation` `id` was detached FROM, which callers use to decide where to re-place it.
 * @example
 * ```
 * // private function; not part of the public API — called by every op in applyOp
 * // that relocates a panel (open, dock, float, minimize, popOut, popIn)
 * declare const layout: import("./tree").PanelLayoutV1;
 * detach(layout, "chat");
 * ```
 */
function detach(l: PanelLayoutV1, id: string): [PanelLayoutV1, PanelLocation] {
  const loc = locate(l, id);
  switch (loc.where) {
    case "closed":
      return [l, loc];
    case "minimized": {
      const minimized = l.expanded.minimized.filter((m) => m !== id);
      return [{ ...l, expanded: { ...l.expanded, minimized } }, loc];
    }
    case "floating": {
      const floating = compactZ(l.expanded.floating.filter((_, i) => i !== loc.index));
      return [{ ...l, expanded: { ...l.expanded, floating } }, loc];
    }
    case "popped-out": {
      const poppedOut = l.expanded.poppedOut.filter((p) => p !== id);
      return [{ ...l, expanded: { ...l.expanded, poppedOut } }, loc];
    }
    case "docked": {
      const zoneNode = l.expanded.zones[loc.zone];
      const group = zoneNode.groups[loc.group];
      const tabs = group.tabs.filter((t) => t !== id);
      const groups =
        tabs.length === 0
          ? renormalize(zoneNode.groups.filter((_, i) => i !== loc.group))
          : zoneNode.groups.map((g, i) =>
              i === loc.group ? { ...g, tabs, active: group.active === id ? tabs[0] : group.active } : g,
            );
      return [
        { ...l, expanded: { ...l.expanded, zones: { ...l.expanded.zones, [loc.zone]: { ...zoneNode, groups } } } },
        loc,
      ];
    }
  }
}

/** Places a detached panel per an explicit or defaulted placement. Falls back to a new
 * docked group in "right" when no zone is given — used by op-driven `open`/`restore`,
 * where the caller is actively surfacing the panel (unlike `defaultLayout`, where an
 * absent `PanelMeta.defaultPlacement` means launcher-only/closed).
 * `DefaultPlacement.order` is not consumed here: callers pass registrations pre-sorted
 * by contribution order; a docked default always opens its own group.
 * @param l The layout to place `id` into. Caller guarantees `id` is already detached (not
 * present in any zone/floating/minimized/popped-out location).
 * @param id The panel id to place.
 * @param placement The explicit or defaulted placement; `undefined` falls back to a new
 * docked group in zone "right".
 * @returns The resulting layout, with `id` placed per `placement`.
 * @example
 * ```
 * // private function; not part of the public API — called by applyOp's "open"/"restore"/
 * // "popIn" cases after detach, and by placeNewRegistrations for a new registration
 * declare const layout: import("./tree").PanelLayoutV1;
 * placeByPlacement(layout, "chat", { kind: "docked", zone: "right" });
 * ```
 */
function placeByPlacement(l: PanelLayoutV1, id: string, placement?: DefaultPlacement): PanelLayoutV1 {
  if (placement?.kind === "floating") {
    const n = l.expanded.floating.length;
    const off = (n % 6) * SHEET_CASCADE_STEP;
    const rect: Rect = { x: SHEET_CASCADE_BASE.x + off, y: SHEET_CASCADE_BASE.y + off, w: SHEET_CASCADE_BASE.w, h: SHEET_CASCADE_BASE.h };
    const maxZ = l.expanded.floating.reduce((m, f) => Math.max(m, f.z), -1);
    const floating = compactZ([...l.expanded.floating, { id, rect, z: maxZ + 1 }]);
    return { ...l, expanded: { ...l.expanded, floating } };
  }
  if (placement?.kind === "minimized") {
    return { ...l, expanded: { ...l.expanded, minimized: [...l.expanded.minimized, id] } };
  }
  const zone: ZoneId = placement?.kind === "docked" ? placement.zone : "right";
  const zoneNode = l.expanded.zones[zone];
  const groups = renormalize([...zoneNode.groups, { tabs: [id], active: id, size: 0 }]);
  return { ...l, expanded: { ...l.expanded, zones: { ...l.expanded.zones, [zone]: { ...zoneNode, groups } } } };
}

/** The sole mutator of `PanelLayoutV1` — every panel-manager state change is a `LayoutOp`
 * dispatched through this reducer. SAME-REFERENCE NO-OP CONTRACT: when `o` would produce no
 * observable change to `l`, this returns `l` itself (not a structurally-equal copy) — see
 * each case's own guard above for what counts as a no-op there. `dock` is the one case with
 * no no-op path: it always reconstructs its target zone, even when detach found nothing to
 * move. Callers (persistence debounce, tests) rely on `toBe`, not deep equality, to detect
 * whether anything changed.
 * @param l The layout to apply `o` to.
 * @param o The op to apply.
 * @returns The resulting layout, or `l` itself when `o` changes nothing.
 * @example
 * ```ts
 * import { applyOp, defaultLayout } from "@shadowcat/module-panels";
 *
 * const layout = defaultLayout([]);
 * const next = applyOp(layout, { op: "open", id: "chat" });
 * ```
 */
export function applyOp(l: PanelLayoutV1, o: LayoutOp): PanelLayoutV1 {
  switch (o.op) {
    case "open": {
      const loc = locate(l, o.id);
      if (loc.where === "docked") {
        const zoneNode = l.expanded.zones[loc.zone];
        const group = zoneNode.groups[loc.group];
        if (group.active === o.id) return l;
        const groups = zoneNode.groups.map((g, i) => (i === loc.group ? { ...g, active: o.id } : g));
        return {
          ...l,
          expanded: { ...l.expanded, zones: { ...l.expanded.zones, [loc.zone]: { ...zoneNode, groups } } },
        };
      }
      if (loc.where === "floating") {
        const maxZ = l.expanded.floating.reduce((m, f) => Math.max(m, f.z), -1);
        const current = l.expanded.floating[loc.index];
        if (current.z === maxZ) return l;
        const floating = compactZ(
          l.expanded.floating.map((f, i) => (i === loc.index ? { ...f, z: maxZ + 1 } : f)),
        );
        return { ...l, expanded: { ...l.expanded, floating } };
      }
      // minimized or closed: detach (no-op if already closed), then surface it.
      const [l1] = detach(l, o.id);
      return placeByPlacement(l1, o.id, o.placement);
    }

    case "close": {
      const [l1] = detach(l, o.id);
      return l1;
    }

    case "dock": {
      // CONTRACT: a numeric `o.group` indexes the target zone as it stood in `l`, the
      // layout passed INTO this op — not post-detach state. If `id` was the sole tab of
      // an earlier group in the SAME target zone, `detach` removes that group and shifts
      // every later index down by one; resolving `o.group` after `detach` would then land
      // one group too far. Resolve the target group's IDENTITY here, before `detach` runs,
      // and re-find that identity in the post-detach array. Identity is keyed on the
      // group's `tabs` array reference rather than the `GroupNode` object itself: when a
      // whole group elsewhere in the zone is removed, `detach`'s `renormalize` rebuilds
      // every surviving `GroupNode` (equal-share resize touches every group's `size`) but
      // reuses each one's original `tabs` array untouched.
      const preZoneNode = l.expanded.zones[o.zone];
      const preLen = preZoneNode.groups.length;
      const preGi = o.group !== "new" && preLen > 0 ? Math.min(Math.max(o.group, 0), preLen - 1) : -1;
      const targetTabs = preGi !== -1 ? preZoneNode.groups[preGi].tabs : null;

      const [l1] = detach(l, o.id);
      const zoneNode = l1.expanded.zones[o.zone];

      let gi = targetTabs ? zoneNode.groups.findIndex((g) => g.tabs === targetTabs) : -1;
      if (gi === -1 && targetTabs && zoneNode.groups.length === preLen) {
        // `detach` mutated the group's OWN `tabs` array in place — this only happens when
        // `id` was detached from WITHIN the target group itself (multi-tab group, still
        // present, just a new `tabs` array). Array length is unchanged, so the index is
        // still valid even though the `tabs` reference is not.
        gi = preGi;
      }

      let groups: GroupNode[];
      if (gi === -1) {
        groups = renormalize([...zoneNode.groups, { tabs: [o.id], active: o.id, size: 0 }]);
      } else {
        groups = zoneNode.groups.map((g, i) => {
          if (i !== gi) return g;
          const at = o.tabIndex === undefined ? g.tabs.length : Math.min(Math.max(o.tabIndex, 0), g.tabs.length);
          const tabs = [...g.tabs.slice(0, at), o.id, ...g.tabs.slice(at)];
          return { ...g, tabs, active: o.id };
        });
      }
      return {
        ...l1,
        expanded: { ...l1.expanded, zones: { ...l1.expanded.zones, [o.zone]: { ...zoneNode, groups } } },
      };
    }

    case "float": {
      // Same-reference no-op for an already-floating id, mirroring
      // "minimize"'s already-minimized guard below: a menu "Float" command on
      // a panel already floating would otherwise `detach` + re-add it at
      // `o.rect` (always `MENU_FLOAT_RECT` for a menu trigger — see
      // `opForMenuCommand`), silently discarding whatever rect the user
      // already dragged/resized it to. Drag gestures never reach this branch
      // for an already-floating panel in the first place: `classifyDrop`'s
      // `kind: "floating"` case has no producer in `DockviewEngine.#toDropSite`'s
      // translation (only `"edge"`/`"group"` sites are ever
      // built from a real drag), so every `float` op in this codebase is
      // menu-originated — this guard changes no drag-originated behavior.
      const loc = locate(l, o.id);
      if (loc.where === "floating") return l;
      const [l1] = detach(l, o.id);
      const maxZ = l1.expanded.floating.reduce((m, f) => Math.max(m, f.z), -1);
      const floating = compactZ([...l1.expanded.floating, { id: o.id, rect: o.rect, z: maxZ + 1 }]);
      return { ...l1, expanded: { ...l1.expanded, floating } };
    }

    case "minimize": {
      const loc = locate(l, o.id);
      if (loc.where === "minimized") return l;
      const [l1] = detach(l, o.id);
      return { ...l1, expanded: { ...l1.expanded, minimized: [...l1.expanded.minimized, o.id] } };
    }

    case "restore": {
      const loc = locate(l, o.id);
      if (loc.where !== "minimized") return l;
      const [l1] = detach(l, o.id);
      return placeByPlacement(l1, o.id, { kind: "docked", zone: "right" });
    }

    case "activeTab": {
      const zoneNode = l.expanded.zones[o.zone];
      const group = zoneNode?.groups[o.group];
      if (!group || !group.tabs.includes(o.id) || group.active === o.id) return l;
      const groups = zoneNode.groups.map((g, i) => (i === o.group ? { ...g, active: o.id } : g));
      return { ...l, expanded: { ...l.expanded, zones: { ...l.expanded.zones, [o.zone]: { ...zoneNode, groups } } } };
    }

    case "resizeZone": {
      const zoneNode = l.expanded.zones[o.zone];
      if (zoneNode.size === o.size) return l;
      return { ...l, expanded: { ...l.expanded, zones: { ...l.expanded.zones, [o.zone]: { ...zoneNode, size: o.size } } } };
    }

    case "resizeGroup": {
      const zoneNode = l.expanded.zones[o.zone];
      const group = zoneNode?.groups[o.group];
      if (!group || group.size === o.size) return l;
      const groups = zoneNode.groups.map((g, i) => (i === o.group ? { ...g, size: o.size } : g));
      return { ...l, expanded: { ...l.expanded, zones: { ...l.expanded.zones, [o.zone]: { ...zoneNode, groups } } } };
    }

    case "resizeFloating": {
      // Live re-drag/resize sync of an ALREADY-floating panel (mirrors
      // "resizeZone"/"resizeGroup"'s in-place update, not "float"'s
      // detach-and-reinsert). No-ops for an id not currently floating —
      // engine-side, this op is only ever emitted for a panel the engine
      // itself already has open in a floating dockview group.
      const index = l.expanded.floating.findIndex((f) => f.id === o.id);
      if (index === -1) return l;
      const current = l.expanded.floating[index];
      if (
        current.rect.x === o.rect.x &&
        current.rect.y === o.rect.y &&
        current.rect.w === o.rect.w &&
        current.rect.h === o.rect.h
      ) {
        return l;
      }
      const floating = l.expanded.floating.map((f, i) => (i === index ? { ...f, rect: o.rect } : f));
      return { ...l, expanded: { ...l.expanded, floating } };
    }

    case "compactView": {
      if (!l.compact.order.includes(o.id) || l.compact.activeView === o.id) return l;
      return { ...l, compact: { ...l.compact, activeView: o.id } };
    }

    case "popOut": {
      // Same-reference no-op for an already-popped-out id, mirroring "float"'s
      // already-floating guard. The gesture-time `addPopoutGroup` in
      // `DockviewEngine` only emits this op AFTER a successful async open, so a
      // popped-out id here is always backed by a live child window.
      const loc = locate(l, o.id);
      if (loc.where === "popped-out") return l;
      const [l1] = detach(l, o.id);
      return { ...l1, expanded: { ...l1.expanded, poppedOut: [...l1.expanded.poppedOut, o.id] } };
    }

    case "popIn": {
      // Returns a popped-out panel to a new docked "right" group (mirrors
      // "restore"). Emitted by the engine when a popout window closes; the
      // menu's dock/float/minimize commands pop a panel in via their own ops
      // (detach handles the "popped-out" source location).
      const loc = locate(l, o.id);
      if (loc.where !== "popped-out") return l;
      const [l1] = detach(l, o.id);
      return placeByPlacement(l1, o.id, { kind: "docked", zone: "right" });
    }
  }
}

/** Drops every id not in `known` from all four locations (zones/floating/minimized) plus
 * `compact.order`, repairing `active`/`activeView` to a surviving member (or the group's/
 * order's new first entry). A zone's groups are renormalized — ALL surviving groups in
 * that zone get an equal-share `size`, not just the affected one — whenever pruning drops
 * a whole group from it; a group that merely lost one of several tabs (but is not itself
 * dropped) keeps every group's manually-set size untouched.
 * SAME-REFERENCE NO-OP CONTRACT: when `known` already contains every id anywhere in `l`,
 * this returns `l` itself (not a structurally-equal copy) — every zone/group/floating-
 * entry/compact-order reference is reused untouched. Callers (e.g. `PanelsController`)
 * rely on this to decide whether a prune pass actually changed anything worth persisting,
 * exactly like every no-op branch of `applyOp` above.
 * @param l The layout to prune.
 * @param known The set of panel ids still registered; anything else is dropped.
 * @returns The pruned layout, or `l` itself when nothing needed dropping or repairing.
 * @example
 * ```ts
 * import { prune, defaultLayout } from "@shadowcat/module-panels";
 *
 * const layout = defaultLayout([{ id: "chat" }]);
 * prune(layout, new Set(["chat"])); // no-op: chat is still known
 * ```
 */
export function prune(l: PanelLayoutV1, known: ReadonlySet<string>): PanelLayoutV1 {
  let changed = false;
  const zones = {} as Record<ZoneId, ZoneNode>;
  for (const zone of ZONE_IDS) {
    const zoneNode = l.expanded.zones[zone];
    const groups: GroupNode[] = [];
    let zoneChanged = false;
    for (const g of zoneNode.groups) {
      const tabs = g.tabs.filter((t) => known.has(t));
      if (tabs.length === 0) {
        zoneChanged = true;
        continue;
      }
      if (tabs.length === g.tabs.length) {
        groups.push(g);
      } else {
        const active = tabs.includes(g.active) ? g.active : tabs[0];
        groups.push({ ...g, tabs, active });
        zoneChanged = true;
      }
    }
    if (!zoneChanged) {
      zones[zone] = zoneNode;
      continue;
    }
    changed = true;
    zones[zone] = groups.length === zoneNode.groups.length ? { ...zoneNode, groups } : { ...zoneNode, groups: renormalize(groups) };
  }

  const floatingKept = l.expanded.floating.filter((f) => known.has(f.id));
  const floatingChanged = floatingKept.length !== l.expanded.floating.length;
  const floating = floatingChanged ? compactZ(floatingKept) : l.expanded.floating;
  if (floatingChanged) changed = true;

  const minimizedKept = l.expanded.minimized.filter((id) => known.has(id));
  const minimizedChanged = minimizedKept.length !== l.expanded.minimized.length;
  if (minimizedChanged) changed = true;

  const poppedOutKept = l.expanded.poppedOut.filter((id) => known.has(id));
  const poppedOutChanged = poppedOutKept.length !== l.expanded.poppedOut.length;
  if (poppedOutChanged) changed = true;

  const orderKept = l.compact.order.filter((id) => known.has(id));
  const orderChanged = orderKept.length !== l.compact.order.length;
  if (orderChanged) changed = true;

  const activeView =
    l.compact.activeView !== null && orderKept.includes(l.compact.activeView) ? l.compact.activeView : (orderKept[0] ?? null);
  if (activeView !== l.compact.activeView) changed = true;

  if (!changed) return l;

  return {
    ...l,
    expanded: {
      ...l.expanded,
      zones,
      floating,
      minimized: minimizedChanged ? minimizedKept : l.expanded.minimized,
      poppedOut: poppedOutChanged ? poppedOutKept : l.expanded.poppedOut,
    },
    compact: { activeView, order: orderChanged ? orderKept : l.compact.order },
  };
}

/** Places `id` into `l` at the EXACT location `loc` recorded for it in `source` — the
 * persisted-history counterpart to `placeByPlacement` (which uses only a static
 * `PanelMeta.defaultPlacement`). Docked groups have no stable identity across a session
 * (a `GroupNode` is a plain array), so a persisted group is matched by TAB MEMBERSHIP: if
 * some other member of `source`'s group for `id` is already live in the same zone, `id`
 * joins that exact live group — re-sorted to the persisted `tabs` order restricted to the
 * ids actually present — and inherits the persisted `active` tab when it is among them;
 * otherwise a fresh single-tab group opens for `id` alone, ready for later persisted
 * groupmates to join by this same rule (order-of-registration independent). Caller
 * guarantees `loc.where !== "closed"` — a closed-in-source id has nothing to place.
 * @param l The layout to place `id` into. Caller guarantees `id` is already detached.
 * @param id The panel id to place.
 * @param source The persisted (pre-prune) layout `loc` was located in — see
 * `decodeLayout`'s `source` field.
 * @param loc `id`'s location in `source`, as returned by `locate(source, id)`.
 * @returns The resulting layout, with `id` placed to reconstruct its persisted location.
 * @example
 * ```
 * // private function; not part of the public API — called only by
 * // placeNewRegistrations for a registration `locate` finds in `persistedSource`
 * declare const layout: import("./tree").PanelLayoutV1;
 * declare const source: import("./tree").PanelLayoutV1;
 * placeFromPersistedLocation(layout, "chat", source, { where: "minimized" });
 * ```
 */
function placeFromPersistedLocation(l: PanelLayoutV1, id: string, source: PanelLayoutV1, loc: PanelLocation): PanelLayoutV1 {
  switch (loc.where) {
    case "minimized":
      return { ...l, expanded: { ...l.expanded, minimized: [...l.expanded.minimized, id] } };

    case "floating": {
      const persisted = source.expanded.floating[loc.index];
      const maxZ = l.expanded.floating.reduce((m, f) => Math.max(m, f.z), -1);
      const floating = compactZ([...l.expanded.floating, { id, rect: persisted.rect, z: maxZ + 1 }]);
      return { ...l, expanded: { ...l.expanded, floating } };
    }

    case "docked": {
      const persistedGroup = source.expanded.zones[loc.zone].groups[loc.group];
      const zoneNode = l.expanded.zones[loc.zone];
      const gi = zoneNode.groups.findIndex(
        (g) => g.tabs.length > 0 && g.tabs.every((t) => persistedGroup.tabs.includes(t)),
      );
      let groups: GroupNode[];
      if (gi === -1) {
        groups = renormalize([...zoneNode.groups, { tabs: [id], active: id, size: 0 }]);
      } else {
        // Reorder to the persisted tab order, restricted to ids actually live (`id` plus
        // whatever of the persisted group's other members already joined this group).
        const tabs = persistedGroup.tabs.filter((t) => t === id || zoneNode.groups[gi].tabs.includes(t));
        const active = tabs.includes(persistedGroup.active) ? persistedGroup.active : zoneNode.groups[gi].active;
        groups = zoneNode.groups.map((g, i) => (i === gi ? { ...g, tabs, active } : g));
      }
      return { ...l, expanded: { ...l.expanded, zones: { ...l.expanded.zones, [loc.zone]: { ...zoneNode, groups } } } };
    }

    case "popped-out": {
      // Popouts never survive reload (no gesture to reopen the window); a
      // persisted popped-out panel comes back as floating. Same rule as
      // `PanelsController.#rehydratePoppedOut`, applied to the not-yet-
      // registered-panel path. Cascade-offsets the rect the same way
      // `placeByPlacement`'s floating branch does — an unoffset rect would
      // stack every rehydrated popout (and the first-ever floating panel) on
      // the identical (x,y), an invisible overlap distinguishable only by
      // z-order.
      const n = l.expanded.floating.length;
      const off = (n % 6) * SHEET_CASCADE_STEP;
      const rect: Rect = { x: SHEET_CASCADE_BASE.x + off, y: SHEET_CASCADE_BASE.y + off, w: SHEET_CASCADE_BASE.w, h: SHEET_CASCADE_BASE.h };
      const maxZ = l.expanded.floating.reduce((m, f) => Math.max(m, f.z), -1);
      const floating = compactZ([...l.expanded.floating, { id, rect, z: maxZ + 1 }]);
      return { ...l, expanded: { ...l.expanded, floating } };
    }

    case "closed":
      return l;
  }
}

/** Inserts `id` (not yet in `order`) preserving its RELATIVE order against every other id
 * already in `order` that also appears in `persistedSource.compact.order` — i.e. among ids
 * `persistedSource` has an opinion on, the final order converges to the persisted order
 * regardless of registration arrival order. An id absent from `persistedSource` (or with no
 * `persistedSource` at all) is simply appended, matching pre-persistence-aware behavior.
 * @param order The current `compact.order` array, not yet containing `id`.
 * @param id The panel id to insert.
 * @param persistedSource The persisted (pre-prune) layout to source a relative order from,
 * or `null` when there is none (a fresh layout).
 * @returns A new array with `id` inserted at the position `persistedSource` implies, or
 * appended when `persistedSource` is `null` or has no opinion on `id`.
 * @example
 * ```
 * // private function; not part of the public API — called only by
 * // placeNewRegistrations for every newly-seen registration
 * insertPersistedOrder(["chat"], "assets", null); // ["chat", "assets"]
 * ```
 */
function insertPersistedOrder(order: string[], id: string, persistedSource: PanelLayoutV1 | null): string[] {
  const srcOrder = persistedSource?.compact.order;
  if (!srcOrder || !srcOrder.includes(id)) return [...order, id];
  const idPos = srcOrder.indexOf(id);
  let at = order.length;
  for (let i = 0; i < order.length; i++) {
    const otherPos = srcOrder.indexOf(order[i]);
    if (otherPos !== -1 && otherPos > idPos) {
      at = i;
      break;
    }
  }
  return [...order.slice(0, at), id, ...order.slice(at)];
}

/** Incrementally places every registration in `regs` not yet present in `compact.order` —
 * i.e. never seen by this layout before. Used both by `defaultLayout` (a fresh layout,
 * where every id is "new", `persistedSource` always `null`) and by
 * `PanelsController.syncRegistrations` (a live layout catching up contributions that
 * register AFTER this controller's own construction — module registration order does not
 * guarantee every panel-contract module is present before the panel host itself mounts, or
 * even before construction completes).
 *
 * When `persistedSource` is non-null (the PRE-`prune` structurally-validated blob this
 * session's user actually saved — see `decodeLayout`'s `source` field) and records a real
 * location for a registration's id, that persisted location is reconstructed exactly via
 * `placeFromPersistedLocation` INSTEAD of `reg.placement`'s static default — this prevents a
 * boot race where every panel beyond the first-registering ones would race
 * `defaultLayout` against their own module's registration and get default-placed (and
 * `#persist`ed), silently discarding the user's saved layout on every reload. An id present
 * in `persistedSource.compact.order` but located nowhere (closed-but-known) is added to
 * `compact.order` and left otherwise unplaced — never re-opened via `reg.placement`. An id
 * genuinely absent from `persistedSource` (never seen by the user's saved session) falls
 * back to `reg.placement` exactly as before. `DefaultPlacement.order` is not consumed here:
 * callers pass registrations pre-sorted by contribution order; a docked default always
 * opens its own group.
 * Same-reference no-op contract: returns `l` itself when every `regs` id is already in
 * `compact.order` — a registration once placed here is never re-defaulted/re-persisted-over
 * even if the user later closes/moves it, since this only catches ids this layout has NEVER
 * recorded.
 * PRECONDITION: `regs` ids are unique (registry-guaranteed).
 * @param l The layout to place new registrations into.
 * @param regs The panel registrations to check for and place, in contribution order.
 * @param persistedSource The persisted (pre-prune) layout to reconstruct positions from, or
 * `null` for a fresh layout with no persisted history.
 * @returns The resulting layout, or `l` itself when every `regs` id is already known.
 * @example
 * ```ts
 * import { placeNewRegistrations, defaultLayout } from "@shadowcat/module-panels";
 *
 * const layout = defaultLayout([{ id: "chat" }]);
 * placeNewRegistrations(layout, [{ id: "chat" }, { id: "assets" }]);
 * ```
 */
export function placeNewRegistrations(
  l: PanelLayoutV1,
  regs: { id: string; placement?: DefaultPlacement }[],
  persistedSource: PanelLayoutV1 | null = null,
): PanelLayoutV1 {
  let out = l;
  let changed = false;
  for (const reg of regs) {
    if (out.compact.order.includes(reg.id)) continue;
    changed = true;
    out = { ...out, compact: { ...out.compact, order: insertPersistedOrder(out.compact.order, reg.id, persistedSource) } };

    const persistedLoc = persistedSource ? locate(persistedSource, reg.id) : null;
    const knownToPersistedSource = persistedSource != null && (persistedSource.compact.order.includes(reg.id) || persistedLoc?.where !== "closed");
    if (knownToPersistedSource) {
      if (persistedLoc && persistedLoc.where !== "closed") out = placeFromPersistedLocation(out, reg.id, persistedSource!, persistedLoc);
      // else: closed-but-known in the persisted blob — stays closed, never re-defaulted.
    } else if (reg.placement) {
      out = placeByPlacement(out, reg.id, reg.placement);
    }
  }
  if (changed && out.compact.activeView === null && out.compact.order.length > 0) {
    const preferred = persistedSource?.compact.activeView;
    out = {
      ...out,
      compact: { ...out.compact, activeView: preferred && out.compact.order.includes(preferred) ? preferred : out.compact.order[0] },
    };
  }
  return changed ? out : l;
}

/** Builds the initial layout for a module set at first launch — every `regs` entry is
 * "new" against an empty layout, so this is `placeNewRegistrations` applied to the empty
 * starting tree.
 * @param regs The panel registrations to seed the fresh layout with.
 * @returns A new `PanelLayoutV1` with every one of `regs` placed per its `placement`.
 * @example
 * ```ts
 * import { defaultLayout } from "@shadowcat/module-panels";
 *
 * defaultLayout([{ id: "chat", placement: { kind: "docked", zone: "right" } }]);
 * ```
 */
export function defaultLayout(regs: { id: string; placement?: DefaultPlacement }[]): PanelLayoutV1 {
  const empty: PanelLayoutV1 = {
    version: 1,
    expanded: { zones: emptyZones(), floating: [], minimized: [], poppedOut: [] },
    compact: { activeView: null, order: [] },
  };
  return placeNewRegistrations(empty, regs);
}
