// Persistence codec for PanelLayoutV1. Pure; no Svelte, no shell coupling. The shell's
// `UiState.worlds[world].panelLayout` field is `unknown` (Zod-free by design — see
// shadowcat-codebase-client-shell), so decoding must hand-roll its own structural guards
// rather than lean on a schema library.
import { prune, type CompactLayout, type ExpandedLayout, type GroupNode, type PanelLayoutV1, type Rect, type ZoneNode } from "./tree";

const ZONE_IDS = ["right", "bottom", "left"] as const;

/** Returns the layout as-is: tree ops are immutable-by-construction, so the reference is
 * never mutated after encode.
 * @param l The layout to encode.
 * @returns `l` itself, typed `unknown` to match the shell's Zod-free `panelLayout` storage.
 * @example
 * ```ts
 * import { encodeLayout, defaultLayout } from "@shadowcat/module-panels";
 *
 * encodeLayout(defaultLayout([]));
 * ```
 */
export function encodeLayout(l: PanelLayoutV1): unknown {
  return l;
}

/** Type guard: is `v` a string.
 * @param v The value to check.
 * @returns Whether `v` is a `string`.
 * @example
 * ```
 * // private function; not part of the public API — used by every structural
 * // guard below that checks a single id/name field
 * isString("chat");
 * ```
 */
function isString(v: unknown): v is string {
  return typeof v === "string";
}

/** Type guard: is `v` an array of strings.
 * @param v The value to check.
 * @returns Whether `v` is an array (of any length) whose every element is a `string`.
 * @example
 * ```
 * // private function; not part of the public API — used to validate `tabs`,
 * // `minimized`, `poppedOut`, and `compact.order`
 * isStringArray(["chat", "assets"]);
 * ```
 */
function isStringArray(v: unknown): v is string[] {
  return Array.isArray(v) && v.every(isString);
}

/** Finite-only check: `NaN`/`±Infinity` must fail every numeric guard below, or they reach
 * reducer arithmetic (and later CSS) as silently-broken layout.
 * @param v The value to check.
 * @returns Whether `v` is a `number` and `Number.isFinite(v)`.
 * @example
 * ```
 * // private function; not part of the public API — the base guard `isFiniteNonNeg`
 * // and `isRect`'s x/y checks build on
 * isFiniteNumber(96);
 * ```
 */
function isFiniteNumber(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

/** Finite AND non-negative — for magnitudes (`w`/`h`/`size`/`z`) that are never legitimately
 * negative. Rect `x`/`y` are excluded: a panel legitimately dragged off-screen-left is
 * negative, only non-finite is invalid there.
 * @param v The value to check.
 * @returns Whether `v` is finite and `v >= 0`.
 * @example
 * ```
 * // private function; not part of the public API — used to validate `w`/`h`/`size`/`z`
 * // fields across isRect/isGroupNode/isZoneNode/isExpandedLayout
 * isFiniteNonNeg(320);
 * ```
 */
function isFiniteNonNeg(v: unknown): v is number {
  return isFiniteNumber(v) && v >= 0;
}

/** Type guard for a `Rect`: `x`/`y` may be any finite number (a panel legitimately
 * dragged off-screen has a negative coordinate); `w`/`h` must additionally be non-negative.
 * @param v The value to check.
 * @returns Whether `v` structurally satisfies `Rect`.
 * @example
 * ```
 * // private function; not part of the public API — used to validate floating
 * // entries' `rect` field
 * isRect({ x: 96, y: 96, w: 420, h: 520 });
 * ```
 */
function isRect(v: unknown): v is Rect {
  if (typeof v !== "object" || v === null) return false;
  const r = v as Record<string, unknown>;
  return isFiniteNumber(r.x) && isFiniteNumber(r.y) && isFiniteNonNeg(r.w) && isFiniteNonNeg(r.h);
}

/** Type guard for a `GroupNode`. Does NOT check that `active` is a member of `tabs` — that
 * cross-field invariant is `isReferentiallyConsistent`'s job, run only after every id in the
 * tree is already known structurally valid.
 * @param v The value to check.
 * @returns Whether `v` structurally satisfies `GroupNode`.
 * @example
 * ```
 * // private function; not part of the public API — used by isZoneNode to validate
 * // each zone's `groups` array
 * isGroupNode({ tabs: ["chat"], active: "chat", size: 1 });
 * ```
 */
function isGroupNode(v: unknown): v is GroupNode {
  if (typeof v !== "object" || v === null) return false;
  const g = v as Record<string, unknown>;
  return isStringArray(g.tabs) && isString(g.active) && isFiniteNonNeg(g.size);
}

/** Type guard for a `ZoneNode`.
 * @param v The value to check.
 * @returns Whether `v` structurally satisfies `ZoneNode` (an array of `GroupNode`s plus a
 * non-negative `size`).
 * @example
 * ```
 * // private function; not part of the public API — used by isExpandedLayout to
 * // validate each of the three fixed zone keys
 * isZoneNode({ groups: [], size: 320 });
 * ```
 */
function isZoneNode(v: unknown): v is ZoneNode {
  if (typeof v !== "object" || v === null) return false;
  const z = v as Record<string, unknown>;
  return Array.isArray(z.groups) && z.groups.every(isGroupNode) && isFiniteNonNeg(z.size);
}

/** Type guard for an `ExpandedLayout`. `poppedOut` is back-compat-optional (see the inline
 * comment below): its ABSENCE is valid (a blob predating the `poppedOut` field), normalized to `[]` by
 * `withPoppedOut` after this guard passes; a PRESENT-but-malformed value instead fails this
 * guard, which fails the WHOLE blob via `isPanelLayoutV1` — the two cases are opposites, not
 * the same leniency applied twice.
 * @param v The value to check.
 * @returns Whether `v` structurally satisfies `ExpandedLayout`.
 * @example
 * ```
 * // private function; not part of the public API — used only by isPanelLayoutV1
 * isExpandedLayout({
 *   zones: { right: { groups: [], size: 320 }, bottom: { groups: [], size: 240 }, left: { groups: [], size: 320 } },
 *   floating: [],
 *   minimized: [],
 *   poppedOut: [],
 * });
 * ```
 */
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
  // Back-compat: a blob predating the `poppedOut` field has none; absent normalizes to []
  // in `decodeLayout`. A present-but-malformed value fails the whole blob.
  return e.poppedOut === undefined || isStringArray(e.poppedOut);
}

/** Type guard for a `CompactLayout`. Does NOT check that `activeView` is a member of
 * `order` — that cross-field invariant is `isReferentiallyConsistent`'s job.
 * @param v The value to check.
 * @returns Whether `v` structurally satisfies `CompactLayout`.
 * @example
 * ```
 * // private function; not part of the public API — used only by isPanelLayoutV1
 * isCompactLayout({ activeView: null, order: [] });
 * ```
 */
function isCompactLayout(v: unknown): v is CompactLayout {
  if (typeof v !== "object" || v === null) return false;
  const c = v as Record<string, unknown>;
  return (c.activeView === null || isString(c.activeView)) && isStringArray(c.order);
}

/** Structural validation for a raw `PanelLayoutV1` blob. Every panel id anywhere in the
 * tree must be a string (checked transitively by the `isString`/`isStringArray` guards
 * above) — a non-string id anywhere fails the whole blob, matching the brief's "any panel
 * id non-string" clause.
 * @param v The value to check.
 * @returns Whether `v` structurally satisfies `PanelLayoutV1` (does NOT check the
 * cross-field referential invariants `isReferentiallyConsistent` checks separately).
 * @example
 * ```
 * // private function; not part of the public API — the first of the two guards
 * // decodeLayout runs before accepting a persisted blob
 * declare const raw: unknown;
 * isPanelLayoutV1(raw);
 * ```
 */
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
 * that are all still present; a violation here must fail the decode instead.
 * @param l A structurally-valid `PanelLayoutV1` (already passed `isPanelLayoutV1`).
 * @returns Whether every group's `active` is one of its own `tabs` AND `compact.activeView`
 * is `null` or present in `compact.order`.
 * @example
 * ```
 * // private function; not part of the public API — the second of the two guards
 * // decodeLayout runs before accepting a persisted blob
 * declare const l: import("./tree").PanelLayoutV1;
 * isReferentiallyConsistent(l);
 * ```
 */
function isReferentiallyConsistent(l: PanelLayoutV1): boolean {
  for (const zone of ZONE_IDS) {
    for (const g of l.expanded.zones[zone].groups) {
      if (!g.tabs.includes(g.active)) return false;
    }
  }
  return l.compact.activeView === null || l.compact.order.includes(l.compact.activeView);
}

/** Fills an absent `poppedOut` (a blob predating the field) with `[]` so reducer arithmetic
 * (`prune`/`locate`/`detach`) never dereferences `undefined`. Returns the input
 * untouched when the field is already an array (the common, current-version path).
 * @param l A structurally-valid, referentially-consistent `PanelLayoutV1` (its
 * `expanded.poppedOut` may still be absent — see `isExpandedLayout`).
 * @returns `l` itself (same reference) when `expanded.poppedOut` is already an array;
 * otherwise a new layout with `expanded.poppedOut` set to `[]`.
 * @example
 * ```
 * // private function; not part of the public API — called only by decodeLayout,
 * // after the structural + referential guards both pass
 * declare const l: import("./tree").PanelLayoutV1;
 * withPoppedOut(l);
 * ```
 */
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
 * `reset`, otherwise the normalized blob (`withPoppedOut(raw)`: `raw` itself, by reference,
 * when `expanded.poppedOut` is already an array; a NEW object with `poppedOut: []` filled in
 * when a blob predating the `poppedOut` field omits it — so `source` is `raw` by reference only in the
 * common/current-version case, not unconditionally), already known to satisfy
 * `isPanelLayoutV1` + `isReferentiallyConsistent`, before `known`-membership pruning ever ran.
 * `known` here is necessarily whatever the CALLER already had registered at decode time
 * (routinely empty or partial — module registration order does not guarantee every
 * panel-contract module has registered before the panel host itself mounts and decodes);
 * pruning against that partial set would otherwise permanently drop every not-yet-registered
 * panel's saved position. `PanelsController` retains `source` to reconstruct later-registering
 * panels' persisted locations via `placeNewRegistrations` instead of losing them to that race.
 * @param raw The persisted blob, of unknown shape (see the file header).
 * @param known The panel ids the caller already has registered at decode time — consumed only
 * by the post-validation `prune` pass, not by the structural/referential validity check.
 * @param fallback Builds the layout to use when `raw` fails validation — typically
 * `defaultLayout` seeded with the caller's current registrations.
 * @returns `layout` (the value to use), `reset` (`true` iff `raw` failed validation), and
 * `source` (the pre-prune normalized blob, or `null` on reset) for later-registering panels to
 * reconstruct their persisted position from.
 * @example
 * ```ts
 * import { decodeLayout, defaultLayout } from "@shadowcat/module-panels";
 *
 * const known = new Set(["chat"]);
 * const { layout, reset, source } = decodeLayout(
 *   undefined,
 *   known,
 *   () => defaultLayout([{ id: "chat" }]),
 * );
 * ```
 */
export function decodeLayout(
  raw: unknown,
  known: ReadonlySet<string>,
  fallback: () => PanelLayoutV1,
): {
  /** The layout to use — `raw` decoded and pruned against `known`, or `fallback()`'s result
   * on any validation failure. */
  layout: PanelLayoutV1;
  /** `true` iff `raw` failed structural or referential validation (see this function's own
   * doc); pruning alone never sets this. */
  reset: boolean;
  /** The pre-prune normalized blob, or `null` on `reset`; see this function's own doc for
   * why later-registering panels need it. */
  source: PanelLayoutV1 | null;
} {
  if (!isPanelLayoutV1(raw) || !isReferentiallyConsistent(raw)) return { layout: fallback(), reset: true, source: null };
  const normalized = withPoppedOut(raw);
  return { layout: prune(normalized, known), reset: false, source: normalized };
}
