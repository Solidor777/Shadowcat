// Persistence codec for PanelLayoutV1. Pure; no Svelte, no shell coupling. The shell's
// `UiState.worlds[world].panelLayout` field is `unknown` (Zod-free by design — see
// shadowcat-codebase-client-shell), so decoding must hand-roll its own structural guards
// rather than lean on a schema library.
import { prune, type CompactLayout, type ExpandedLayout, type GroupNode, type PanelLayoutV1, type Rect, type ZoneNode } from "./tree";

const ZONE_IDS = ["right", "bottom", "left"] as const;

/** Identity encode: `PanelLayoutV1` is already JSON-safe (plain objects/arrays/primitives). */
export function encodeLayout(l: PanelLayoutV1): unknown {
  return l;
}

function isString(v: unknown): v is string {
  return typeof v === "string";
}

function isStringArray(v: unknown): v is string[] {
  return Array.isArray(v) && v.every(isString);
}

function isRect(v: unknown): v is Rect {
  if (typeof v !== "object" || v === null) return false;
  const r = v as Record<string, unknown>;
  return typeof r.x === "number" && typeof r.y === "number" && typeof r.w === "number" && typeof r.h === "number";
}

function isGroupNode(v: unknown): v is GroupNode {
  if (typeof v !== "object" || v === null) return false;
  const g = v as Record<string, unknown>;
  return isStringArray(g.tabs) && isString(g.active) && typeof g.size === "number";
}

function isZoneNode(v: unknown): v is ZoneNode {
  if (typeof v !== "object" || v === null) return false;
  const z = v as Record<string, unknown>;
  return Array.isArray(z.groups) && z.groups.every(isGroupNode) && typeof z.size === "number";
}

function isExpandedLayout(v: unknown): v is ExpandedLayout {
  if (typeof v !== "object" || v === null) return false;
  const e = v as Record<string, unknown>;
  const zones = e.zones as Record<string, unknown> | undefined;
  if (typeof zones !== "object" || zones === null) return false;
  if (!ZONE_IDS.every((z) => isZoneNode(zones[z]))) return false;
  if (!Array.isArray(e.floating)) return false;
  if (
    !e.floating.every(
      (f) =>
        typeof f === "object" &&
        f !== null &&
        isString((f as Record<string, unknown>).id) &&
        isRect((f as Record<string, unknown>).rect) &&
        typeof (f as Record<string, unknown>).z === "number",
    )
  ) {
    return false;
  }
  return isStringArray(e.minimized);
}

function isCompactLayout(v: unknown): v is CompactLayout {
  if (typeof v !== "object" || v === null) return false;
  const c = v as Record<string, unknown>;
  return (c.activeView === null || isString(c.activeView)) && isStringArray(c.order);
}

/** Structural validation for a raw `PanelLayoutV1` blob. Every panel id anywhere in the
 * tree must be a string (checked transitively by the `isString`/`isStringArray` guards
 * above) — a non-string id anywhere fails the whole blob, matching the brief's "any panel
 * id non-string" clause. */
function isPanelLayoutV1(v: unknown): v is PanelLayoutV1 {
  if (typeof v !== "object" || v === null) return false;
  const l = v as Record<string, unknown>;
  if (l.version !== 1) return false;
  return isExpandedLayout(l.expanded) && isCompactLayout(l.compact);
}

/** Decodes a persisted blob. Returns `reset: true` with a freshly-built `fallback()` layout
 * on ANY structural mismatch (non-object, wrong version, malformed shape, non-string id) —
 * the shell's `ui_state` is deliberately Zod-free, so this hand-rolled guard is the only
 * validation layer. A structurally valid blob is then `prune`d against `known` so stale
 * panel ids (module uninstalled/renamed since last save) never linger; pruning alone does
 * NOT trigger a reset — `reset` reports decode-time validity, not membership drift. */
export function decodeLayout(
  raw: unknown,
  known: ReadonlySet<string>,
  fallback: () => PanelLayoutV1,
): { layout: PanelLayoutV1; reset: boolean } {
  if (!isPanelLayoutV1(raw)) return { layout: fallback(), reset: true };
  return { layout: prune(raw, known), reset: false };
}
