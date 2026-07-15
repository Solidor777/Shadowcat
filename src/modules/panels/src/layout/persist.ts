// Persistence codec for PanelLayoutV1. Pure; no Svelte, no shell coupling. The shell's
// `UiState.worlds[world].panelLayout` field is `unknown` (Zod-free by design — see
// shadowcat-codebase-client-shell), so decoding must hand-roll its own structural guards
// rather than lean on a schema library.
import { prune, type CompactLayout, type ExpandedLayout, type GroupNode, type PanelLayoutV1, type Rect, type ZoneNode } from "./tree";

const ZONE_IDS = ["right", "bottom", "left"] as const;

/** Returns the layout as-is: tree ops are immutable-by-construction, so the reference is
 * never mutated after encode. */
export function encodeLayout(l: PanelLayoutV1): unknown {
  return l;
}

function isString(v: unknown): v is string {
  return typeof v === "string";
}

function isStringArray(v: unknown): v is string[] {
  return Array.isArray(v) && v.every(isString);
}

/** Finite-only check: `NaN`/`±Infinity` must fail every numeric guard below, or they reach
 * reducer arithmetic (and later CSS) as silently-broken layout. */
function isFiniteNumber(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

/** Finite AND non-negative — for magnitudes (`w`/`h`/`size`/`z`) that are never legitimately
 * negative. Rect `x`/`y` are excluded: a panel legitimately dragged off-screen-left is
 * negative, only non-finite is invalid there. */
function isFiniteNonNeg(v: unknown): v is number {
  return isFiniteNumber(v) && v >= 0;
}

function isRect(v: unknown): v is Rect {
  if (typeof v !== "object" || v === null) return false;
  const r = v as Record<string, unknown>;
  return isFiniteNumber(r.x) && isFiniteNumber(r.y) && isFiniteNonNeg(r.w) && isFiniteNonNeg(r.h);
}

function isGroupNode(v: unknown): v is GroupNode {
  if (typeof v !== "object" || v === null) return false;
  const g = v as Record<string, unknown>;
  return isStringArray(g.tabs) && isString(g.active) && isFiniteNonNeg(g.size);
}

function isZoneNode(v: unknown): v is ZoneNode {
  if (typeof v !== "object" || v === null) return false;
  const z = v as Record<string, unknown>;
  return Array.isArray(z.groups) && z.groups.every(isGroupNode) && isFiniteNonNeg(z.size);
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
        isFiniteNonNeg((f as Record<string, unknown>).z),
    )
  ) {
    return false;
  }
  if (!isStringArray(e.minimized)) return false;
  // Back-compat: a pre-M12e blob has no `poppedOut`; absent normalizes to []
  // in `decodeLayout`. A present-but-malformed value fails the whole blob.
  return e.poppedOut === undefined || isStringArray(e.poppedOut);
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

/** Checks referential consistency for a structurally-valid `PanelLayoutV1`: each group's
 * `active` must be one of its own `tabs`, and `compact.activeView` must be `null` or present
 * in `compact.order`. Both ids are KNOWN-valid (structural guard already passed), so `prune`
 * — which only drops ids missing from `known` — cannot repair a dangling reference among ids
 * that are all still present; a violation here must fail the decode instead. */
function isReferentiallyConsistent(l: PanelLayoutV1): boolean {
  for (const zone of ZONE_IDS) {
    for (const g of l.expanded.zones[zone].groups) {
      if (!g.tabs.includes(g.active)) return false;
    }
  }
  return l.compact.activeView === null || l.compact.order.includes(l.compact.activeView);
}

/** Fills an absent `poppedOut` (pre-M12e blob) with `[]` so reducer arithmetic
 * (`prune`/`locate`/`detach`) never dereferences `undefined`. Returns the input
 * untouched when the field is already an array (the common, current-version path). */
function withPoppedOut(l: PanelLayoutV1): PanelLayoutV1 {
  if (Array.isArray(l.expanded.poppedOut)) return l;
  return { ...l, expanded: { ...l.expanded, poppedOut: [] } };
}

/** Decodes a persisted blob. Returns `reset: true` with a freshly-built `fallback()` layout
 * on ANY structural mismatch (non-object, wrong version, malformed shape, non-string id) or
 * referential inconsistency (`active`/`activeView` pointing at an id absent from its own
 * `tabs`/`order`) — the shell's `ui_state` is deliberately Zod-free, so this hand-rolled guard
 * is the only validation layer. A structurally valid AND referentially consistent blob is
 * then `prune`d against `known` so stale panel ids (module uninstalled/renamed since last
 * save) never linger; pruning alone does NOT trigger a reset — `reset` reports decode-time
 * validity, not membership drift.
 *
 * `source` carries the SAME guard-validated blob `layout` was pruned FROM — `null` on
 * `reset`, otherwise the untouched `raw` (already known to satisfy `isPanelLayoutV1` +
 * `isReferentiallyConsistent`), before `known`-membership pruning ever ran. `known` here is
 * necessarily whatever the CALLER already had registered at decode time (routinely empty or
 * partial — module registration order does not guarantee every panel-contract module has
 * registered before the panel host itself mounts and decodes); pruning against that partial
 * set would otherwise permanently drop every not-yet-registered panel's saved position.
 * `PanelsController` retains `source` to reconstruct later-registering panels' persisted
 * locations via `placeNewRegistrations` instead of losing them to that race. */
export function decodeLayout(
  raw: unknown,
  known: ReadonlySet<string>,
  fallback: () => PanelLayoutV1,
): { layout: PanelLayoutV1; reset: boolean; source: PanelLayoutV1 | null } {
  if (!isPanelLayoutV1(raw) || !isReferentiallyConsistent(raw)) return { layout: fallback(), reset: true, source: null };
  const normalized = withPoppedOut(raw);
  return { layout: prune(normalized, known), reset: false, source: normalized };
}
