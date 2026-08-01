// Pure drop-veto + zone-classification policy for the M12a stage well. NO
// dockview import here (that boundary is `dockview.ts`'s alone) — dockview's
// drop events are translated into `DropSite` INSIDE `dockview.ts`; this file
// only ever sees our own vocabulary, so it is directly unit-testable and
// carries zero DOM/engine dependency.
import type { ExpandedLayout, LayoutOp, Rect } from "../layout/tree";
import type { ZoneId } from "@shadowcat/core";

/** Reserved panel id for the always-present canvas/stage content. No
 * registration ever contributes this id — `DockviewEngine` mounts it directly
 * at `init()` (W1) — so `"stage"` can never collide with a real panel id. */
export const STAGE_ID = "stage";

/** Our own drop-target vocabulary — a dockview `DockviewWillDropEvent` is
 * translated into this shape by `dockview.ts`
 * before reaching `classifyDrop`. `id` is the panel being dropped/dragged;
 * it must be carried here (rather than passed as a separate argument)
 * because `"any op whose subject is 'stage'"` is itself a veto rule this
 * function evaluates. */
export interface DropSite {
  kind: "group" | "edge" | "floating";
  /** The panel id being dropped/dragged. */
  id: string;
  /** Target dock zone, for `kind: "group"` (an existing group lives in
   * exactly one zone) or resolved from `position` for `kind: "edge"`. */
  zone?: ZoneId;
  /** Target group index within `zone`, for `kind: "group"`. */
  group?: number;
  /** Tab-strip insertion index, when dropping into an existing group's tab
   * bar (`kind: "group"`, a `"tab"` — as opposed to `"content"` — drop). */
  tabIndex?: number;
  /** Target rect, for `kind: "floating"`. */
  rect?: Rect;
  /** True when this site IS the stage's own dedicated dockview group —
   * set by `DockviewEngine` when it recognises the target group as the one
   * it created for the stage at `init()`. `layout` (the pure tree) has no
   * notion of the stage at all, so this can't be derived from `layout`
   * alone. Defense-in-depth: dockview's `locked: 'no-drop-target'` on that
   * group already stops these drops from firing in practice (M12a-0 spike
   * report, dockviewGroupPanelModel.ts `handleDropEvent` — the model
   * returns before constructing/firing the drop event at all when
   * `locked === 'no-drop-target'`), so this branch is a second, independent
   * layer rather than the sole guard. */
  stageGroup?: boolean;
  position: "left" | "right" | "top" | "bottom" | "center";
}

/** Every command `PanelMenu` can offer, on a docked tab or a floating
 * header alike (no context-dependent filtering — `applyOp` already handles
 * every op from any starting location). */
export type MenuCommand = "dockRight" | "dockBottom" | "dockLeft" | "float" | "minimize" | "popOut" | "close";

/** Fixed floating rect a menu-triggered "Float" command opens at — a drag
 * gesture supplies its own drop rect (`classifyDrop`'s `kind: "floating"`
 * site, sourced from the pointer position); the menu has no pointer
 * position to derive one from. */
export const MENU_FLOAT_RECT: Rect = { x: 96, y: 96, w: 360, h: 280 };

/** Maps a `PanelMenu` command to the exact `LayoutOp` the equivalent drag
 * gesture produces through `classifyDrop`: `dockRight`/`dockBottom`/`dockLeft`
 * mirror an edge drop's `{op:"dock", zone, group:"new"}`; `minimize`/`close`
 * are already 1:1 with their own `LayoutOp`. `float` has no drag rect to
 * mirror (see `MENU_FLOAT_RECT`) — parity there is by construction, not by
 * equality with a drag result. Kept here (not in `dockview.ts`) so this
 * mapping — and its parity with `classifyDrop` — is testable with zero
 * dockview dependency.
 *
 * Vetoes `id === STAGE_ID` up front, mirroring `classifyDrop`'s own stage
 * veto — belt-and-suspenders alongside `dockview.ts`'s `#handleMenuCommand`
 * guard and W1's headerless stage group (which never gives the stage a menu
 * button to invoke this with in the first place): a menu-command chokepoint
 * with the identical shape as the drag chokepoint, not the sole guard.
 * @param command The `PanelMenu` command chosen.
 * @param id The panel id the command targets.
 * @returns The `LayoutOp` to dispatch, or a veto with its reason.
 * @example
 * ```
 * // not exported from the package's index.ts — internal to engine/dockview.ts's
 * // #handleMenuCommand, which translates a PanelMenu command into a LayoutOp
 * opForMenuCommand("dockRight", "chat");
 * ```
 */
export function opForMenuCommand(command: MenuCommand, id: string): ClassifyResult {
  if (id === STAGE_ID) {
    return veto("the stage panel is not a menu-command subject");
  }
  switch (command) {
    case "dockRight":
      return { op: "dock", id, zone: "right", group: "new" };
    case "dockBottom":
      return { op: "dock", id, zone: "bottom", group: "new" };
    case "dockLeft":
      return { op: "dock", id, zone: "left", group: "new" };
    case "float":
      return { op: "float", id, rect: MENU_FLOAT_RECT };
    case "minimize":
      return { op: "minimize", id };
    case "popOut":
      return { op: "popOut", id };
    case "close":
      return { op: "close", id };
  }
}

export type ClassifyResult = LayoutOp | { veto: true; reason: string };

/** Builds a veto result — the `{ veto: true, reason }` shape both
 * `classifyDrop` and `opForMenuCommand` return to refuse a gesture.
 * @param reason Human-readable reason surfaced in a vetoed-gesture log message.
 * @returns A veto `ClassifyResult`.
 * @example
 * ```
 * // private function; not part of the public API — invoked only by
 * // classifyDrop and opForMenuCommand's own veto branches
 * veto("the stage panel is not a draggable subject");
 * ```
 */
function veto(reason: string): { veto: true; reason: string } {
  return { veto: true, reason };
}

/** Classifies a drop site into the `LayoutOp` it should produce, or vetoes
 * it. Every veto here is deliberate defense-in-depth: dockview's own
 * `locked: 'no-drop-target'` (stage group) and this module's zone vocabulary
 * (no `"top"` `ZoneId` exists) already make most of these unreachable in
 * practice — this function is what a reviewer/test can exercise directly,
 * without needing a real drag gesture, to prove the invariant holds even if
 * an upstream guard were ever weakened.
 * @param target The drop site to classify (already translated into our own
 * vocabulary by `dockview.ts`).
 * @param layout The current expanded layout, consulted to validate a
 * `"group"` site's target zone exists.
 * @returns The `LayoutOp` this drop should produce, or a veto with its reason.
 * @example
 * ```ts
 * import { classifyDrop, type DropSite } from "@shadowcat/module-panels";
 * import type { ExpandedLayout } from "@shadowcat/module-panels";
 *
 * declare const site: DropSite;
 * declare const layout: ExpandedLayout;
 * const result = classifyDrop(site, layout);
 * ```
 */
export function classifyDrop(target: DropSite, layout: ExpandedLayout): ClassifyResult {
  // Rule: any op whose subject is "stage" — the stage panel itself is never
  // draggable (W1 leaves it no drag handle at all; this is the policy-level
  // backstop).
  if (target.id === STAGE_ID) {
    return veto("the stage panel is not a draggable subject");
  }

  // Rule: any drop targeting or splitting the stage's own group.
  if (target.stageGroup) {
    return veto("cannot dock into or split the stage's dedicated group");
  }

  switch (target.kind) {
    case "edge": {
      // Rule: no drop may resolve ABOVE the stage row — there is no "top"
      // ZoneId in this layout model (spec D4); a container-edge drop is the
      // only path that could otherwise ask for one.
      if (target.position === "top") {
        return veto("no top dock zone exists (spec D4)");
      }
      if (target.position === "center") {
        return veto("an edge drop cannot resolve to 'center'");
      }
      // left/right/bottom edges map 1:1 onto our zone names.
      return { op: "dock", id: target.id, zone: target.position, group: "new" };
    }

    case "group": {
      if (target.zone === undefined || target.group === undefined) {
        return veto("drop site is missing a target zone/group");
      }
      if (!layout.zones[target.zone]) {
        return veto(`unknown target zone "${target.zone}"`);
      }
      const op: LayoutOp = {
        op: "dock",
        id: target.id,
        zone: target.zone,
        group: target.group,
      };
      if (target.tabIndex !== undefined) {
        return { ...op, tabIndex: target.tabIndex };
      }
      return op;
    }

    case "floating": {
      if (!target.rect) {
        return veto("floating drop site is missing a target rect");
      }
      return { op: "float", id: target.id, rect: target.rect };
    }
  }
}
