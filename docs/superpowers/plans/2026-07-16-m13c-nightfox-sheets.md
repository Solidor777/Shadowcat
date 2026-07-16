# M13c · Nightfox Sheets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Register Nightfox actor/item/effect sheets that render the resolved `system.stats` model (type-specific editors, computed-value previews, error/warning chips, presentation-only drag/drop reorder, item/effect modifier editors with active/transfer toggles) through the M12c `shadowcat.sheet:<doc_type>` registry, above the generic sheets.

**Architecture:** M13c is NOT a new package. It adds sheet source under `src/sheets/` inside the **single existing Nightfox module repo** (`C:\Dev\Nightfox`, bootstrapped by M13-1 Task 18, populated with the rules engine by M13b), all compiled into that one module's `dist/index.js`. Sheets are pure Svelte 5 components over the OPTIMISTIC store: they read a top-level `WireDocument`, feed it to M13b's `resolveNightfox` (which always walks from the top-level host so item→actor modifier flow is correct), and write every edit as a field-path Update carrying the RAW stored `old` pre-image. Engine packages (`svelte`, `@shadowcat/core`, `@shadowcat/ui-kit`, `@shadowcat/formula`, `@shadowcat/types`) are build-time externals resolved at runtime by the host import map — never bundled.

**Tech Stack:** TypeScript, Svelte 5 (Runes), Vitest + @testing-library/svelte, Zod (via M13b), `@shadowcat/formula`.

**Execution dependency:** M13c executes ONLY after **M13-1** (external-module toolchain implemented; Nightfox repo bootstrapped) AND **M13b** (rules package source — `src/nightfox-docs.ts`, `src/contributions.ts`, `src/resolve.ts` — exists in the Nightfox repo) AND **M12c** (sheet registry + `item` doc_type) AND **M13-0** (three-band document shape: `engine`/`system` split). Every task's paths are **relative to the Nightfox repo root** (`C:\Dev\Nightfox\`). Execution happens in the **nested dev location** — the Nightfox repo cloned into a Shadowcat checkout at `src/modules/nightfox/` so the pnpm workspace resolves `@shadowcat/*` (M13-1 Task 18 README dev flow). All commands run `pnpm --filter nightfox <script>` from the **nested Shadowcat checkout root**. All commits are made **inside the Nightfox repo** (`cd "C:\Dev\Nightfox"`), except the Shadowcat-repo doc rows in Task 12, which commit in the Shadowcat checkout. **Never `git push` the Nightfox repo** — the user owns its remote.

## Global Constraints

These are copied from the M13 spec's sheet-relevant hard requirements (§6, §10, D8–D14) and the M13-1 toolchain invariants that bind module code. Every task's requirements implicitly include this section.

- **Single-instance externals (M13-1 invariant 1):** `svelte`, every `svelte/*` subpath, and every `@shadowcat/*` package are `rollupOptions.external` — never bundled. Sheets import them normally; the host supplies exactly one runtime instance via the import map. Adding a runtime dependency is a defect.
- **Parity dev flow (M13-1 invariant 3):** Nightfox always loads through the modules-folder → server → import-map path, including in dev. M13c adds no static-import or dev-only shortcut.
- **Sheets read the OPTIMISTIC store (spec §6):** every derived value comes from `ctx.documents` (the per-recipient-redacted, rollback-applied view), never `ctx.store`. `resolveNightfox` is fed ONLY an optimistic-store doc — nothing hidden reaches the evaluator (§10). Formula evaluation can neither widen visibility nor leak.
- **Real OCC pre-images (spec §6, [[sheet-reactive-bridge-missing-subscription]]):** every field-path Update's `old` is the RAW current stored value at that path, read through a `createSubscriber`/`subscribe()` bridge so a second edit in the same instance reads the first edit's result. A hardcoded/defaulted `old` is a Critical (rejected+rolled-back on the 2nd edit).
- **Map-keyed CRUD (D11, M10b faction-registry precedent):** stats and modifiers are `Record<key,…>`. Add a key = single-key Update `old: null`; edit a subfield = single-key Update with the raw old; remove a key = whole-map replace Update `{ old: currentMap, new: mapWithoutKey }`. `set_pointer` cannot grow arrays.
- **Display order is presentation-only (D12):** drag/drop rewrites `order` fields via single-key Updates and can never change any evaluated value. Enforced by the M13b resolver (dependency-ordered) and asserted in Task 11.
- **Tier-1 validation (spec §6, D6):** every stat-key write is validated with M13b's `validateStatKey`; invalid input never dispatches. Formula inputs get live validation via `@shadowcat/formula`'s `parseFormula`; a malformed formula is surfaced as a chip but is still storable (fail-closed at read time).
- **Editability advisory (spec §6):** write controls gate on `ctx.canEdit(doc, path)`. The server stays authoritative; GM ⇒ always editable.
- **i18n-keyed chrome; user data is not localized (spec §6):** all sheet chrome text goes through a namespaced `nfT` key; user-authored stat `label`/`key` and `text`/`resource` values are data, rendered verbatim.
- **44px touch targets, reflow (CLAUDE.md cross-platform directive):** drag handles, toggles, and buttons are ≥44px; sheets reflow to a phone width.
- **Roll buttons are OUT of scope (deferred to M13d):** the M13 §13 checkpoint table assigns per-stat roll templates → chat to **M13d** ("Roll wire"), which gates on M13b+M11, not M13c. M13c renders NO roll affordance. (Spec §6 lists roll buttons descriptively under the eventual actor sheet; §13 is authoritative for checkpoint boundaries — see Spec gaps.)
- **GM-only secret text is OUT of scope (deferred):** spec §12 lists the GM-only-secret authoring affordance as a *candidate* for M13c "or a later sheet pass"; this plan defers it (no new egress/property-override logic) — see Spec gaps.
- **`effect` is a client-semantics doc_type (D9):** exactly as M12c introduced `item`; zero server change. M13c defines `EFFECT_DOC_TYPE = "effect"` (see Task 10 + Spec gaps for its home).
- Commit per task once green (`pnpm --filter nightfox test` + `pnpm --filter nightfox typecheck`).

## File Structure

All paths relative to the Nightfox repo root (`C:\Dev\Nightfox\`). M13-1 Task 18 already created `package.json`, `vite.config.ts`, `tsconfig.json`, `svelte.config.js`, `vitest.config.ts`, `vitest.setup.ts`, `src/index.ts`. M13b already created `src/nightfox-docs.ts`, `src/contributions.ts`, `src/resolve.ts` and their tests. M13c does NOT recreate any of these.

| File | Responsibility | Task |
|---|---|---|
| `src/sheets/nf-i18n.ts` | `nfT` chrome-translation helper with an English fallback map (external module has no shell i18n-registration seam). | 1 |
| `src/sheets/sheet-model.ts` | Pure read helper `sheetView(top, systemPrefix)` (resolve host graph → self doc block + resolved stats + warnings) and the field-path write helpers (stat/modifier add/edit/remove, order, active/transfer, embedded toggles). | 2 |
| `src/sheets/format.ts` | `formatValue(FormulaValue)` display, `formulaIssues(src)` live validation, `warningChips(warnings)` chip descriptors. | 3 |
| `src/sheets/StatRow.svelte` | One resolved stat: type-specific editors + computed preview + error chip. | 4 |
| `src/sheets/StatTable.svelte` | Ordered stat list + add-stat control + presentation-only drag/drop reorder. | 5 |
| `src/sheets/ModifiersEditor.svelte` | Modifier list (target key, op, magnitude) + add/remove + inert/dangling warnings. | 6 |
| `src/sheets/ActorSheet.svelte` | Actor: header + StatTable + inventory list (active toggle, open) + effects list (active/transfer toggle, open). | 7 |
| `src/sheets/ItemSheet.svelte` | Item: name + own StatTable + ModifiersEditor + active toggle + embedded-effects list. | 8 |
| `src/sheets/EffectSheet.svelte` | Effect: name + own StatTable + ModifiersEditor + active + transfer toggles. | 9 |
| `src/index.ts` (modify) | Add `EFFECT_DOC_TYPE`; register the three sheets above the generics; preserve the M13b rules barrel re-exports. | 10 |
| `src/index.test.ts` (modify) | Supersede the M13b/Task-18 registration test with the M13c sheet-registration assertions. | 10 |
| `src/sheets/flow.integration.test.ts` | The spec §11 flow: author stats → derived visible → equip item → value changes → toggle effect → reverts; reorder changes only order; OCC pre-image correctness. | 11 |
| `README.md`, `CHANGELOG.md` (Nightfox) + Shadowcat `docs/PLAN.md`, `docs/POST_WORK_FINDINGS.md`, skill | Documentation sync. | 12 |

---

## Task 1: Sheet folder scaffold + i18n fallback helper

**Files:**
- Create: `src/sheets/nf-i18n.ts`
- Test: `src/sheets/nf-i18n.test.ts`

**Interfaces:**
- Consumes: `AppContext` (`@shadowcat/ui-kit`) — only its `t: TFunc` field.
- Produces: `export function nfT(ctx: { t: (k: string, p?: Record<string, string | number>) => string }, key: string, params?: Record<string, string | number>): string` — returns `ctx.t(key)`, falling back to the built-in English `NF_MESSAGES[key]` when the shell returns the key unchanged (no registration). `export const NF_MESSAGES: Readonly<Record<string, string>>`.

- [ ] **Step 1: Write the failing test** — create `src/sheets/nf-i18n.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { nfT, NF_MESSAGES } from "./nf-i18n";

describe("nfT", () => {
  it("uses the shell translation when the shell resolves the key", () => {
    const ctx = { t: (k: string) => (k === "nightfox.stat.add" ? "Ajouter" : k) };
    expect(nfT(ctx, "nightfox.stat.add")).toBe("Ajouter");
  });

  it("falls back to the built-in English message when the shell returns the key unchanged", () => {
    const ctx = { t: (k: string) => k }; // passthrough shell (key unregistered)
    expect(nfT(ctx, "nightfox.stat.add")).toBe(NF_MESSAGES["nightfox.stat.add"]);
    expect(NF_MESSAGES["nightfox.stat.add"]).toBe("Add stat");
  });

  it("returns the raw key when neither the shell nor the fallback map knows it", () => {
    const ctx = { t: (k: string) => k };
    expect(nfT(ctx, "nightfox.unknown.key")).toBe("nightfox.unknown.key");
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run (from the nested Shadowcat checkout root): `pnpm --filter nightfox test nf-i18n`
Expected: FAIL — `Cannot find module './nf-i18n'`.

- [ ] **Step 3: Implement** — create `src/sheets/nf-i18n.ts`:

```ts
// Chrome-translation helper for the Nightfox sheets. An external module has no
// public seam to register i18n keys into the shell catalog (M13-1 ships none —
// logged as cross-repo friction, POST_WORK_FINDINGS). `nfT` therefore prefers the
// shell's `t` (so a future seam or a host override wins) and falls back to this
// built-in English map when `t` returns the key unchanged (the i18next / test
// "missing key" behavior). User-authored stat labels/keys/values are DATA — never
// routed through here.
export const NF_MESSAGES: Readonly<Record<string, string>> = {
  "nightfox.stats": "Stats",
  "nightfox.stat.add": "Add stat",
  "nightfox.stat.remove": "Remove stat",
  "nightfox.stat.key": "Key",
  "nightfox.stat.type": "Type",
  "nightfox.stat.label": "Label",
  "nightfox.stat.base": "Base",
  "nightfox.stat.formula": "Formula",
  "nightfox.stat.value": "Value",
  "nightfox.stat.current": "Current",
  "nightfox.stat.maxBase": "Max (base)",
  "nightfox.stat.maxFormula": "Max (formula)",
  "nightfox.stat.computed": "Computed",
  "nightfox.stat.reorder": "Drag to reorder",
  "nightfox.type.number": "Number",
  "nightfox.type.resource": "Resource",
  "nightfox.type.text": "Text",
  "nightfox.type.boolean": "Flag",
  "nightfox.modifiers": "Modifiers",
  "nightfox.modifier.add": "Add modifier",
  "nightfox.modifier.remove": "Remove modifier",
  "nightfox.modifier.stat": "Target stat",
  "nightfox.modifier.op": "Operation",
  "nightfox.modifier.value": "Magnitude",
  "nightfox.op.add": "Add",
  "nightfox.op.mulAdditive": "Multiply (additive)",
  "nightfox.op.mulCompound": "Multiply (compound)",
  "nightfox.active": "Active",
  "nightfox.transfer": "Transfer to owner",
  "nightfox.inventory": "Inventory",
  "nightfox.effects": "Effects",
  "nightfox.warn.inertMissing": "Modifier targets a missing stat (inert)",
  "nightfox.warn.inertUnmodifiable": "Modifier targets a text/flag stat (inert)",
  "nightfox.warn.hostInert": "Modifiers on this document are inert here",
  "nightfox.warn.dangling": "Modifiers are dangling (document is not embedded)",
  "nightfox.badKey": "Invalid stat key",
  "nightfox.missing": "No data",
  "nightfox.name": "Name",
};

export function nfT(
  ctx: { t: (k: string, p?: Record<string, string | number>) => string },
  key: string,
  params?: Record<string, string | number>,
): string {
  const shell = ctx.t(key, params);
  if (shell !== key) return shell; // shell resolved it (or a host override)
  return NF_MESSAGES[key] ?? key; // built-in English fallback, else the raw key
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter nightfox test nf-i18n`
Expected: PASS (3 tests).
Run: `pnpm --filter nightfox typecheck`
Expected: clean.

- [ ] **Step 5: Commit (inside the Nightfox repo)**

```bash
cd "C:\Dev\Nightfox"
git add src/sheets/nf-i18n.ts src/sheets/nf-i18n.test.ts
git commit -m "feat(sheets): nfT chrome-translation helper with English fallback map"
```

---

## Task 2: Sheet model — read helper + field-path write helpers

**Files:**
- Create: `src/sheets/sheet-model.ts`
- Test: `src/sheets/sheet-model.test.ts`

**Interfaces:**
- Consumes:
  - `parseNightfox`, `type NightfoxBlock`, `type Stat`, `type Modifier` from `../nightfox-docs` (M13b Task 2).
  - `resolveNightfox`, `type ResolvedStat`, `type ResolveWarning` from `../resolve` (M13b Task 4); `type NightfoxWarning` from `../contributions` (M13b Task 3).
  - `getPointer`, `type WireDocument` from `@shadowcat/core`.
  - `setField`, `type AppContext` from `@shadowcat/ui-kit` (M12c `sheetEdit.ts` / `appContext.ts`).
- Produces:
```ts
export type NfWarning = ResolveWarning | NightfoxWarning;
export interface SheetView {
  selfId: string;
  block: NightfoxBlock | null;
  stats: Map<string, ResolvedStat>;
  warnings: NfWarning[];
}
export function sheetView(top: WireDocument | undefined, systemPrefix: string): SheetView;
export function statsPath(systemPrefix: string): string;
export function modifiersPath(systemPrefix: string): string;
export function addStat(ctx: AppContext, docId: string, systemPrefix: string, key: string, stat: Stat): void;
export function editStatField(ctx: AppContext, docId: string, systemPrefix: string, key: string, field: string, old: unknown, value: unknown): void;
export function removeStat(ctx: AppContext, docId: string, systemPrefix: string, key: string, currentStats: Record<string, Stat>): void;
export function setStatOrder(ctx: AppContext, docId: string, systemPrefix: string, key: string, old: number, value: number): void;
export function addModifier(ctx: AppContext, docId: string, systemPrefix: string, id: string, modifier: Modifier): void;
export function editModifierField(ctx: AppContext, docId: string, systemPrefix: string, id: string, field: keyof Modifier, old: unknown, value: unknown): void;
export function removeModifier(ctx: AppContext, docId: string, systemPrefix: string, id: string, currentMods: Record<string, Modifier>): void;
export function setMechanicsFlag(ctx: AppContext, docId: string, systemPrefix: string, flag: "active" | "transfer", old: boolean | undefined, value: boolean): void;
```

- [ ] **Step 1: Write the failing test** — create `src/sheets/sheet-model.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import type { WireDocument } from "@shadowcat/core";
import {
  sheetView, statsPath, modifiersPath, addStat, editStatField, removeStat,
  setStatOrder, addModifier, removeModifier, editModifierField, setMechanicsFlag,
} from "./sheet-model";
import type { Stat } from "../nightfox-docs";
import type { AppContext } from "@shadowcat/ui-kit";

const num = (order: number, base: number, formula?: string): Stat =>
  ({ type: "number", order, base, ...(formula ? { formula } : {}) });
const sys = (stats: Record<string, unknown> = {}, mech: Record<string, unknown> = {}) =>
  ({ stats, mechanics: { version: 1, ...mech } });
const doc = (id: string, doc_type: string, system: unknown, embedded: Record<string, unknown[]> = {}) =>
  ({ id, doc_type, system, embedded }) as unknown as WireDocument;

/** Capture the ops a write helper dispatches. */
function capture() {
  const calls: unknown[][] = [];
  const ctx = { dispatchIntent: (ops: unknown[]) => calls.push(ops) } as unknown as AppContext;
  return { ctx, calls };
}

describe("sheetView", () => {
  it("resolves an actor host: self stats include modifier effects from an embedded item", () => {
    const top = doc("A", "actor", sys({ str: num(0, undefined), attack: num(1, "str + 1") }), {
      item: [doc("I", "item", sys({}, { modifiers: { m: { stat: "str", op: "add", value: 2 } } }))],
    });
    // str: base 0 with a +2 belt -> final 2; attack = final(str)+1 = 3.
    (top.system as { stats: Record<string, Stat> }).stats.str = num(0, undefined);
    (top.system as { stats: Record<string, Stat> }).stats.str.base = 0;
    const v = sheetView(top, "/system");
    expect(v.selfId).toBe("A");
    const attack = v.stats.get("attack");
    expect(attack && "final" in attack ? attack.final : undefined).toBe(3);
  });

  it("resolves an embedded item's own stats via the top-level host (belt flows to actor, not item)", () => {
    const top = doc("A", "actor", sys(), {
      item: [doc("I", "item", sys({ damage: num(0, undefined) }), {
        effect: [doc("E", "effect", sys({}, { modifiers: { m: { stat: "damage", op: "add", value: 1 } } }))],
      })],
    });
    (top.embedded!.item![0] as unknown as { system: { stats: Record<string, Stat> } }).system.stats.damage.base = 2;
    const v = sheetView(top, "/embedded/item/0/system");
    expect(v.selfId).toBe("I");
    const dmg = v.stats.get("damage");
    expect(dmg && "final" in dmg ? dmg.final : undefined).toBe(3); // 2 + effect(+1)
  });

  it("returns an empty view for a non-Nightfox / missing doc", () => {
    expect(sheetView(undefined, "/system")).toEqual({ selfId: "", block: null, stats: new Map(), warnings: [] });
    const bare = doc("B", "actor", { grid: {} });
    const v = sheetView(bare, "/system");
    expect(v.block).toBeNull();
    expect(v.stats.size).toBe(0);
  });

  it("filters warnings to the self doc", () => {
    const top = doc("A", "actor", sys(), {
      item: [doc("I", "item", sys({}, { modifiers: { m: { stat: "ghost", op: "add", value: 1 } } }))],
    });
    const v = sheetView(top, "/system");
    // The inert-missing warning is attributed to the actor (target of the modifier).
    expect(v.warnings.every((w) => w.docId === "A")).toBe(true);
  });
});

describe("path builders", () => {
  it("build the stats and modifiers directories under a system prefix", () => {
    expect(statsPath("/system")).toBe("/system/stats");
    expect(modifiersPath("/embedded/item/0/system")).toBe("/embedded/item/0/system/mechanics/modifiers");
  });
});

describe("stat write helpers", () => {
  it("addStat writes a single-key add with old:null", () => {
    const { ctx, calls } = capture();
    const s = num(0, 3);
    addStat(ctx, "A", "/system", "dex", s);
    expect(calls).toEqual([[{ op: "update", doc_id: "A", changes: [{ path: "/system/stats/dex", old: null, new: s }] }]]);
  });

  it("editStatField writes the raw old pre-image at the field path", () => {
    const { ctx, calls } = capture();
    editStatField(ctx, "A", "/system", "dex", "base", 3, 4);
    expect(calls).toEqual([[{ op: "update", doc_id: "A", changes: [{ path: "/system/stats/dex/base", old: 3, new: 4 }] }]]);
  });

  it("removeStat replaces the whole stats map with the map as the pre-image", () => {
    const { ctx, calls } = capture();
    const current = { dex: num(0, 3), str: num(1, 2) };
    removeStat(ctx, "A", "/system", "dex", current);
    expect(calls).toEqual([[{ op: "update", doc_id: "A", changes: [{ path: "/system/stats", old: current, new: { str: current.str } }] }]]);
  });

  it("setStatOrder writes the order field with its raw old", () => {
    const { ctx, calls } = capture();
    setStatOrder(ctx, "A", "/system", "dex", 0, 2);
    expect(calls).toEqual([[{ op: "update", doc_id: "A", changes: [{ path: "/system/stats/dex/order", old: 0, new: 2 }] }]]);
  });
});

describe("modifier + mechanics write helpers", () => {
  it("addModifier / removeModifier follow the map-key add / whole-map-replace idiom", () => {
    const { ctx, calls } = capture();
    const m = { stat: "dex", op: "add" as const, value: 1 };
    addModifier(ctx, "I", "/system", "m1", m);
    const current = { m1: m };
    removeModifier(ctx, "I", "/system", "m1", current);
    expect(calls[0]).toEqual([{ op: "update", doc_id: "I", changes: [{ path: "/system/mechanics/modifiers/m1", old: null, new: m }] }]);
    expect(calls[1]).toEqual([{ op: "update", doc_id: "I", changes: [{ path: "/system/mechanics/modifiers", old: current, new: {} }] }]);
  });

  it("editModifierField writes the raw old at the modifier field", () => {
    const { ctx, calls } = capture();
    editModifierField(ctx, "I", "/system", "m1", "op", "add", "mulCompound");
    expect(calls).toEqual([[{ op: "update", doc_id: "I", changes: [{ path: "/system/mechanics/modifiers/m1/op", old: "add", new: "mulCompound" }] }]]);
  });

  it("setMechanicsFlag writes active/transfer with old:null when the flag was absent", () => {
    const { ctx, calls } = capture();
    setMechanicsFlag(ctx, "E", "/system", "transfer", undefined, true);
    setMechanicsFlag(ctx, "E", "/system", "active", true, false);
    expect(calls[0]).toEqual([{ op: "update", doc_id: "E", changes: [{ path: "/system/mechanics/transfer", old: null, new: true }] }]);
    expect(calls[1]).toEqual([{ op: "update", doc_id: "E", changes: [{ path: "/system/mechanics/active", old: true, new: false }] }]);
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter nightfox test sheet-model`
Expected: FAIL — `Cannot find module './sheet-model'`.

- [ ] **Step 3: Implement** — create `src/sheets/sheet-model.ts`:

```ts
// Read + write model shared by every Nightfox sheet.
//
// READ: `sheetView` always resolves from the TOP-LEVEL host document, then extracts
// the self doc's slice by id. This is load-bearing: an item's belt reaches the actor
// only when the actor is the resolve host (M13b §5.3) — resolving the embedded item in
// isolation would drop transfer/parent flow. The self doc is located by the JSON
// pointer that `resolveDocRef` produced as `writePrefix` (systemPrefix): `/system` = the
// top doc itself; `/embedded/<coll>/<i>/system` = an embedded child.
//
// WRITE: every mutation is a field-path Update carrying the RAW stored `old` pre-image
// (OCC; [[sheet-reactive-bridge-missing-subscription]]). Maps mutate by the D11 idiom —
// add = single-key `old:null`; edit = single-key raw-old; remove = whole-map replace with
// the current map as the pre-image (`set_pointer` cannot delete a key in place).
import { getPointer, type WireDocument } from "@shadowcat/core";
import { setField, type AppContext } from "@shadowcat/ui-kit";
import { parseNightfox, type NightfoxBlock, type Stat, type Modifier } from "../nightfox-docs";
import { resolveNightfox, type ResolvedStat, type ResolveWarning } from "../resolve";
import type { NightfoxWarning } from "../contributions";

export type NfWarning = ResolveWarning | NightfoxWarning;

export interface SheetView {
  selfId: string;
  block: NightfoxBlock | null;
  stats: Map<string, ResolvedStat>;
  warnings: NfWarning[];
}

export function sheetView(top: WireDocument | undefined, systemPrefix: string): SheetView {
  const empty: SheetView = { selfId: "", block: null, stats: new Map(), warnings: [] };
  if (!top || typeof top !== "object") return empty;
  const basePrefix = systemPrefix.replace(/\/system$/, "");
  const selfDoc = (basePrefix === "" ? top : getPointer(top, basePrefix)) as WireDocument | undefined;
  if (!selfDoc || typeof selfDoc !== "object" || typeof selfDoc.id !== "string") return empty;
  const resolved = resolveNightfox(top);
  const stats = resolved.byDoc.get(selfDoc.id) ?? new Map<string, ResolvedStat>();
  const warnings = resolved.warnings.filter((w) => w.docId === selfDoc.id);
  return { selfId: selfDoc.id, block: parseNightfox(selfDoc), stats, warnings };
}

export function statsPath(systemPrefix: string): string {
  return `${systemPrefix}/stats`;
}
export function modifiersPath(systemPrefix: string): string {
  return `${systemPrefix}/mechanics/modifiers`;
}

export function addStat(ctx: AppContext, docId: string, systemPrefix: string, key: string, stat: Stat): void {
  setField(ctx, docId, `${statsPath(systemPrefix)}/${key}`, null, stat);
}
export function editStatField(ctx: AppContext, docId: string, systemPrefix: string, key: string, field: string, old: unknown, value: unknown): void {
  setField(ctx, docId, `${statsPath(systemPrefix)}/${key}/${field}`, old, value);
}
export function removeStat(ctx: AppContext, docId: string, systemPrefix: string, key: string, currentStats: Record<string, Stat>): void {
  const next: Record<string, Stat> = { ...currentStats };
  delete next[key];
  setField(ctx, docId, statsPath(systemPrefix), currentStats, next);
}
export function setStatOrder(ctx: AppContext, docId: string, systemPrefix: string, key: string, old: number, value: number): void {
  setField(ctx, docId, `${statsPath(systemPrefix)}/${key}/order`, old, value);
}

export function addModifier(ctx: AppContext, docId: string, systemPrefix: string, id: string, modifier: Modifier): void {
  setField(ctx, docId, `${modifiersPath(systemPrefix)}/${id}`, null, modifier);
}
export function editModifierField(ctx: AppContext, docId: string, systemPrefix: string, id: string, field: keyof Modifier, old: unknown, value: unknown): void {
  setField(ctx, docId, `${modifiersPath(systemPrefix)}/${id}/${field}`, old, value);
}
export function removeModifier(ctx: AppContext, docId: string, systemPrefix: string, id: string, currentMods: Record<string, Modifier>): void {
  const next: Record<string, Modifier> = { ...currentMods };
  delete next[id];
  setField(ctx, docId, modifiersPath(systemPrefix), currentMods, next);
}

export function setMechanicsFlag(ctx: AppContext, docId: string, systemPrefix: string, flag: "active" | "transfer", old: boolean | undefined, value: boolean): void {
  setField(ctx, docId, `${systemPrefix}/mechanics/${flag}`, old ?? null, value);
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter nightfox test sheet-model`
Expected: PASS.
Run: `pnpm --filter nightfox typecheck`
Expected: clean.

- [ ] **Step 5: Commit (inside the Nightfox repo)**

```bash
cd "C:\Dev\Nightfox"
git add src/sheets/sheet-model.ts src/sheets/sheet-model.test.ts
git commit -m "feat(sheets): sheetView read helper + field-path stat/modifier write helpers"
```

---

## Task 3: Format helpers — value display, live formula validation, warning chips

**Files:**
- Create: `src/sheets/format.ts`
- Test: `src/sheets/format.test.ts`

**Interfaces:**
- Consumes: `isFormulaError`, `parseFormula`, `type FormulaValue` from `@shadowcat/formula` (M13a Tasks 1/3); `type NfWarning` from `./sheet-model` (Task 2).
- Produces:
```ts
export interface DisplayValue { text: string; error: boolean; title?: string }
export function formatValue(v: FormulaValue | undefined): DisplayValue;
export function formulaIssues(src: string): string[];   // [] = parseable/empty; else one detail string
export interface ChipDescriptor { messageKey: string; detail: string }
export function warningChips(warnings: NfWarning[]): ChipDescriptor[];
```

- [ ] **Step 1: Write the failing test** — create `src/sheets/format.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { formatValue, formulaIssues, warningChips } from "./format";

describe("formatValue", () => {
  it("renders a finite number, trimming float noise", () => {
    expect(formatValue(3)).toEqual({ text: "3", error: false });
    expect(formatValue(2.4000000001)).toEqual({ text: "2.4", error: false });
  });
  it("renders undefined as an em dash, not an error", () => {
    expect(formatValue(undefined)).toEqual({ text: "—", error: false });
  });
  it("renders a FormulaError as an error chip carrying its detail as the title", () => {
    const d = formatValue({ error: "cycle", detail: "a -> b -> a" });
    expect(d.error).toBe(true);
    expect(d.text).toContain("cycle");
    expect(d.title).toBe("a -> b -> a");
  });
});

describe("formulaIssues", () => {
  it("returns [] for an empty or valid formula", () => {
    expect(formulaIssues("")).toEqual([]);
    expect(formulaIssues("  ")).toEqual([]);
    expect(formulaIssues("dex + floor(str / 2)")).toEqual([]);
  });
  it("returns one issue for a malformed formula", () => {
    const issues = formulaIssues("dex + ");
    expect(issues.length).toBe(1);
    expect(typeof issues[0]).toBe("string");
  });
});

describe("warningChips", () => {
  it("maps every resolver/collection warning kind to a message key", () => {
    const chips = warningChips([
      { docId: "A", kind: "inert-missing-stat", detail: "ghost" },
      { docId: "A", kind: "inert-unmodifiable-stat", detail: "class" },
      { docId: "A", kind: "host-modifiers-inert", detail: "m1" },
      { docId: "I", kind: "dangling-modifiers", detail: "m1" },
    ]);
    expect(chips.map((c) => c.messageKey)).toEqual([
      "nightfox.warn.inertMissing",
      "nightfox.warn.inertUnmodifiable",
      "nightfox.warn.hostInert",
      "nightfox.warn.dangling",
    ]);
    expect(chips[0].detail).toBe("ghost");
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter nightfox test format`
Expected: FAIL — `Cannot find module './format'`.

- [ ] **Step 3: Implement** — create `src/sheets/format.ts`:

```ts
// Presentation helpers for the sheets. Pure; no Svelte, no ctx.
import { isFormulaError, parseFormula, type FormulaValue } from "@shadowcat/formula";
import type { NfWarning } from "./sheet-model";

export interface DisplayValue {
  text: string;
  error: boolean;
  title?: string;
}

/** Display a resolved value. A FormulaError renders as a visible error chip carrying its
 * `detail` as the hover title; `undefined` (unresolved / non-numeric slot) is a neutral
 * em dash. Finite numbers are rounded to 6 decimals for DISPLAY only — the stored/evaluated
 * value is never mutated (D4 magnitudes are exact). */
export function formatValue(v: FormulaValue | undefined): DisplayValue {
  if (v === undefined) return { text: "—", error: false };
  if (isFormulaError(v)) return { text: `⚠ ${v.error}`, error: true, title: v.detail };
  const rounded = Math.round(v * 1e6) / 1e6;
  return { text: String(rounded), error: false };
}

/** Live validation for a formula input. Empty is valid (the field is optional). A parse
 * failure yields exactly one human-readable issue; the value is still storable (readers
 * fail closed at eval time) — the chip is advisory. */
export function formulaIssues(src: string): string[] {
  if (src.trim() === "") return [];
  const ast = parseFormula(src);
  return isFormulaError(ast as never) ? [(ast as { detail: string }).detail] : [];
}

export interface ChipDescriptor {
  messageKey: string;
  detail: string;
}

const WARNING_KEY: Record<NfWarning["kind"], string> = {
  "inert-missing-stat": "nightfox.warn.inertMissing",
  "inert-unmodifiable-stat": "nightfox.warn.inertUnmodifiable",
  "host-modifiers-inert": "nightfox.warn.hostInert",
  "dangling-modifiers": "nightfox.warn.dangling",
};

export function warningChips(warnings: NfWarning[]): ChipDescriptor[] {
  return warnings.map((w) => ({ messageKey: WARNING_KEY[w.kind], detail: w.detail }));
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter nightfox test format`
Expected: PASS.
Run: `pnpm --filter nightfox typecheck`
Expected: clean.

- [ ] **Step 5: Commit (inside the Nightfox repo)**

```bash
cd "C:\Dev\Nightfox"
git add src/sheets/format.ts src/sheets/format.test.ts
git commit -m "feat(sheets): value formatting, live formula validation, warning chip descriptors"
```

---

## Task 4: `StatRow.svelte` — one resolved stat's editors + computed preview

**Files:**
- Create: `src/sheets/StatRow.svelte`
- Test: `src/sheets/StatRow.test.ts`

**Interfaces:**
- Consumes: `getAppContext` (`@shadowcat/ui-kit`); `editStatField` (Task 2); `formatValue`, `formulaIssues` (Task 3); `nfT` (Task 1); `type Stat` (`../nightfox-docs`), `type ResolvedStat` (`../resolve`).
- Produces: a Svelte component with props `{ docId: string; systemPrefix: string; statKey: string; stat: Stat; resolved: ResolvedStat | undefined; readOnly: boolean; onRemove: () => void }`. Renders type-specific editable fields (each an `onchange` → `editStatField` with the raw old), a computed-value preview, an error chip for an errored formula/final, a live formula-validity chip, and a 44px remove button. Emits reorder via a drag handle element carrying `data-stat-key` (StatTable orchestrates the drop).

- [ ] **Step 1: Write the failing test** — create `src/sheets/StatRow.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import StatRow from "./StatRow.svelte";
import type { Stat } from "../nightfox-docs";
import type { ResolvedStat } from "../resolve";

function mount(over: {
  statKey?: string; stat: Stat; resolved?: ResolvedStat; readOnly?: boolean;
  dispatchIntent?: (ops: unknown[]) => void; onRemove?: () => void;
}) {
  const context = setAppContextForTest({ dispatchIntent: over.dispatchIntent ?? (() => {}), canEdit: () => true });
  return render(StatRow, {
    props: {
      docId: "A", systemPrefix: "/system", statKey: over.statKey ?? "dex",
      stat: over.stat, resolved: over.resolved, readOnly: over.readOnly ?? false,
      onRemove: over.onRemove ?? (() => {}),
    },
    context,
  });
}

describe("StatRow — number", () => {
  const stat: Stat = { type: "number", order: 0, base: 3, formula: "dex + 1" };
  const resolved: ResolvedStat = { type: "number", order: 0, base: 3, final: 4 };

  it("shows the computed final value", () => {
    const { getByText } = mount({ stat, resolved });
    expect(getByText("4")).toBeTruthy();
  });

  it("edits base with the raw old pre-image", async () => {
    const calls: unknown[][] = [];
    const { getByLabelText } = mount({ stat, resolved, dispatchIntent: (o) => calls.push(o) });
    await fireEvent.change(getByLabelText("nightfox.stat.base"), { target: { value: "5" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "A", changes: [{ path: "/system/stats/dex/base", old: 3, new: 5 }] }]]);
  });

  it("edits the formula string with the raw old", async () => {
    const calls: unknown[][] = [];
    const { getByLabelText } = mount({ stat, resolved, dispatchIntent: (o) => calls.push(o) });
    await fireEvent.change(getByLabelText("nightfox.stat.formula"), { target: { value: "dex + 2" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "A", changes: [{ path: "/system/stats/dex/formula", old: "dex + 1", new: "dex + 2" }] }]]);
  });

  it("shows an error chip when the resolved final is a FormulaError", () => {
    const { getByText } = mount({ stat, resolved: { type: "number", order: 0, base: 3, final: { error: "cycle", detail: "d -> d" } } });
    expect(getByText(/cycle/)).toBeTruthy();
  });

  it("disables inputs when readOnly", () => {
    const { getByLabelText } = mount({ stat, resolved, readOnly: true });
    expect((getByLabelText("nightfox.stat.base") as HTMLInputElement).disabled).toBe(true);
  });
});

describe("StatRow — resource", () => {
  const stat: Stat = { type: "resource", order: 0, current: 8, maxBase: 10, maxFormula: "10 + str" };
  const resolved: ResolvedStat = { type: "resource", order: 0, current: 8, max: 12, effectiveCurrent: 8 };

  it("shows current and computed max", () => {
    const { getByText, getByLabelText } = mount({ stat, resolved });
    expect((getByLabelText("nightfox.stat.current") as HTMLInputElement).value).toBe("8");
    expect(getByText("12")).toBeTruthy();
  });

  it("edits current with the raw old", async () => {
    const calls: unknown[][] = [];
    const { getByLabelText } = mount({ stat, resolved, dispatchIntent: (o) => calls.push(o) });
    await fireEvent.change(getByLabelText("nightfox.stat.current"), { target: { value: "6" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "A", changes: [{ path: "/system/stats/dex/current", old: 8, new: 6 }] }]]);
  });
});

describe("StatRow — boolean / text", () => {
  it("toggles a boolean with the raw old", async () => {
    const calls: unknown[][] = [];
    const stat: Stat = { type: "boolean", order: 0, value: false };
    const { getByLabelText } = mount({ stat, resolved: { type: "boolean", order: 0, value: false }, dispatchIntent: (o) => calls.push(o) });
    await fireEvent.click(getByLabelText("nightfox.stat.value"));
    expect(calls).toEqual([[{ op: "update", doc_id: "A", changes: [{ path: "/system/stats/dex/value", old: false, new: true }] }]]);
  });

  it("edits text value with the raw old", async () => {
    const calls: unknown[][] = [];
    const stat: Stat = { type: "text", order: 0, value: "ranger" };
    const { getByLabelText } = mount({ stat, resolved: { type: "text", order: 0, value: "ranger" }, dispatchIntent: (o) => calls.push(o) });
    await fireEvent.change(getByLabelText("nightfox.stat.value"), { target: { value: "wizard" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "A", changes: [{ path: "/system/stats/dex/value", old: "ranger", new: "wizard" }] }]]);
  });
});

describe("StatRow — remove", () => {
  it("calls onRemove from the remove button", async () => {
    let removed = false;
    const stat: Stat = { type: "number", order: 0, base: 1 };
    const { getByLabelText } = mount({ stat, resolved: { type: "number", order: 0, base: 1, final: 1 }, onRemove: () => (removed = true) });
    await fireEvent.click(getByLabelText("nightfox.stat.remove"));
    expect(removed).toBe(true);
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter nightfox test StatRow`
Expected: FAIL — `Cannot find module './StatRow.svelte'`.

- [ ] **Step 3: Implement** — create `src/sheets/StatRow.svelte`:

```svelte
<script lang="ts">
  import { getAppContext, type AppContext } from "@shadowcat/ui-kit";
  import { editStatField } from "./sheet-model";
  import { formatValue, formulaIssues } from "./format";
  import { nfT } from "./nf-i18n";
  import type { Stat } from "../nightfox-docs";
  import type { ResolvedStat } from "../resolve";

  // One stat row. Editable fields write single-key field-path Updates carrying the RAW
  // stored old (`stat.<field>`); the computed preview reads the already-resolved value
  // (parent passed it in — the whole graph is resolved once per render up-tree). Formula
  // inputs get live parse validation (advisory chip; the value is still storable and fails
  // closed at read time). No roll affordance here — rolls are M13d.
  let { docId, systemPrefix, statKey, stat, resolved, readOnly, onRemove }:
    { docId: string; systemPrefix: string; statKey: string; stat: Stat;
      resolved: ResolvedStat | undefined; readOnly: boolean; onRemove: () => void } = $props();

  const ctx: AppContext = getAppContext();
  const t = (k: string) => nfT(ctx, k);

  function edit(field: string, old: unknown, value: unknown): void {
    editStatField(ctx, docId, systemPrefix, statKey, field, old, value);
  }
  function editNumber(field: string, old: number, raw: string): void {
    const n = Number(raw);
    if (!Number.isFinite(n)) return;
    edit(field, old, n);
  }

  const preview = $derived.by(() => {
    if (!resolved) return formatValue(undefined);
    if (resolved.type === "number") return formatValue(resolved.final);
    if (resolved.type === "resource") return formatValue(resolved.max);
    return formatValue(undefined);
  });
  const formulaErr = $derived(
    stat.type === "number" ? formulaIssues(stat.formula ?? "")
    : stat.type === "resource" ? formulaIssues(stat.maxFormula ?? "")
    : [],
  );
</script>

<div class="stat-row">
  <span class="handle" data-stat-key={statKey} title={t("nightfox.stat.reorder")} aria-label={t("nightfox.stat.reorder")} draggable={!readOnly}>⠿</span>
  <span class="key">{stat.label ?? statKey}</span>

  {#if stat.type === "number"}
    <label>{t("nightfox.stat.base")}
      <input type="number" step="any" aria-label={t("nightfox.stat.base")} value={stat.base} disabled={readOnly}
        onchange={(e) => editNumber("base", stat.base, (e.currentTarget as HTMLInputElement).value)} /></label>
    <label>{t("nightfox.stat.formula")}
      <input aria-label={t("nightfox.stat.formula")} value={stat.formula ?? ""} disabled={readOnly}
        onchange={(e) => edit("formula", stat.formula ?? null, (e.currentTarget as HTMLInputElement).value || null)} /></label>
  {:else if stat.type === "resource"}
    <label>{t("nightfox.stat.current")}
      <input type="number" step="any" aria-label={t("nightfox.stat.current")} value={stat.current} disabled={readOnly}
        onchange={(e) => editNumber("current", stat.current, (e.currentTarget as HTMLInputElement).value)} /></label>
    <label>{t("nightfox.stat.maxBase")}
      <input type="number" step="any" aria-label={t("nightfox.stat.maxBase")} value={stat.maxBase} disabled={readOnly}
        onchange={(e) => editNumber("maxBase", stat.maxBase, (e.currentTarget as HTMLInputElement).value)} /></label>
    <label>{t("nightfox.stat.maxFormula")}
      <input aria-label={t("nightfox.stat.maxFormula")} value={stat.maxFormula ?? ""} disabled={readOnly}
        onchange={(e) => edit("maxFormula", stat.maxFormula ?? null, (e.currentTarget as HTMLInputElement).value || null)} /></label>
  {:else if stat.type === "boolean"}
    <label class="flag">{t("nightfox.stat.value")}
      <input type="checkbox" aria-label={t("nightfox.stat.value")} checked={stat.value} disabled={readOnly}
        onchange={(e) => edit("value", stat.value, (e.currentTarget as HTMLInputElement).checked)} /></label>
  {:else}
    <label>{t("nightfox.stat.value")}
      <input aria-label={t("nightfox.stat.value")} value={stat.value} disabled={readOnly}
        onchange={(e) => edit("value", stat.value, (e.currentTarget as HTMLInputElement).value)} /></label>
  {/if}

  {#if stat.type === "number" || stat.type === "resource"}
    <span class="preview" class:err={preview.error} title={preview.title}>{t("nightfox.stat.computed")}: {preview.text}</span>
  {/if}
  {#each formulaErr as issue}<span class="chip err" title={issue}>{issue}</span>{/each}

  <button type="button" class="remove" aria-label={t("nightfox.stat.remove")} disabled={readOnly} onclick={onRemove}>×</button>
</div>

<style lang="scss">
  .stat-row { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-1); padding: var(--space-1) 0; border-bottom: 1px solid var(--border); }
  .handle { min-width: 44px; min-height: 44px; display: inline-flex; align-items: center; justify-content: center; cursor: grab; touch-action: none; }
  .key { font-weight: 600; min-width: 6ch; }
  label { display: flex; flex-direction: column; gap: 2px; }
  label.flag { flex-direction: row; align-items: center; gap: var(--space-1); }
  input[type="checkbox"] { min-width: 44px; min-height: 44px; }
  .preview { font-family: monospace; }
  .preview.err, .chip.err { color: var(--danger, #c00); }
  .chip { border: 1px solid var(--danger, #c00); border-radius: var(--radius-1); padding: 0 var(--space-1); font-size: 0.85em; }
  .remove { min-width: 44px; min-height: 44px; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); }
  .remove:focus-visible { outline: 2px solid var(--accent); }
</style>
```

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter nightfox test StatRow`
Expected: PASS.
Run: `pnpm --filter nightfox typecheck`
Expected: clean.

- [ ] **Step 5: Commit (inside the Nightfox repo)**

```bash
cd "C:\Dev\Nightfox"
git add src/sheets/StatRow.svelte src/sheets/StatRow.test.ts
git commit -m "feat(sheets): StatRow — per-type stat editors + computed preview + error chips"
```

---

## Task 5: `StatTable.svelte` — ordered stat list + add-stat + presentation-only reorder

**Files:**
- Create: `src/sheets/StatTable.svelte`
- Test: `src/sheets/StatTable.test.ts`

**Interfaces:**
- Consumes: `getAppContext` (`@shadowcat/ui-kit`); `addStat`, `removeStat`, `setStatOrder` (Task 2); `nfT` (Task 1); `StatRow` (Task 4); `validateStatKey`, `type Stat`, `type StatType` (`../nightfox-docs`, M13b Task 2); `type ResolvedStat` (`../resolve`).
- Produces: a Svelte component with props `{ docId: string; systemPrefix: string; stats: Record<string, Stat>; resolved: Map<string, ResolvedStat>; readOnly: boolean }`. Renders stats sorted by `order` (ties by key) as `StatRow`s; an add-stat control (key text input validated by `validateStatKey`, type `<select>`, add button — a default-shaped stat for the chosen type is written with `order` = current count); and HTML5 drag/drop that rewrites only the affected `order` fields (D12 — never a value).

- [ ] **Step 1: Write the failing test** — create `src/sheets/StatTable.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import StatTable from "./StatTable.svelte";
import type { Stat } from "../nightfox-docs";
import type { ResolvedStat } from "../resolve";

const number = (order: number, base: number): Stat => ({ type: "number", order, base });
const rnum = (order: number, base: number, final: number): ResolvedStat => ({ type: "number", order, base, final });

function mount(stats: Record<string, Stat>, resolved: Map<string, ResolvedStat>, dispatchIntent?: (o: unknown[]) => void) {
  const context = setAppContextForTest({ dispatchIntent: dispatchIntent ?? (() => {}), canEdit: () => true });
  return render(StatTable, { props: { docId: "A", systemPrefix: "/system", stats, resolved, readOnly: false }, context });
}

describe("StatTable", () => {
  it("renders stats in ascending order", () => {
    const stats = { str: number(1, 2), dex: number(0, 3) };
    const resolved = new Map<string, ResolvedStat>([["str", rnum(1, 2, 2)], ["dex", rnum(0, 3, 3)]]);
    const { getAllByText } = mount(stats, resolved);
    const keys = getAllByText(/^(dex|str)$/).map((n) => n.textContent);
    expect(keys).toEqual(["dex", "str"]);
  });

  it("adds a new number stat with order = current count and old:null", async () => {
    const calls: unknown[][] = [];
    const { getByLabelText, getByText } = mount({ dex: number(0, 3) }, new Map([["dex", rnum(0, 3, 3)]]), (o) => calls.push(o));
    await fireEvent.change(getByLabelText("nightfox.stat.key"), { target: { value: "str" } });
    await fireEvent.click(getByText("nightfox.stat.add"));
    expect(calls).toEqual([[{ op: "update", doc_id: "A", changes: [
      { path: "/system/stats/str", old: null, new: { type: "number", order: 1, base: 0 } },
    ] }]]);
  });

  it("blocks an invalid stat key (does not dispatch)", async () => {
    const calls: unknown[][] = [];
    const { getByLabelText, getByText } = mount({}, new Map(), (o) => calls.push(o));
    await fireEvent.change(getByLabelText("nightfox.stat.key"), { target: { value: "parent" } }); // reserved
    await fireEvent.click(getByText("nightfox.stat.add"));
    expect(calls).toEqual([]);
    expect(getByText("nightfox.badKey")).toBeTruthy();
  });

  it("reorder writes only order fields (D12), never a value", async () => {
    const calls: unknown[][] = [];
    const stats = { dex: number(0, 3), str: number(1, 2), con: number(2, 1) };
    const resolved = new Map<string, ResolvedStat>([["dex", rnum(0, 3, 3)], ["str", rnum(1, 2, 2)], ["con", rnum(2, 1, 1)]]);
    const { container } = mount(stats, resolved, (o) => calls.push(o));
    const handles = container.querySelectorAll<HTMLElement>(".handle");
    // Drag "con" (index 2) onto "dex" (index 0): new key order [con, dex, str].
    await fireEvent.dragStart(handles[2]);
    await fireEvent.drop(handles[0]);
    // Every dispatched change touches a `/order` path only.
    const paths = calls.flat().flatMap((op) => (op as { changes: { path: string }[] }).changes.map((c) => c.path));
    expect(paths.length).toBeGreaterThan(0);
    expect(paths.every((p) => p.endsWith("/order"))).toBe(true);
    // The new orders reflect [con, dex, str].
    const byPath = Object.fromEntries(calls.flat().flatMap((op) =>
      (op as { changes: { path: string; new: unknown }[] }).changes.map((c) => [c.path, c.new])));
    expect(byPath["/system/stats/con/order"]).toBe(0);
    expect(byPath["/system/stats/dex/order"]).toBe(1);
    expect(byPath["/system/stats/str/order"]).toBe(2);
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter nightfox test StatTable`
Expected: FAIL — `Cannot find module './StatTable.svelte'`.

- [ ] **Step 3: Implement** — create `src/sheets/StatTable.svelte`:

```svelte
<script lang="ts">
  import { getAppContext, type AppContext } from "@shadowcat/ui-kit";
  import { addStat, removeStat, setStatOrder } from "./sheet-model";
  import { nfT } from "./nf-i18n";
  import StatRow from "./StatRow.svelte";
  import { validateStatKey, type Stat, type StatType } from "../nightfox-docs";
  import type { ResolvedStat } from "../resolve";

  // Ordered stat editor. Order is presentation-only (D12): drag/drop rewrites only `order`
  // fields via single-key Updates — the M13b resolver is dependency-ordered, so no value can
  // shift. Add validates the key with the M13b tier-1 rules before any dispatch.
  let { docId, systemPrefix, stats, resolved, readOnly }:
    { docId: string; systemPrefix: string; stats: Record<string, Stat>;
      resolved: Map<string, ResolvedStat>; readOnly: boolean } = $props();

  const ctx: AppContext = getAppContext();
  const t = (k: string) => nfT(ctx, k);

  const ordered = $derived(
    Object.entries(stats).sort((a, b) => (a[1].order - b[1].order) || a[0].localeCompare(b[0])),
  );

  let newKey = $state("");
  let newType = $state<StatType>("number");
  let keyIssues = $state<string[]>([]);

  function defaultStat(type: StatType, order: number): Stat {
    switch (type) {
      case "number": return { type: "number", order, base: 0 };
      case "resource": return { type: "resource", order, current: 0, maxBase: 0 };
      case "text": return { type: "text", order, value: "" };
      case "boolean": return { type: "boolean", order, value: false };
    }
  }

  function add(): void {
    const issues = validateStatKey(newKey);
    if (issues.length > 0 || Object.prototype.hasOwnProperty.call(stats, newKey)) {
      keyIssues = issues.length > 0 ? issues : ["duplicate"];
      return;
    }
    addStat(ctx, docId, systemPrefix, newKey, defaultStat(newType, ordered.length));
    newKey = "";
    keyIssues = [];
  }

  function remove(key: string): void {
    removeStat(ctx, docId, systemPrefix, key, stats);
  }

  // Presentation-only reorder. Dragging key A onto key B recomputes the key sequence and
  // writes each MOVED key's new index as its `order` (raw old = its current order).
  let dragKey = $state<string | null>(null);
  function onDragStart(e: DragEvent): void {
    const key = (e.currentTarget as HTMLElement).dataset.statKey ?? null;
    dragKey = key;
  }
  function onDrop(e: DragEvent): void {
    e.preventDefault();
    const targetKey = (e.currentTarget as HTMLElement).dataset.statKey;
    if (!dragKey || !targetKey || dragKey === targetKey) { dragKey = null; return; }
    const keys = ordered.map(([k]) => k);
    const from = keys.indexOf(dragKey);
    const to = keys.indexOf(targetKey);
    if (from < 0 || to < 0) { dragKey = null; return; }
    keys.splice(to, 0, keys.splice(from, 1)[0]);
    keys.forEach((k, idx) => {
      const cur = stats[k].order;
      if (cur !== idx) setStatOrder(ctx, docId, systemPrefix, k, cur, idx);
    });
    dragKey = null;
  }
</script>

<section class="stat-table">
  <h3>{t("nightfox.stats")}</h3>
  {#each ordered as [key, stat] (key)}
    <div class="row-wrap" role="listitem"
      ondragover={(e) => e.preventDefault()}
      ondragstart={onDragStart}
      ondrop={onDrop}
      data-stat-key={key}>
      <StatRow {docId} {systemPrefix} statKey={key} {stat} resolved={resolved.get(key)} {readOnly} onRemove={() => remove(key)} />
    </div>
  {/each}

  {#if !readOnly}
    <div class="add">
      <label>{t("nightfox.stat.key")}
        <input aria-label={t("nightfox.stat.key")} value={newKey}
          oninput={(e) => { newKey = (e.currentTarget as HTMLInputElement).value; keyIssues = []; }} /></label>
      <label>{t("nightfox.stat.type")}
        <select aria-label={t("nightfox.stat.type")} value={newType}
          onchange={(e) => (newType = (e.currentTarget as HTMLSelectElement).value as StatType)}>
          <option value="number">{t("nightfox.type.number")}</option>
          <option value="resource">{t("nightfox.type.resource")}</option>
          <option value="text">{t("nightfox.type.text")}</option>
          <option value="boolean">{t("nightfox.type.boolean")}</option>
        </select></label>
      <button type="button" onclick={add}>{t("nightfox.stat.add")}</button>
      {#if keyIssues.length > 0}<span class="chip err">{t("nightfox.badKey")}</span>{/if}
    </div>
  {/if}
</section>

<style lang="scss">
  .stat-table { display: flex; flex-direction: column; gap: var(--space-1); }
  .row-wrap { display: block; }
  .add { display: flex; flex-wrap: wrap; align-items: flex-end; gap: var(--space-1); }
  .add label { display: flex; flex-direction: column; gap: 2px; }
  .add button { min-height: 44px; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); }
  .chip.err { color: var(--danger, #c00); border: 1px solid var(--danger, #c00); border-radius: var(--radius-1); padding: 0 var(--space-1); }
</style>
```

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter nightfox test StatTable`
Expected: PASS.
Run: `pnpm --filter nightfox typecheck`
Expected: clean.

- [ ] **Step 5: Commit (inside the Nightfox repo)**

```bash
cd "C:\Dev\Nightfox"
git add src/sheets/StatTable.svelte src/sheets/StatTable.test.ts
git commit -m "feat(sheets): StatTable — ordered list, validated add-stat, presentation-only reorder"
```

---

## Task 6: `ModifiersEditor.svelte` — modifier list + add/remove + inert warnings

**Files:**
- Create: `src/sheets/ModifiersEditor.svelte`
- Test: `src/sheets/ModifiersEditor.test.ts`

**Interfaces:**
- Consumes: `getAppContext` (`@shadowcat/ui-kit`); `addModifier`, `editModifierField`, `removeModifier` (Task 2); `warningChips` (Task 3); `nfT` (Task 1); `type Modifier`, `type ModifierOp` (`../nightfox-docs`, M13b Task 2); `type NfWarning` (`./sheet-model`).
- Produces: a Svelte component with props `{ docId: string; systemPrefix: string; modifiers: Record<string, Modifier>; ownStatKeys: string[]; warnings: NfWarning[]; readOnly: boolean }`. Renders each modifier as target-stat input + op `<select>` + magnitude input (literal-or-formula string), an add-modifier button (a fresh id via `crypto.randomUUID()`), a remove button, and the inert/dangling warning chips from `warnings`.

- [ ] **Step 1: Write the failing test** — create `src/sheets/ModifiersEditor.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ModifiersEditor from "./ModifiersEditor.svelte";
import type { Modifier } from "../nightfox-docs";
import type { NfWarning } from "./sheet-model";

function mount(over: {
  modifiers?: Record<string, Modifier>; ownStatKeys?: string[]; warnings?: NfWarning[];
  readOnly?: boolean; dispatchIntent?: (o: unknown[]) => void;
}) {
  const context = setAppContextForTest({ dispatchIntent: over.dispatchIntent ?? (() => {}), canEdit: () => true });
  return render(ModifiersEditor, {
    props: {
      docId: "I", systemPrefix: "/system", modifiers: over.modifiers ?? {},
      ownStatKeys: over.ownStatKeys ?? [], warnings: over.warnings ?? [], readOnly: over.readOnly ?? false,
    },
    context,
  });
}

describe("ModifiersEditor", () => {
  it("adds a modifier with a fresh id and default add-op shape", async () => {
    vi.stubGlobal("crypto", { randomUUID: () => "mid-1" });
    const calls: unknown[][] = [];
    const { getByText } = mount({ dispatchIntent: (o) => calls.push(o) });
    await fireEvent.click(getByText("nightfox.modifier.add"));
    expect(calls).toEqual([[{ op: "update", doc_id: "I", changes: [
      { path: "/system/mechanics/modifiers/mid-1", old: null, new: { stat: "", op: "add", value: 0 } },
    ] }]]);
    vi.unstubAllGlobals();
  });

  it("edits the op with the raw old", async () => {
    const calls: unknown[][] = [];
    const { getByLabelText } = mount({ modifiers: { m1: { stat: "dex", op: "add", value: 2 } }, dispatchIntent: (o) => calls.push(o) });
    await fireEvent.change(getByLabelText("nightfox.modifier.op"), { target: { value: "mulCompound" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "I", changes: [{ path: "/system/mechanics/modifiers/m1/op", old: "add", new: "mulCompound" }] }]]);
  });

  it("edits the magnitude (literal-or-formula string) with the raw old", async () => {
    const calls: unknown[][] = [];
    const { getByLabelText } = mount({ modifiers: { m1: { stat: "dex", op: "add", value: 2 } }, dispatchIntent: (o) => calls.push(o) });
    await fireEvent.change(getByLabelText("nightfox.modifier.value"), { target: { value: "parent.str / 2" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "I", changes: [{ path: "/system/mechanics/modifiers/m1/value", old: 2, new: "parent.str / 2" }] }]]);
  });

  it("removes a modifier via whole-map replace", async () => {
    const calls: unknown[][] = [];
    const current = { m1: { stat: "dex", op: "add" as const, value: 2 } };
    const { getByLabelText } = mount({ modifiers: current, dispatchIntent: (o) => calls.push(o) });
    await fireEvent.click(getByLabelText("nightfox.modifier.remove"));
    expect(calls).toEqual([[{ op: "update", doc_id: "I", changes: [{ path: "/system/mechanics/modifiers", old: current, new: {} }] }]]);
  });

  it("surfaces inert/dangling warning chips", () => {
    const { getByText } = mount({ warnings: [{ docId: "I", kind: "dangling-modifiers", detail: "m1" }] });
    expect(getByText("nightfox.warn.dangling")).toBeTruthy();
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter nightfox test ModifiersEditor`
Expected: FAIL — `Cannot find module './ModifiersEditor.svelte'`.

- [ ] **Step 3: Implement** — create `src/sheets/ModifiersEditor.svelte`:

```svelte
<script lang="ts">
  import { getAppContext, type AppContext } from "@shadowcat/ui-kit";
  import { addModifier, editModifierField, removeModifier } from "./sheet-model";
  import { warningChips } from "./format";
  import { nfT } from "./nf-i18n";
  import type { Modifier, ModifierOp } from "../nightfox-docs";
  import type { NfWarning } from "./sheet-model";

  // Modifier editor for item/effect sheets. A magnitude is a literal-or-formula STRING
  // (D4); it is stored verbatim and evaluated at read time through the same cycle-guarded
  // graph. Inert/dangling outcomes (§5.4) are passed in as resolver/collection warnings and
  // rendered as advisory chips — a mismatched modifier never errors the sheet.
  let { docId, systemPrefix, modifiers, ownStatKeys, warnings, readOnly }:
    { docId: string; systemPrefix: string; modifiers: Record<string, Modifier>;
      ownStatKeys: string[]; warnings: NfWarning[]; readOnly: boolean } = $props();

  const ctx: AppContext = getAppContext();
  const t = (k: string) => nfT(ctx, k);
  const ops: ModifierOp[] = ["add", "mulAdditive", "mulCompound"];

  const entries = $derived(Object.entries(modifiers));
  const chips = $derived(warningChips(warnings));

  function add(): void {
    addModifier(ctx, docId, systemPrefix, crypto.randomUUID(), { stat: "", op: "add", value: 0 });
  }
  function editField(id: string, field: keyof Modifier, old: unknown, value: unknown): void {
    editModifierField(ctx, docId, systemPrefix, id, field, old, value);
  }
  function remove(id: string): void {
    removeModifier(ctx, docId, systemPrefix, id, modifiers);
  }
</script>

<section class="modifiers">
  <h3>{t("nightfox.modifiers")}</h3>
  {#each entries as [id, m] (id)}
    <div class="mod-row">
      <label>{t("nightfox.modifier.stat")}
        <input aria-label={t("nightfox.modifier.stat")} value={m.stat} list="nf-own-stats" disabled={readOnly}
          onchange={(e) => editField(id, "stat", m.stat, (e.currentTarget as HTMLInputElement).value)} /></label>
      <label>{t("nightfox.modifier.op")}
        <select aria-label={t("nightfox.modifier.op")} value={m.op} disabled={readOnly}
          onchange={(e) => editField(id, "op", m.op, (e.currentTarget as HTMLSelectElement).value)}>
          {#each ops as op}<option value={op}>{t(`nightfox.op.${op}`)}</option>{/each}
        </select></label>
      <label>{t("nightfox.modifier.value")}
        <input aria-label={t("nightfox.modifier.value")} value={String(m.value)} disabled={readOnly}
          onchange={(e) => editField(id, "value", m.value, (e.currentTarget as HTMLInputElement).value)} /></label>
      <button type="button" class="remove" aria-label={t("nightfox.modifier.remove")} disabled={readOnly} onclick={() => remove(id)}>×</button>
    </div>
  {/each}

  <datalist id="nf-own-stats">
    {#each ownStatKeys as k}<option value={k}></option>{/each}
  </datalist>

  {#each chips as chip}<span class="chip err" title={chip.detail}>{t(chip.messageKey)}</span>{/each}

  {#if !readOnly}
    <button type="button" class="add" onclick={add}>{t("nightfox.modifier.add")}</button>
  {/if}
</section>

<style lang="scss">
  .modifiers { display: flex; flex-direction: column; gap: var(--space-1); }
  .mod-row { display: flex; flex-wrap: wrap; align-items: flex-end; gap: var(--space-1); }
  label { display: flex; flex-direction: column; gap: 2px; }
  button { min-height: 44px; min-width: 44px; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); }
  button:focus-visible { outline: 2px solid var(--accent); }
  .chip.err { color: var(--danger, #c00); border: 1px solid var(--danger, #c00); border-radius: var(--radius-1); padding: 0 var(--space-1); }
</style>
```

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter nightfox test ModifiersEditor`
Expected: PASS.
Run: `pnpm --filter nightfox typecheck`
Expected: clean.

- [ ] **Step 5: Commit (inside the Nightfox repo)**

```bash
cd "C:\Dev\Nightfox"
git add src/sheets/ModifiersEditor.svelte src/sheets/ModifiersEditor.test.ts
git commit -m "feat(sheets): ModifiersEditor — target/op/magnitude editing + inert warnings"
```

---

## Task 7: `ActorSheet.svelte` — header + stat table + inventory/effects lists

**FLAGGED FOR BUDDY-CHECK** (permission-sensitive rendering — see Buddy-check directives).

**Files:**
- Create: `src/sheets/ActorSheet.svelte`
- Test: `src/sheets/ActorSheet.test.ts`

**Interfaces:**
- Consumes: `getAppContext`, `setField` (`@shadowcat/ui-kit`); `createSubscriber` (`svelte/reactivity`); `getPointer`, `actorDisplayName`, `type WireDocument`, `type ActorEngine` (`@shadowcat/core`); `sheetView`, `setMechanicsFlag` (Task 2); `nfT` (Task 1); `StatTable` (Task 5); `parseNightfox` (`../nightfox-docs`).
- Produces: a Svelte component with props `{ docId: string; systemPrefix: string; close: () => void }` (the sheetsController contract). Reads the OPTIMISTIC store through a `subscribe()` bridge; feeds the top-level doc to `sheetView`; renders the name/displayName header, a `StatTable` over the actor's own resolved stats, an inventory list (each embedded item: open + active toggle), and an effects list (each embedded effect: open + active + transfer toggles).

- [ ] **Step 1: Write the failing test** — create `src/sheets/ActorSheet.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { DocumentStore, envelope } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ActorSheet from "./ActorSheet.svelte";

const sys = (stats: Record<string, unknown> = {}, mech: Record<string, unknown> = {}) =>
  ({ stats, mechanics: { version: 1, ...mech } });

function storeWithActor() {
  const s = new DocumentStore();
  const actor = envelope("w1", "actor", null,
    sys({ str: { type: "number", order: 0, base: 2 }, attack: { type: "number", order: 1, base: 0, formula: "str + 1" } }),
    "a1", { displayName: "Aria", shape: "square", size: { w: 1, h: 1 } }, "Aria");
  actor.embedded = {
    item: [envelope("w1", "item", null, sys({}, { active: true, modifiers: { m: { stat: "str", op: "add", value: 2 } } }), "i0", undefined, "Belt")],
    effect: [envelope("w1", "effect", null, sys({}, { active: true, transfer: true, modifiers: { m: { stat: "attack", op: "add", value: 1 } } }), "e0", undefined, "Bless")],
  };
  s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [{ op: "create", doc: actor }] });
  return s;
}

describe("ActorSheet", () => {
  it("renders the resolved attack including the belt (str 2 + 2) + 1 = 5", () => {
    const documents = storeWithActor();
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getByText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    expect(getByText("nightfox.stat.computed: 5")).toBeTruthy();
  });

  it("lists inventory + effects and opens an item via openDocument", async () => {
    const opened: unknown[] = [];
    const documents = storeWithActor();
    const context = setAppContextForTest({ documents, canEdit: () => true, openDocument: (r) => opened.push(r) });
    const { getByText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByText("Belt"));
    expect(opened).toEqual([{ docId: "a1", embeddedPath: "/embedded/item/0" }]);
    expect(getByText("Bless")).toBeTruthy();
  });

  it("toggles an embedded effect's transfer with the raw old", async () => {
    const calls: unknown[][] = [];
    const documents = storeWithActor();
    const context = setAppContextForTest({ documents, canEdit: () => true, dispatchIntent: (o) => calls.push(o) });
    const { getByLabelText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByLabelText("Bless nightfox.transfer"));
    expect(calls).toEqual([[{ op: "update", doc_id: "a1", changes: [
      { path: "/embedded/effect/0/system/mechanics/transfer", old: true, new: false },
    ] }]]);
  });

  it("shows the missing state for a non-Nightfox actor", () => {
    const s = new DocumentStore();
    s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [{ op: "create", doc: envelope("w1", "actor", null, { grid: {} }, "a2", { displayName: "X", shape: "square", size: { w: 1, h: 1 } }, "X") }] });
    const context = setAppContextForTest({ documents: s, canEdit: () => true });
    const { getByText } = render(ActorSheet, { props: { docId: "a2", systemPrefix: "/system", close: () => {} }, context });
    expect(getByText("nightfox.missing")).toBeTruthy();
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter nightfox test ActorSheet`
Expected: FAIL — `Cannot find module './ActorSheet.svelte'`.

- [ ] **Step 3: Implement** — create `src/sheets/ActorSheet.svelte`:

```svelte
<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, setField, type AppContext } from "@shadowcat/ui-kit";
  import { getPointer, actorDisplayName, type WireDocument, type ActorEngine } from "@shadowcat/core";
  import { sheetView } from "./sheet-model";
  import { parseNightfox } from "../nightfox-docs";
  import { nfT } from "./nf-i18n";
  import StatTable from "./StatTable.svelte";

  // Actor sheet: header + Nightfox stat table + inventory/effects lists. Reads the OPTIMISTIC
  // store (per-recipient redaction is already applied) through a subscribe() bridge so a 2nd
  // edit reads the 1st's result (OCC). `resolveNightfox` is fed ONLY the optimistic top-level
  // doc — nothing hidden reaches the evaluator (§10). systemPrefix is always "/system" for an
  // actor top-level open.
  let { docId, systemPrefix, close }: { docId: string; systemPrefix: string; close: () => void } = $props();

  const ctx: AppContext = getAppContext();
  const t = (k: string) => nfT(ctx, k);
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const doc = $derived.by((): WireDocument | undefined => { subscribe(); return ctx.documents.get(docId); });
  const engine = $derived.by((): ActorEngine | undefined => (doc ? (getPointer(doc, "/engine") as ActorEngine | undefined) : undefined));
  const name = $derived.by((): string | null => (doc ? ((getPointer(doc, "/name") as string | null | undefined) ?? null) : null));
  const view = $derived.by(() => { subscribe(); return sheetView(doc, systemPrefix); });
  const stats = $derived(view.block?.stats ?? {});
  const readOnly = $derived(!doc || !ctx.canEdit(doc, systemPrefix));

  interface Carrier { name: string; index: number; active: boolean; activeRaw: boolean | undefined; transfer?: boolean; transferRaw?: boolean | undefined }
  function carriers(collection: "item" | "effect"): Carrier[] {
    subscribe();
    if (!doc || systemPrefix !== "/system") return [];
    return (doc.embedded?.[collection] ?? []).map((c, index) => {
      const mech = parseNightfox(c)?.mechanics;
      const rawMech = (c.system as { mechanics?: { active?: boolean; transfer?: boolean } } | undefined)?.mechanics;
      return {
        name: c.name ?? t("nightfox.missing"),
        index,
        active: mech?.active !== false,
        activeRaw: rawMech?.active,
        transfer: mech?.transfer === true,
        transferRaw: rawMech?.transfer,
      };
    });
  }
  const inventory = $derived.by(() => carriers("item"));
  const effects = $derived.by(() => carriers("effect"));

  function toggleFlag(collection: "item" | "effect", index: number, flag: "active" | "transfer", old: boolean | undefined, value: boolean): void {
    setField(ctx, docId, `/embedded/${collection}/${index}/system/mechanics/${flag}`, old ?? null, value);
  }
</script>

<div class="sheet" role="dialog" aria-label={t("nightfox.stats")}>
  <header class="sheet-header">
    <h2>{engine ? actorDisplayName({ name, displayName: engine.displayName }) : (name ?? t("nightfox.stats"))}</h2>
    <button type="button" class="close" aria-label="close" onclick={close}>×</button>
  </header>

  {#if doc && view.block}
    <StatTable {docId} {systemPrefix} {stats} resolved={view.stats} {readOnly} />

    {#if inventory.length > 0}
      <h3>{t("nightfox.inventory")}</h3>
      <ul class="carriers">
        {#each inventory as it (it.index)}
          <li>
            <button type="button" class="open" onclick={() => ctx.openDocument({ docId, embeddedPath: `/embedded/item/${it.index}` })}>{it.name}</button>
            <label>{t("nightfox.active")}
              <input type="checkbox" aria-label={`${it.name} ${t("nightfox.active")}`} checked={it.active} disabled={readOnly}
                onchange={(e) => toggleFlag("item", it.index, "active", it.activeRaw, (e.currentTarget as HTMLInputElement).checked)} /></label>
          </li>
        {/each}
      </ul>
    {/if}

    {#if effects.length > 0}
      <h3>{t("nightfox.effects")}</h3>
      <ul class="carriers">
        {#each effects as ef (ef.index)}
          <li>
            <button type="button" class="open" onclick={() => ctx.openDocument({ docId, embeddedPath: `/embedded/effect/${ef.index}` })}>{ef.name}</button>
            <label>{t("nightfox.active")}
              <input type="checkbox" aria-label={`${ef.name} ${t("nightfox.active")}`} checked={ef.active} disabled={readOnly}
                onchange={(e) => toggleFlag("effect", ef.index, "active", ef.activeRaw, (e.currentTarget as HTMLInputElement).checked)} /></label>
            <label>{t("nightfox.transfer")}
              <input type="checkbox" aria-label={`${ef.name} ${t("nightfox.transfer")}`} checked={ef.transfer} disabled={readOnly}
                onchange={(e) => toggleFlag("effect", ef.index, "transfer", ef.transferRaw, (e.currentTarget as HTMLInputElement).checked)} /></label>
          </li>
        {/each}
      </ul>
    {/if}
  {:else}
    <p class="missing">{t("nightfox.missing")}</p>
  {/if}
</div>

<style lang="scss">
  .sheet { display: flex; flex-direction: column; gap: var(--space-1); padding: var(--space-1); height: 100%; overflow: auto; }
  .sheet-header { display: flex; align-items: center; justify-content: space-between; }
  .close { min-width: 44px; min-height: 44px; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); }
  .carriers { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-1); }
  .carriers li { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-1); }
  .carriers .open { min-height: 44px; text-align: left; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); flex: 1; }
  label { display: flex; align-items: center; gap: 2px; }
  input[type="checkbox"] { min-width: 44px; min-height: 44px; }
  .missing { opacity: 0.7; font-style: italic; }
</style>
```

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter nightfox test ActorSheet`
Expected: PASS.
Run: `pnpm --filter nightfox typecheck`
Expected: clean.

- [ ] **Step 5: Commit (inside the Nightfox repo)**

```bash
cd "C:\Dev\Nightfox"
git add src/sheets/ActorSheet.svelte src/sheets/ActorSheet.test.ts
git commit -m "feat(sheets): ActorSheet — resolved stat table + inventory/effects toggles"
```

---

## Task 8: `ItemSheet.svelte` — name + own stat block + modifiers editor

**Files:**
- Create: `src/sheets/ItemSheet.svelte`
- Test: `src/sheets/ItemSheet.test.ts`

**Interfaces:**
- Consumes: `getAppContext`, `setField` (`@shadowcat/ui-kit`); `createSubscriber` (`svelte/reactivity`); `getPointer`, `type WireDocument` (`@shadowcat/core`); `sheetView`, `setMechanicsFlag` (Task 2); `nfT` (Task 1); `StatTable` (Task 5); `ModifiersEditor` (Task 6); `parseNightfox` (`../nightfox-docs`).
- Produces: a Svelte component with props `{ docId: string; systemPrefix: string; close: () => void }`. Renders the item name (written at `basePrefix/name`, embedded-aware), a `StatTable` over the item's own stats, a `ModifiersEditor` over `mechanics.modifiers`, and an active toggle.

- [ ] **Step 1: Write the failing test** — create `src/sheets/ItemSheet.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { DocumentStore, envelope } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ItemSheet from "./ItemSheet.svelte";

const sys = (stats: Record<string, unknown> = {}, mech: Record<string, unknown> = {}) =>
  ({ stats, mechanics: { version: 1, ...mech } });

function storeWithTopItem() {
  const s = new DocumentStore();
  const item = envelope("w1", "item", null,
    sys({ damage: { type: "number", order: 0, base: 3 } }, { active: true, modifiers: { m1: { stat: "str", op: "add", value: 2 } } }),
    "i1", undefined, "Sword");
  s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [{ op: "create", doc: item }] });
  return s;
}

describe("ItemSheet", () => {
  it("shows the item name and its own resolved damage stat", () => {
    const documents = storeWithTopItem();
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getByLabelText, getByText } = render(ItemSheet, { props: { docId: "i1", systemPrefix: "/system", close: () => {} }, context });
    expect((getByLabelText("nightfox.name") as HTMLInputElement).value).toBe("Sword");
    expect(getByText("nightfox.stat.computed: 3")).toBeTruthy();
  });

  it("renders the modifier editor over mechanics.modifiers", () => {
    const documents = storeWithTopItem();
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getByDisplayValue } = render(ItemSheet, { props: { docId: "i1", systemPrefix: "/system", close: () => {} }, context });
    expect((getByDisplayValue("str")).getAttribute("aria-label")).toBe("nightfox.modifier.stat");
  });

  it("toggles active with the raw old", async () => {
    const calls: unknown[][] = [];
    const documents = storeWithTopItem();
    const context = setAppContextForTest({ documents, canEdit: () => true, dispatchIntent: (o) => calls.push(o) });
    const { getByLabelText } = render(ItemSheet, { props: { docId: "i1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByLabelText("nightfox.active"));
    expect(calls).toEqual([[{ op: "update", doc_id: "i1", changes: [{ path: "/system/mechanics/active", old: true, new: false }] }]]);
  });

  it("edits the item name with the real pre-image", async () => {
    const calls: unknown[][] = [];
    const documents = storeWithTopItem();
    const context = setAppContextForTest({ documents, canEdit: () => true, dispatchIntent: (o) => calls.push(o) });
    const { getByLabelText } = render(ItemSheet, { props: { docId: "i1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByLabelText("nightfox.name"), { target: { value: "Axe" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "i1", changes: [{ path: "/name", old: "Sword", new: "Axe" }] }]]);
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter nightfox test ItemSheet`
Expected: FAIL — `Cannot find module './ItemSheet.svelte'`.

- [ ] **Step 3: Implement** — create `src/sheets/ItemSheet.svelte`:

```svelte
<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, setField, type AppContext } from "@shadowcat/ui-kit";
  import { getPointer, type WireDocument } from "@shadowcat/core";
  import { sheetView, setMechanicsFlag } from "./sheet-model";
  import { parseNightfox } from "../nightfox-docs";
  import { nfT } from "./nf-i18n";
  import StatTable from "./StatTable.svelte";
  import ModifiersEditor from "./ModifiersEditor.svelte";

  // Item sheet: name + own stat block + modifiers editor + active toggle. `systemPrefix`
  // ends in "/system"; for an embedded (inventory) item it is "/embedded/item/<i>/system"
  // and `docId` is the PARENT actor. `basePrefix` (systemPrefix minus "/system") is the
  // sibling root of `/name` and `/mechanics` — writing the literal "/name" for an embedded
  // item would rename the parent actor.
  let { docId, systemPrefix, close }: { docId: string; systemPrefix: string; close: () => void } = $props();

  const ctx: AppContext = getAppContext();
  const t = (k: string) => nfT(ctx, k);
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const basePrefix = $derived(systemPrefix.replace(/\/system$/, ""));
  const namePrefix = $derived(`${basePrefix}/name`);

  const doc = $derived.by((): WireDocument | undefined => { subscribe(); return ctx.documents.get(docId); });
  const name = $derived.by((): string | null => (doc ? ((getPointer(doc, namePrefix) as string | null | undefined) ?? null) : null));
  const view = $derived.by(() => { subscribe(); return sheetView(doc, systemPrefix); });
  const stats = $derived(view.block?.stats ?? {});
  const modifiers = $derived(view.block?.mechanics.modifiers ?? {});
  const active = $derived(view.block?.mechanics.active !== false);
  const activeRaw = $derived.by((): boolean | undefined => (doc ? (getPointer(doc, `${basePrefix}/mechanics/active`) as boolean | undefined) : undefined));
  const readOnly = $derived(!doc || !ctx.canEdit(doc, systemPrefix));

  function setName(value: string): void { if (doc) setField(ctx, docId, namePrefix, name, value); }
</script>

<div class="sheet" role="dialog" aria-label={t("nightfox.stats")}>
  <header class="sheet-header">
    <h2>{name ?? t("nightfox.stats")}</h2>
    <button type="button" class="close" aria-label="close" onclick={close}>×</button>
  </header>

  {#if doc && view.block}
    <label>{t("nightfox.name")}
      <input aria-label={t("nightfox.name")} value={name ?? ""} disabled={readOnly}
        onchange={(e) => setName((e.currentTarget as HTMLInputElement).value)} /></label>
    <label class="flag">{t("nightfox.active")}
      <input type="checkbox" aria-label={t("nightfox.active")} checked={active} disabled={readOnly}
        onchange={(e) => setMechanicsFlag(ctx, docId, systemPrefix, "active", activeRaw, (e.currentTarget as HTMLInputElement).checked)} /></label>

    <StatTable {docId} {systemPrefix} {stats} resolved={view.stats} {readOnly} />
    <ModifiersEditor {docId} {systemPrefix} {modifiers} ownStatKeys={Object.keys(stats)} warnings={view.warnings} {readOnly} />
  {:else}
    <p class="missing">{t("nightfox.missing")}</p>
  {/if}
</div>

<style lang="scss">
  .sheet { display: flex; flex-direction: column; gap: var(--space-1); padding: var(--space-1); height: 100%; overflow: auto; }
  .sheet-header { display: flex; align-items: center; justify-content: space-between; }
  .close { min-width: 44px; min-height: 44px; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); }
  label { display: flex; flex-direction: column; gap: 2px; }
  label.flag { flex-direction: row; align-items: center; }
  input[type="checkbox"] { min-width: 44px; min-height: 44px; }
  .missing { opacity: 0.7; font-style: italic; }
</style>
```

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter nightfox test ItemSheet`
Expected: PASS.
Run: `pnpm --filter nightfox typecheck`
Expected: clean.

- [ ] **Step 5: Commit (inside the Nightfox repo)**

```bash
cd "C:\Dev\Nightfox"
git add src/sheets/ItemSheet.svelte src/sheets/ItemSheet.test.ts
git commit -m "feat(sheets): ItemSheet — name + own stat block + modifiers + active toggle"
```

---

## Task 9: `EffectSheet.svelte` — name + own stat block + modifiers + active/transfer

**Files:**
- Create: `src/sheets/EffectSheet.svelte`
- Test: `src/sheets/EffectSheet.test.ts`

**Interfaces:**
- Consumes: identical set to Task 8 plus `setMechanicsFlag` for BOTH `active` and `transfer`.
- Produces: a Svelte component with props `{ docId: string; systemPrefix: string; close: () => void }`. Same shape as `ItemSheet` but adds a `transfer` toggle (effects only, D14/§4).

- [ ] **Step 1: Write the failing test** — create `src/sheets/EffectSheet.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { DocumentStore, envelope } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import EffectSheet from "./EffectSheet.svelte";

const sys = (stats: Record<string, unknown> = {}, mech: Record<string, unknown> = {}) =>
  ({ stats, mechanics: { version: 1, ...mech } });

function storeWithTopEffect() {
  const s = new DocumentStore();
  const effect = envelope("w1", "effect", null,
    sys({ potency: { type: "number", order: 0, base: 4 } }, { active: true, transfer: false, modifiers: { m1: { stat: "str", op: "add", value: 1 } } }),
    "e1", undefined, "Bless");
  s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [{ op: "create", doc: effect }] });
  return s;
}

describe("EffectSheet", () => {
  it("shows the name, own resolved stat, and both toggles", () => {
    const documents = storeWithTopEffect();
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getByLabelText, getByText } = render(EffectSheet, { props: { docId: "e1", systemPrefix: "/system", close: () => {} }, context });
    expect((getByLabelText("nightfox.name") as HTMLInputElement).value).toBe("Bless");
    expect(getByText("nightfox.stat.computed: 4")).toBeTruthy();
    expect((getByLabelText("nightfox.active") as HTMLInputElement).checked).toBe(true);
    expect((getByLabelText("nightfox.transfer") as HTMLInputElement).checked).toBe(false);
  });

  it("toggles transfer with the raw old (false stored)", async () => {
    const calls: unknown[][] = [];
    const documents = storeWithTopEffect();
    const context = setAppContextForTest({ documents, canEdit: () => true, dispatchIntent: (o) => calls.push(o) });
    const { getByLabelText } = render(EffectSheet, { props: { docId: "e1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByLabelText("nightfox.transfer"));
    expect(calls).toEqual([[{ op: "update", doc_id: "e1", changes: [{ path: "/system/mechanics/transfer", old: false, new: true }] }]]);
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter nightfox test EffectSheet`
Expected: FAIL — `Cannot find module './EffectSheet.svelte'`.

- [ ] **Step 3: Implement** — create `src/sheets/EffectSheet.svelte`:

```svelte
<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, setField, type AppContext } from "@shadowcat/ui-kit";
  import { getPointer, type WireDocument } from "@shadowcat/core";
  import { sheetView, setMechanicsFlag } from "./sheet-model";
  import { nfT } from "./nf-i18n";
  import StatTable from "./StatTable.svelte";
  import ModifiersEditor from "./ModifiersEditor.svelte";

  // Effect sheet: like the item sheet plus a `transfer` toggle (effects only, D14/§4). An
  // item-embedded effect reaches the owning actor only when transfer is set; the resolver
  // enforces the gating — this sheet only authors the flags.
  let { docId, systemPrefix, close }: { docId: string; systemPrefix: string; close: () => void } = $props();

  const ctx: AppContext = getAppContext();
  const t = (k: string) => nfT(ctx, k);
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const basePrefix = $derived(systemPrefix.replace(/\/system$/, ""));
  const namePrefix = $derived(`${basePrefix}/name`);

  const doc = $derived.by((): WireDocument | undefined => { subscribe(); return ctx.documents.get(docId); });
  const name = $derived.by((): string | null => (doc ? ((getPointer(doc, namePrefix) as string | null | undefined) ?? null) : null));
  const view = $derived.by(() => { subscribe(); return sheetView(doc, systemPrefix); });
  const stats = $derived(view.block?.stats ?? {});
  const modifiers = $derived(view.block?.mechanics.modifiers ?? {});
  const active = $derived(view.block?.mechanics.active !== false);
  const transfer = $derived(view.block?.mechanics.transfer === true);
  const activeRaw = $derived.by((): boolean | undefined => (doc ? (getPointer(doc, `${basePrefix}/mechanics/active`) as boolean | undefined) : undefined));
  const transferRaw = $derived.by((): boolean | undefined => (doc ? (getPointer(doc, `${basePrefix}/mechanics/transfer`) as boolean | undefined) : undefined));
  const readOnly = $derived(!doc || !ctx.canEdit(doc, systemPrefix));

  function setName(value: string): void { if (doc) setField(ctx, docId, namePrefix, name, value); }
</script>

<div class="sheet" role="dialog" aria-label={t("nightfox.stats")}>
  <header class="sheet-header">
    <h2>{name ?? t("nightfox.stats")}</h2>
    <button type="button" class="close" aria-label="close" onclick={close}>×</button>
  </header>

  {#if doc && view.block}
    <label>{t("nightfox.name")}
      <input aria-label={t("nightfox.name")} value={name ?? ""} disabled={readOnly}
        onchange={(e) => setName((e.currentTarget as HTMLInputElement).value)} /></label>
    <label class="flag">{t("nightfox.active")}
      <input type="checkbox" aria-label={t("nightfox.active")} checked={active} disabled={readOnly}
        onchange={(e) => setMechanicsFlag(ctx, docId, systemPrefix, "active", activeRaw, (e.currentTarget as HTMLInputElement).checked)} /></label>
    <label class="flag">{t("nightfox.transfer")}
      <input type="checkbox" aria-label={t("nightfox.transfer")} checked={transfer} disabled={readOnly}
        onchange={(e) => setMechanicsFlag(ctx, docId, systemPrefix, "transfer", transferRaw, (e.currentTarget as HTMLInputElement).checked)} /></label>

    <StatTable {docId} {systemPrefix} {stats} resolved={view.stats} {readOnly} />
    <ModifiersEditor {docId} {systemPrefix} {modifiers} ownStatKeys={Object.keys(stats)} warnings={view.warnings} {readOnly} />
  {:else}
    <p class="missing">{t("nightfox.missing")}</p>
  {/if}
</div>

<style lang="scss">
  .sheet { display: flex; flex-direction: column; gap: var(--space-1); padding: var(--space-1); height: 100%; overflow: auto; }
  .sheet-header { display: flex; align-items: center; justify-content: space-between; }
  .close { min-width: 44px; min-height: 44px; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); }
  label { display: flex; flex-direction: column; gap: 2px; }
  label.flag { flex-direction: row; align-items: center; }
  input[type="checkbox"] { min-width: 44px; min-height: 44px; }
  .missing { opacity: 0.7; font-style: italic; }
</style>
```

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter nightfox test EffectSheet`
Expected: PASS.
Run: `pnpm --filter nightfox typecheck`
Expected: clean.

- [ ] **Step 5: Commit (inside the Nightfox repo)**

```bash
cd "C:\Dev\Nightfox"
git add src/sheets/EffectSheet.svelte src/sheets/EffectSheet.test.ts
git commit -m "feat(sheets): EffectSheet — own stat block + modifiers + active/transfer toggles"
```

---

## Task 10: `src/index.ts` — register the three sheets above the generics

**Files:**
- Modify: `src/index.ts` (final form — supersedes the M13b Task 6 headless barrel and the M13-1 Task 18 placeholder `Hello` contribution)
- Modify: `src/index.test.ts` (supersedes the M13b/Task-18 registration assertions)

**Interfaces:**
- Consumes: `sheetContract`, `ITEM_DOC_TYPE`, `type Module` (`@shadowcat/core`); the three sheet components (Tasks 7–9); the M13b rules barrel modules (`./nightfox-docs`, `./contributions`, `./resolve`); `NOTATION_KEYWORDS`, `MAX_FORMULA_LENGTH` (`@shadowcat/formula`, re-exported per M13b Task 6).
- Produces: `export const EFFECT_DOC_TYPE = "effect"`; the updated `export const nightfox: Module` / `export default nightfox` whose `register(ctx)` contributes `ActorSheet`/`ItemSheet`/`EffectSheet` at `sheet.priority = 10` (above the generic sheets' `0`, below any community bid); manifest `provides` the three sheet contracts. All prior rules-barrel exports are preserved.

- [ ] **Step 1: Write the failing test** — replace the entire contents of `src/index.test.ts` with:

```ts
import { describe, it, expect } from "vitest";
import { ContributionRegistry, sheetContract, ITEM_DOC_TYPE } from "@shadowcat/core";
import { nightfox, EFFECT_DOC_TYPE, parseNightfox, resolveNightfox, NOTATION_KEYWORDS } from "./index";

describe("nightfox module registration (M13c)", () => {
  it("still exposes the M13b rules barrel", () => {
    expect(typeof parseNightfox).toBe("function");
    expect(typeof resolveNightfox).toBe("function");
    expect(Array.isArray(NOTATION_KEYWORDS)).toBe(true);
  });

  it("declares its identity and engine-compat range", () => {
    expect(nightfox.manifest.id).toBe("nightfox");
    expect(nightfox.manifest.engines?.shadowcat).toBe("^0.1.0");
  });

  it("provides all three sheet contracts", () => {
    const contracts = nightfox.manifest.provides.map((p) => p.contract);
    expect(contracts).toEqual(expect.arrayContaining([
      sheetContract("actor"), sheetContract(ITEM_DOC_TYPE), sheetContract(EFFECT_DOC_TYPE),
    ]));
  });

  it("registers actor/item/effect sheets above the generic sheets (priority 10)", () => {
    const contributions = new ContributionRegistry();
    nightfox.register({
      contributions: { contribute: (c: Parameters<typeof contributions.contribute>[0]) => contributions.contribute(c, { module: "nightfox" }) },
      logger: { debug() {}, warn() {}, error() {} },
    } as never);
    for (const dt of ["actor", ITEM_DOC_TYPE, EFFECT_DOC_TYPE]) {
      const entry = contributions.entriesFor(sheetContract(dt))[0];
      expect(entry?.contribution.sheet?.priority).toBe(10);
      expect(entry?.module).toBe("nightfox");
    }
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter nightfox test index`
Expected: FAIL — `EFFECT_DOC_TYPE` not exported / sheets not registered / priority mismatch.

- [ ] **Step 3: Implement** — replace the entire contents of `src/index.ts` with:

```ts
import { sheetContract, ITEM_DOC_TYPE, type Module } from "@shadowcat/core";
import ActorSheet from "./sheets/ActorSheet.svelte";
import ItemSheet from "./sheets/ItemSheet.svelte";
import EffectSheet from "./sheets/EffectSheet.svelte";

// M13b rules barrel — preserved so M13d and external consumers keep their pure API.
export * from "./nightfox-docs";
export * from "./contributions";
export * from "./resolve";
export { NOTATION_KEYWORDS, MAX_FORMULA_LENGTH } from "@shadowcat/formula";

/** Nightfox's client-semantics `effect` doc_type (D9) — embeddable in actors and items,
 * exactly as M12c introduced `item`; zero server change. Defined here (the sheets barrel)
 * because M13b never declared a constant for it; see the plan's Spec gaps note. */
export const EFFECT_DOC_TYPE = "effect";

/** Nightfox module: the headless rules engine (M13b) plus the actor/item/effect sheets
 * (M13c), registered above the generic sheets (priority 10 > generic 0). A community sheet
 * module outbids with a higher priority (D10). */
export const nightfox: Module = {
  manifest: {
    id: "nightfox",
    version: "0.1.0",
    dependencies: {},
    engines: { shadowcat: "^0.1.0" },
    capabilities: [],
    requirements: [],
    provides: [
      { contract: sheetContract("actor"), cardinality: "multi" },
      { contract: sheetContract(ITEM_DOC_TYPE), cardinality: "multi" },
      { contract: sheetContract(EFFECT_DOC_TYPE), cardinality: "multi" },
    ],
    requires: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "nightfox:sheet-actor", contract: sheetContract("actor"), component: ActorSheet, sheet: { priority: 10 } });
    ctx.contributions.contribute({ id: "nightfox:sheet-item", contract: sheetContract(ITEM_DOC_TYPE), component: ItemSheet, sheet: { priority: 10 } });
    ctx.contributions.contribute({ id: "nightfox:sheet-effect", contract: sheetContract(EFFECT_DOC_TYPE), component: EffectSheet, sheet: { priority: 10 } });
    ctx.logger.debug("nightfox sheets registered");
  },
};

export default nightfox;
```

Note: this deletes the Task-18 `Hello.svelte` placeholder contribution and the `nightfox.example:hello` provides entry. Also delete the now-unused placeholder files so `pnpm --filter nightfox typecheck`/`build` do not carry dead code:

Run: `rm -f "C:\Dev\Nightfox\src\Hello.svelte"`
(The Task-18 `src/index.test.ts` is fully replaced in Step 1; if a separate `src/Hello.test.ts` or `src/index.test.ts` reference to `Hello` exists from Task 18, it is superseded by Step 1's rewrite — confirm no other file imports `./Hello`.)

Run: `pnpm --filter nightfox test 2>&1 | head -40` and confirm no test references `./Hello`. If one does (a leftover Task-18 test file), delete it: `rm -f "C:\Dev\Nightfox\src\Hello.test.ts"`.

- [ ] **Step 4: Run the full package gate**

Run: `pnpm --filter nightfox test`
Expected: PASS (all M13b + M13c suites).
Run: `pnpm --filter nightfox typecheck`
Expected: clean.
Run: `pnpm --filter nightfox build`
Expected: builds `dist/index.js` with `svelte`/`@shadowcat/*` external (no duplicate runtime).

- [ ] **Step 5: Commit (inside the Nightfox repo)**

```bash
cd "C:\Dev\Nightfox"
git add src/index.ts src/index.test.ts
git rm -f --ignore-unmatch src/Hello.svelte src/Hello.test.ts
git commit -m "feat(sheets): register actor/item/effect sheets (priority 10) + EFFECT_DOC_TYPE"
```

---

## Task 11: Full-flow integration test (spec §11)

**Files:**
- Create: `src/sheets/flow.integration.test.ts`

**Interfaces:**
- Consumes: `ActorSheet` (Task 7); `DocumentStore`, `envelope` (`@shadowcat/core`); `setAppContextForTest` (`@shadowcat/ui-kit/test`). Drives real field-path Updates back into a live `DocumentStore` so reactive re-resolution is exercised end to end (the component-level stand-in for the spec's Playwright e2e — see Spec gaps for why a browser harness is out-of-toolchain).
- Produces: no exports (test only).

- [ ] **Step 1: Write the test** — create `src/sheets/flow.integration.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { DocumentStore, envelope } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ActorSheet from "./ActorSheet.svelte";

const sys = (stats: Record<string, unknown> = {}, mech: Record<string, unknown> = {}) =>
  ({ stats, mechanics: { version: 1, ...mech } });

/** A store whose dispatchIntent applies the op straight back as an authoritative command,
 * so the sheet's subscribe()-bridged deriveds re-run — the deterministic component-level
 * stand-in for the live optimistic round-trip. */
function liveStore(documents: DocumentStore) {
  return (ops: { doc_id: string; changes: { path: string; new: unknown }[] }[]) => {
    documents.applyCommand({
      seq: documents.appliedSeq + 1, world_id: "w1", author: "u", ts: 0,
      ops: ops.map((o) => ({ op: "update" as const, doc_id: o.doc_id, changes: o.changes })),
    });
  };
}

function seed(): DocumentStore {
  const s = new DocumentStore();
  const actor = envelope("w1", "actor", null,
    sys({ str: { type: "number", order: 0, base: 2 }, attack: { type: "number", order: 1, base: 0, formula: "str + 1" } }),
    "a1", { displayName: "Aria", shape: "square", size: { w: 1, h: 1 } }, "Aria");
  actor.embedded = {
    item: [envelope("w1", "item", null, sys({}, { active: true, modifiers: { m: { stat: "str", op: "add", value: 2 } } }), "i0", undefined, "Belt")],
    effect: [envelope("w1", "effect", null, sys({}, { active: true, transfer: true, modifiers: { m: { stat: "attack", op: "add", value: 3 } } }), "e0", undefined, "Focus")],
  };
  s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [{ op: "create", doc: actor }] });
  return s;
}

describe("Nightfox sheet flow (spec §11)", () => {
  it("derived visible → equip belt changes value → toggle effect reverts contribution", async () => {
    const documents = seed();
    const context = setAppContextForTest({ documents, canEdit: () => true, dispatchIntent: liveStore(documents) as never });
    const { getByText, getByLabelText, queryByText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });

    // Baseline: str = 2 + belt(2) = 4; attack = final(str) + 1 + focus(3) = 8.
    expect(getByText("nightfox.stat.computed: 4")).toBeTruthy();
    expect(getByText("nightfox.stat.computed: 8")).toBeTruthy();

    // Un-equip the belt (active=false): str drops to 2, attack to 2 + 1 + 3 = 6.
    await fireEvent.click(getByLabelText("Belt nightfox.active"));
    expect(getByText("nightfox.stat.computed: 2")).toBeTruthy();
    expect(getByText("nightfox.stat.computed: 6")).toBeTruthy();
    expect(queryByText("nightfox.stat.computed: 4")).toBeNull();

    // Toggle the Focus effect off: attack reverts to base+belt-less = 3 (2 + 1, no focus, no belt).
    await fireEvent.click(getByLabelText("Focus nightfox.active"));
    expect(getByText("nightfox.stat.computed: 3")).toBeTruthy();
  });

  it("drag/drop reorder changes only order, never a value (D12)", async () => {
    const dispatched: { path: string; new: unknown }[] = [];
    const documents = seed();
    const apply = liveStore(documents);
    const context = setAppContextForTest({
      documents, canEdit: () => true,
      dispatchIntent: ((ops: { doc_id: string; changes: { path: string; new: unknown }[] }[]) => {
        for (const o of ops) for (const c of o.changes) dispatched.push(c);
        apply(ops);
      }) as never,
    });
    const { container, getByText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    const handles = container.querySelectorAll<HTMLElement>(".handle");
    await fireEvent.dragStart(handles[1]); // attack
    await fireEvent.drop(handles[0]);      // onto str
    // Only /order paths were written.
    expect(dispatched.length).toBeGreaterThan(0);
    expect(dispatched.every((c) => c.path.endsWith("/order"))).toBe(true);
    // Values unchanged after reorder: str still 4, attack still 8.
    expect(getByText("nightfox.stat.computed: 4")).toBeTruthy();
    expect(getByText("nightfox.stat.computed: 8")).toBeTruthy();
  });

  it("a second edit dispatches a fresh OCC old reflecting the first (subscribe bridge)", async () => {
    const calls: { path: string; old: unknown; new: unknown }[] = [];
    const documents = seed();
    const apply = liveStore(documents);
    const context = setAppContextForTest({
      documents, canEdit: () => true,
      dispatchIntent: ((ops: { doc_id: string; changes: { path: string; old: unknown; new: unknown }[] }[]) => {
        for (const o of ops) for (const c of o.changes) calls.push(c);
        apply(ops);
      }) as never,
    });
    const { getByLabelText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    // Edit str base twice; the 2nd old must be the 1st new (not the frozen initial).
    const base = getByLabelText("nightfox.stat.base") as HTMLInputElement; // str is first row
    await fireEvent.change(base, { target: { value: "5" } });
    await fireEvent.change(getByLabelText("nightfox.stat.base") as HTMLInputElement, { target: { value: "6" } });
    const strBaseEdits = calls.filter((c) => c.path === "/system/stats/str/base");
    expect(strBaseEdits).toEqual([
      { path: "/system/stats/str/base", old: 2, new: 5 },
      { path: "/system/stats/str/base", old: 5, new: 6 },
    ]);
  });
});
```

- [ ] **Step 2: Run to verify it passes**

Run: `pnpm --filter nightfox test flow.integration`
Expected: PASS (3 tests). A failure here is a real defect in an earlier task's component — fix the source, not the test.
Run: `pnpm --filter nightfox typecheck`
Expected: clean.

- [ ] **Step 3: Commit (inside the Nightfox repo)**

```bash
cd "C:\Dev\Nightfox"
git add src/sheets/flow.integration.test.ts
git commit -m "test(sheets): full author→equip→toggle→revert flow + reorder + OCC (spec §11)"
```

---

## Task 12: Documentation sync + reviewed skill-update gate

**Files:**
- Create: `C:\Dev\Nightfox\CHANGELOG.md` (Nightfox repo)
- Modify: `C:\Dev\Nightfox\README.md` (Nightfox repo — add a "Sheets" section)
- Modify (Shadowcat checkout): `docs/PLAN.md` (M13c done-row), `docs/POST_WORK_FINDINGS.md` (cross-repo friction), and the `.claude/skills/shadowcat-codebase-nightfox/SKILL.md` skill.

**Interfaces:** none (documentation only).

- [ ] **Step 1: Nightfox repo docs** — create `C:\Dev\Nightfox\CHANGELOG.md`:

```markdown
# Changelog

## Unreleased — M13c sheets

- Actor / item / effect sheets registered under `shadowcat.sheet:<doc_type>` at priority 10
  (above the generic sheets; a community sheet outbids with a higher priority).
- Stat table: per-type editors (number / resource / text / flag), computed-value preview,
  error/warning chips, tier-1 stat-key validation, presentation-only drag/drop reorder.
- Item/effect modifier editor (target key, op, literal-or-formula magnitude) with
  inert/dangling warnings; active (items+effects) and transfer (effects) toggles.
- All edits are field-path Updates with real OCC pre-images; sheets read the optimistic store.
```

Append to `C:\Dev\Nightfox\README.md` after the "Testing" section:

```markdown
## Sheets

Nightfox registers actor/item/effect sheets that render the `system.stats` model with live
computed values (via `@shadowcat/formula` + the M13b resolver), type-specific editors,
warning/error chips, and touch-friendly drag/drop stat reordering (order is presentation-only
and never changes a computed value). Sheet chrome uses a built-in English string map with a
`ctx.t` override hook (there is no external-module i18n-registration seam yet — see the
Shadowcat repo's `docs/POST_WORK_FINDINGS.md`).
```

- [ ] **Step 2: Commit the Nightfox docs (inside the Nightfox repo)**

```bash
cd "C:\Dev\Nightfox"
git add CHANGELOG.md README.md
git commit -m "docs(m13c): sheets changelog + README sheets section"
```

- [ ] **Step 3: Shadowcat-repo doc rows** — in the Shadowcat checkout, add the M13c done-row to `docs/PLAN.md` (house style, matching the M13b row), and add these friction entries to `docs/POST_WORK_FINDINGS.md` (the D16 cross-repo API bug-report channel):

```markdown
- Title: External-module i18n registration seam missing. Summary: An out-of-tree module
  (Nightfox sheets) has no public seam to register i18n keys into the shell catalog; M13c
  ships a built-in English fallback map with a `ctx.t` override hook as a workaround. Status:
  Needs Review (candidate engine seam for a later checkpoint).
- Title: `effect` doc_type constant has no engine home. Summary: D9 makes `effect` a
  client-semantics doc_type but neither M12c (which owns `ITEM_DOC_TYPE` in
  `scene-docs.ts`) nor the M13b rules plan declares an `EFFECT_DOC_TYPE`; M13c defines it in
  the Nightfox barrel. Consider promoting it beside `ITEM_DOC_TYPE` if a second consumer
  appears. Status: Needs Review.
- Title: No browser e2e harness for external modules. Summary: The M13-1 toolchain e2e is
  HTTP-only (no DOM); the spec §11 "Playwright e2e" for M13c has no browser harness, so the
  author→equip→toggle→revert flow is covered by a component-level integration test instead.
  Status: Needs Review (Playwright harness is a toolchain follow-up).
```

- [ ] **Step 4: Reviewed skill-update gate** — update `.claude/skills/shadowcat-codebase-nightfox/SKILL.md` (extend the M13b entry) with the M13c sheet seams: the `sheetView` resolve-from-top-level-host rule, the map-keyed stat/modifier CRUD idiom (add `old:null` / edit raw-old / remove whole-map-replace), presentation-only order (D12), the `nfT` i18n fallback, and priority-10 registration above the generics. Then dispatch `shadowcat-spec-reviewer` on the skill diff (the reviewed skill-update gate) to confirm the diff accurately captures the change with no drift or broken pointer.

- [ ] **Step 5: Commit the Shadowcat-repo docs (in the Shadowcat checkout, NOT the Nightfox repo)**

```bash
# from the Shadowcat checkout root
git add docs/PLAN.md docs/POST_WORK_FINDINGS.md .claude/skills/shadowcat-codebase-nightfox/SKILL.md
git commit -m "docs(m13c): PLAN done-row, cross-repo friction findings, nightfox skill update"
```

---

## Model/Effort directives

- Plan authored mainline (Fable 5, effort high; M13 track directive 2026-07-15).
- Execution: **subagent-driven-development** — implementers `shadowcat-coder` (sonnet, `effort: medium`), reviewers `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (`effort: high`), `-opus` twins on BLOCKED/shallow findings (project CLAUDE.md tiering).
- Execution gated on **M13-1** (toolchain + Nightfox bootstrap), **M13b** (rules source in the Nightfox repo), **M12c**, and **M13-0** — per-checkpoint branch in a git worktree; work in the nested dev clone (`<shadowcat>/src/modules/nightfox/`). Never push the Nightfox repo.

## Buddy-check directives

- **Pre-authorized task-level buddy-check: Task 7 (ActorSheet) — permission-sensitive rendering.** The actor sheet is the one surface that feeds a whole document graph (actor + embedded items/effects) into `resolveNightfox` and renders derived values. The reviewer must confirm: (1) the resolver is fed ONLY `ctx.documents` (the optimistic, per-recipient-redacted view), never `ctx.store`; (2) no computed value or chip exposes a field the recipient was not already sent (spec §10 — nothing hidden reaches the evaluator); (3) `readOnly` gates every write control via `ctx.canEdit`. Item/effect sheets (Tasks 8–9) inherit the same review lens; the whole-branch buddy-check covers them as a group.
- **No new egress/permission logic is introduced by this checkpoint.** Formula evaluation is client-side over already-redacted data; there are no new wire frames and no server change. The deferred GM-only-secret authoring affordance (which WOULD write `property_overrides` and be permission-sensitive) is explicitly OUT of scope (see Spec gaps) — if a later pass pulls it in, that task must be flagged.
- **Other risk signals:** the session broker decides conservatively (user asleep). The OCC pre-image correctness across all write helpers (Task 2) and the subscribe-bridge freshness (Tasks 7–9, asserted in Task 11) are the highest-value non-permission review targets. Customary **whole-branch buddy-check before the checkpoint merge**.

## Self-Review

**1. Spec coverage (§6 sheets + §10 security + relevant decisions):**
- Actor sheet: stat table (Tasks 5/7), per-type controls (Task 4), touch drag/drop reorder 44px (Task 5), formula inputs + live validation + computed preview (Tasks 3/4), error/warning chips (Tasks 3/4/6), inventory list + active toggles + openDocument (Task 7), effects list + active/transfer toggles (Task 7). Template actions (create/pull/push/revert) correctly NOT built (M13e). ✓
- Item/effect sheets: own stat block (Tasks 8/9), modifiers editor with target/op/magnitude literal-or-formula, validated-vs-owner / warned-when-dangling (Task 6), active/transfer toggles (Tasks 8/9). ✓
- Hard M12 rules: optimistic store read, field-path Updates with real OCC pre-images, `canEdit` advisory, write-site resolution via `openDocument`/`resolveDocRef` prefix (Tasks 2/7/8/9, asserted Task 11). ✓
- Tier-1 validation before dispatch (Task 5 stat keys via `validateStatKey`; Task 3 live formula validation). ✓
- i18n-keyed chrome, user labels/keys as data (Task 1 `nfT`; StatRow renders `label ?? key`). ✓
- §10 security: resolver over optimistic (redacted) data only; no new egress (Buddy-check Task 7). ✓
- D9 `effect` doc_type (Task 10); D10 priority-above-generic + community outbid (Task 10 priority 10); D11 map CRUD (Task 2); D12 presentation-only order (Tasks 5/11); D13/D14 `system.stats`/`system.mechanics` paths (Task 2). ✓
- Testing strategy §11 (component tests Tasks 4–9; full flow + reorder + OCC Task 11). ✓
- Repo targeting: all paths Nightfox-repo-relative, nested-execution note, commits inside the Nightfox repo, engine packages as build-time externals (header + Global Constraints + every commit block). ✓

**2. Placeholder scan:** No "TBD"/"similar to Task N"/"add validation"-style steps; every code step carries complete code; every consumed type/function is defined in a cited task or an existing file read during planning (`sheetContract`, `ITEM_DOC_TYPE`, `envelope`, `getPointer`, `actorDisplayName`, `setField`, `setAppContextForTest`, `resolveDocRef` prefix contract, M13b `parseNightfox`/`resolveNightfox`/`validateStatKey`/types, M13a `parseFormula`/`isFormulaError`/`FormulaValue`). ✓

**3. Type consistency:** `sheetView`/`SheetView`, `NfWarning`, `statsPath`/`modifiersPath`, `addStat`/`editStatField`/`removeStat`/`setStatOrder`/`addModifier`/`editModifierField`/`removeModifier`/`setMechanicsFlag`, `formatValue`/`formulaIssues`/`warningChips`/`ChipDescriptor`/`DisplayValue`, `nfT`/`NF_MESSAGES`, `EFFECT_DOC_TYPE` — used identically across defining and consuming tasks. Sheet component prop contract `{ docId, systemPrefix, close }` matches `sheetsController.#register`. `ResolvedStat` shape (`final` for number, `max`/`effectiveCurrent` for resource, `value` for text/boolean) consumed consistently in Tasks 4/7/8/9/11. ✓

## Spec gaps (surfaced, not silently resolved)

1. **`effect` doc_type has no engine/rules home.** D9 declares `effect` a client-semantics doc_type "exactly as M12c introduces item", but `ITEM_DOC_TYPE` lives in the engine (`scene-docs.ts`) and the M13b plan never declares an `EFFECT_DOC_TYPE` constant or an effect-doc builder — it only uses the string `"effect"` in fixtures. This plan defines `EFFECT_DOC_TYPE = "effect"` in the Nightfox barrel (Task 10) and files it as friction (Task 12). A human should decide whether it belongs beside `ITEM_DOC_TYPE` in the engine.
2. **Per-stat roll buttons — checkpoint boundary conflict.** Spec §6 lists "per-stat roll buttons" under the actor sheet, but §13 assigns per-stat roll templates → chat to **M13d** (gating on M13b+M11, not M13c). This plan follows §13 (authoritative for checkpoint decomposition) and builds NO roll affordance in M13c. If the intent was to ship the roll BUTTON UI in M13c with M13d only wiring the transport, that is a one-task addition — flag for a human.
3. **No document-creation affordance for items/effects.** The spec's sheet bullets cover the inventory/effects *lists* (open + toggle) and stat/modifier CRUD, but nothing specifies how a user creates a new item or effect document and embeds it on an actor (only stats and modifiers are single-key field writes; whole embedded-doc creation is not a field write). This plan builds stat/modifier CRUD but no "add item/effect" button. A human should confirm whether embedded-doc creation belongs in M13c or is deferred.
4. **GM-only secret text on items/effects (spec §12 deferred candidate).** Listed as a candidate for "M13c or a later sheet pass". Deferred here (it introduces `property_overrides` authoring — permission-sensitive). A human should decide whether to pull it into this checkpoint (it would add a buddy-check-flagged task).
5. **External-module i18n registration seam absent.** No public seam exists for an out-of-tree module to register i18n keys; mitigated by the `nfT` English fallback (Task 1) and filed as friction (Task 12). Chrome renders English until a seam or host override ships.
6. **Playwright e2e vs. toolchain reality.** Spec §11 asks for a Playwright e2e; the locked M13-1 toolchain provides only an HTTP (no-DOM) e2e harness. The DOM-level flow is covered by a component-level integration test (Task 11); a true browser pass is a toolchain follow-up (filed as friction, Task 12).
