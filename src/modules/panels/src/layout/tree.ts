// Pure layout tree + reducer for the M12a panel-manager host. Engine-agnostic: no Svelte,
// no dockview, no ui-kit — a host component maps this state onto whatever docking widget
// it renders (dockview-core, in this codebase). All mutating functions return a NEW object;
// unchanged-input calls return the SAME reference so a host can cheaply skip a re-render.
import type { ZoneId, DefaultPlacement } from "@shadowcat/core";

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

export interface ExpandedLayout {
  // All three ZoneId keys are always present, even with empty groups — callers never
  // guard a missing zone.
  zones: Record<ZoneId, ZoneNode>;
  floating: { id: string; rect: Rect; z: number }[];
  minimized: string[];
}

export interface CompactLayout {
  activeView: string | null;
  order: string[];
}

export interface PanelLayoutV1 {
  version: 1;
  expanded: ExpandedLayout;
  compact: CompactLayout;
}

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
  | { op: "compactView"; id: string };

export type PanelLocation =
  | { where: "docked"; zone: ZoneId; group: number; tabIndex: number }
  | { where: "floating"; index: number }
  | { where: "minimized" }
  | { where: "closed" };

const ZONE_IDS: readonly ZoneId[] = ["right", "bottom", "left"];

// Px-basis defaults for a fresh zone. Arbitrary but stable; a host persists real sizes
// via `resizeZone` once the user drags a splitter.
const ZONE_DEFAULT_SIZE: Record<ZoneId, number> = { right: 320, bottom: 240, left: 320 };

function emptyZones(): Record<ZoneId, ZoneNode> {
  return {
    right: { groups: [], size: ZONE_DEFAULT_SIZE.right },
    bottom: { groups: [], size: ZONE_DEFAULT_SIZE.bottom },
    left: { groups: [], size: ZONE_DEFAULT_SIZE.left },
  };
}

/** Equal-share renormalization of a zone's groups after a structural insert/remove.
 * Manual `resizeGroup` calls are NOT renormalized — only insert/remove touches sizes. */
function renormalize(groups: GroupNode[]): GroupNode[] {
  if (groups.length === 0) return groups;
  const size = 1 / groups.length;
  return groups.map((g) => (g.size === size ? g : { ...g, size }));
}

/** Reassigns floating z to a contiguous 0..n-1 range (ascending by current z), bounding
 * growth — repeated `open`/`float` focus-bumps never inflate z without limit. */
function compactZ(
  floating: ExpandedLayout["floating"],
): ExpandedLayout["floating"] {
  return [...floating]
    .sort((a, b) => a.z - b.z)
    .map((f, i) => (f.z === i ? f : { ...f, z: i }));
}

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
  return { where: "closed" };
}

/** Removes `id` from wherever it currently lives (INVARIANT: at most one location holds
 * it, so this is exhaustive). Used by every mutating op that relocates a panel; total —
 * a "closed" location is a no-op that returns the SAME reference. */
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
 * by contribution order; a docked default always opens its own group. */
function placeByPlacement(l: PanelLayoutV1, id: string, placement?: DefaultPlacement): PanelLayoutV1 {
  if (placement?.kind === "minimized") {
    return { ...l, expanded: { ...l.expanded, minimized: [...l.expanded.minimized, id] } };
  }
  const zone: ZoneId = placement?.kind === "docked" ? placement.zone : "right";
  const zoneNode = l.expanded.zones[zone];
  const groups = renormalize([...zoneNode.groups, { tabs: [id], active: id, size: 0 }]);
  return { ...l, expanded: { ...l.expanded, zones: { ...l.expanded.zones, [zone]: { ...zoneNode, groups } } } };
}

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

    case "compactView": {
      if (!l.compact.order.includes(o.id) || l.compact.activeView === o.id) return l;
      return { ...l, compact: { ...l.compact, activeView: o.id } };
    }
  }
}

/** Drops every id not in `known` from all four locations (zones/floating/minimized) plus
 * `compact.order`, repairing `active`/`activeView` to a surviving member (or the group's/
 * order's new first entry). A zone's groups are renormalized — ALL surviving groups in
 * that zone get an equal-share `size`, not just the affected one — whenever pruning drops
 * a whole group from it; a group that merely lost one of several tabs (but is not itself
 * dropped) keeps every group's manually-set size untouched. */
export function prune(l: PanelLayoutV1, known: ReadonlySet<string>): PanelLayoutV1 {
  const zones = {} as Record<ZoneId, ZoneNode>;
  for (const zone of ZONE_IDS) {
    const zoneNode = l.expanded.zones[zone];
    const groups: GroupNode[] = [];
    for (const g of zoneNode.groups) {
      const tabs = g.tabs.filter((t) => known.has(t));
      if (tabs.length === 0) continue;
      const active = tabs.includes(g.active) ? g.active : tabs[0];
      groups.push(tabs.length === g.tabs.length && active === g.active ? g : { ...g, tabs, active });
    }
    zones[zone] = groups.length === zoneNode.groups.length ? { ...zoneNode, groups } : { ...zoneNode, groups: renormalize(groups) };
  }
  const floating = compactZ(l.expanded.floating.filter((f) => known.has(f.id)));
  const minimized = l.expanded.minimized.filter((id) => known.has(id));
  const order = l.compact.order.filter((id) => known.has(id));
  const activeView =
    l.compact.activeView !== null && order.includes(l.compact.activeView) ? l.compact.activeView : (order[0] ?? null);
  return {
    ...l,
    expanded: { ...l.expanded, zones, floating, minimized },
    compact: { activeView, order },
  };
}

/** Builds the initial layout for a module set at first launch. A registration with no
 * `placement` mirrors `PanelMeta.defaultPlacement` absence: launcher-only/closed — present
 * in `compact.order` (so compact mode can still switch to it) but nowhere in `expanded`.
 * `DefaultPlacement.order` is not consumed here: callers pass registrations pre-sorted
 * by contribution order; a docked default always opens its own group.
 * PRECONDITION: `regs` ids are unique (registry-guaranteed). */
export function defaultLayout(regs: { id: string; placement?: DefaultPlacement }[]): PanelLayoutV1 {
  let l: PanelLayoutV1 = {
    version: 1,
    expanded: { zones: emptyZones(), floating: [], minimized: [] },
    compact: { activeView: null, order: [] },
  };
  for (const reg of regs) {
    l = { ...l, compact: { ...l.compact, order: [...l.compact.order, reg.id] } };
    if (reg.placement) l = placeByPlacement(l, reg.id, reg.placement);
  }
  if (l.compact.order.length > 0) {
    l = { ...l, compact: { ...l.compact, activeView: l.compact.order[0] } };
  }
  return l;
}
