# M13b · `@shadowcat/module-nightfox` — Headless Rules Package Implementation Plan

> **⚠ RE-TARGET BEFORE EXECUTION (D16, 2026-07-15):** Nightfox is an external project — its own
> GitHub repository and project folder, consuming engine packages through the real third-party
> path. Task 1's scaffolding (paths, package.json dependency mechanism, workspace/install steps)
> and every `src/modules/nightfox/...` path in this plan re-target the Nightfox repo once the
> **M13-1 external-module toolchain** spec locks the consumption mechanism. The task BODIES
> (schemas, contribution collection, resolver, buckets, tests) are repo-agnostic and stand as
> written. Do not execute this plan until it has been revised against the M13-1 spec.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Nightfox rules engine as a headless module package: the `system.stats` + `system.mechanics` data model (Zod tier-1 validation), the one-dependency-graph resolver with typed commutative modifier buckets, and `item`/`effect` semantics (`active`/`transfer`) — spec §§4–5 (`docs/superpowers/specs/2026-07-15-m13-nightfox-system-design.md`, decisions D2–D4, D7–D9, D11–D14).

**Architecture:** Pure functions over `WireDocument`s (no Svelte, no store subscription — sheets call the resolver with docs they already read). Formula mechanics come from `@shadowcat/formula`. Data lives in the two engine-reserved system directories (D13/D14): `system.stats` (the variables directory — universal location, Nightfox-defined entry shape) and `system.mechanics` (the singleton system's non-variable model data). EVERYTHING in this package (stat types, buckets, `parent`/`base` vocabulary, reserved keys) is a Nightfox convention other systems may ignore or replace. Zero server change; `effect` becomes a client-semantics doc_type exactly as M12c introduces `item`. Independent of M13-0 (this package never reads the `engine` band).

**Tech Stack:** TypeScript, Zod, Vitest; deps `@shadowcat/core` (types), `@shadowcat/formula`.

## Global Constraints

- New package `src/modules/nightfox/`, name `@shadowcat/module-nightfox` (workspace glob `src/modules/*` picks it up).
- Nightfox data lives ONLY in `system.stats` and `system.mechanics` (D13/D14) — never other system keys, never the envelope, never the `engine` band.
- Readers fail closed: `parseNightfox` returns `null` when BOTH directories are absent (not a Nightfox-bearing doc) or when EITHER present directory is malformed; an absent side defaults (`stats: {}` / `mechanics: {version: 1}`). The resolver treats `null` blocks as "no Nightfox data", never throws.
- Caps (tier-1 validation): `MAX_STATS = 128`, `MAX_MODIFIERS = 128` per document; `label` ≤ 64 chars; `text.value` ≤ 1024 chars; formula strings ≤ `MAX_FORMULA_LENGTH` (512, re-exported from `@shadowcat/formula`).
- Stat keys: `/^[a-z][a-z0-9_]*$/`, max 32 chars, not in `RESERVED_STAT_KEYS`, no dice-notation collision (rules in Task 2).
- The permutation invariant (spec D3/D12) is a tested property: shuffling embed arrays, modifier-record insertion order, and stat-record insertion order never changes any resolved value.
- Booleans coerce to 1/0 in formulas; text references are `type` errors; `%`/`/` semantics live in the library — this package adds no arithmetic.
- Commit per task once green (`pnpm --filter @shadowcat/module-nightfox test` + `typecheck`).

---

### Task 1: Package scaffold

**Files:**
- Create: `src/modules/nightfox/package.json`
- Create: `src/modules/nightfox/tsconfig.json` (copy `src/client/core/tsconfig.json` verbatim — this is a pure-TS package, not a Svelte one)
- Create: `src/modules/nightfox/vitest.config.ts`
- Create: `src/modules/nightfox/src/index.ts` (placeholder barrel: `export {};`)

- [ ] **Step 1: package.json:**

```json
{
  "name": "@shadowcat/module-nightfox",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "main": "src/index.ts",
  "dependencies": {
    "@shadowcat/core": "workspace:*",
    "@shadowcat/formula": "workspace:*",
    "zod": "^3.23.8"
  },
  "devDependencies": {
    "@types/node": "^22.0.0"
  },
  "scripts": {
    "typecheck": "tsc --noEmit",
    "test": "vitest run"
  }
}
```

- [ ] **Step 2: vitest.config.ts:**

```ts
import { defineConfig } from "vitest/config";
export default defineConfig({ test: { include: ["src/**/*.test.ts"] } });
```

- [ ] **Step 3:** `pnpm install`; then `pnpm --filter @shadowcat/module-nightfox typecheck`. Expected: green (empty barrel). Commit — `feat(nightfox): scaffold @shadowcat/module-nightfox package`

---

### Task 2: Schema, fail-closed parse, key validation (`nightfox-docs.ts`)

**Files:**
- Create: `src/modules/nightfox/src/nightfox-docs.ts`
- Test: `src/modules/nightfox/src/nightfox-docs.test.ts`

**Interfaces:**
- Produces (exact exported surface later tasks and M13c/M13d consume):

```ts
export const NIGHTFOX_VERSION = 1;
export type StatType = "number" | "resource" | "text" | "boolean";
export type Stat =
  | { type: "number"; order: number; label?: string; base: number; formula?: string; roll?: string }
  | { type: "resource"; order: number; label?: string; current: number; maxBase: number;
      maxFormula?: string; clampToMax?: boolean; roll?: string }
  | { type: "text"; order: number; label?: string; value: string }
  | { type: "boolean"; order: number; label?: string; value: boolean };
export type ModifierOp = "add" | "mulAdditive" | "mulCompound";
export interface Modifier { stat: string; op: ModifierOp; value: string | number }
export interface NightfoxMechanics {
  version: 1;
  modifiers?: Record<string, Modifier>;  // items + effects only (resolver-enforced)
  active?: boolean;                      // items + effects; resolver default: true
  transfer?: boolean;                    // effects only; resolver default: false
}
export interface NightfoxBlock { stats: Record<string, Stat>; mechanics: NightfoxMechanics }
export const RESERVED_STAT_KEYS: ReadonlySet<string>;
export function validateStatKey(key: string): string[];   // [] = valid; else human-readable issues
export function validateNightfox(v: { stats?: unknown; mechanics?: unknown }):
  { block: NightfoxBlock } | { issues: string[] };
export function parseNightfox(doc: { system?: unknown } | null | undefined): NightfoxBlock | null;
```

- [ ] **Step 1: Write the failing tests:**

```ts
import { describe, expect, it } from "vitest";
import { parseNightfox, validateStatKey, validateNightfox } from "./nightfox-docs";

const goodStats = {
  dex: { type: "number", order: 0, base: 3, formula: "dex + str", roll: "d20 + dex" },
  hp: { type: "resource", order: 1, current: 10, maxBase: 10, maxFormula: "10 + str" },
  class: { type: "text", order: 2, value: "ranger" },
  inspired: { type: "boolean", order: 3, value: false },
  str: { type: "number", order: 4, base: 2 },
};

describe("parseNightfox", () => {
  it("parses system.stats + system.mechanics", () => {
    const b = parseNightfox({ system: { stats: goodStats, mechanics: { version: 1 } } });
    expect(b?.stats.dex.type).toBe("number");
    expect(b?.mechanics.version).toBe(1);
  });
  it("an absent side defaults (stats-only doc; mechanics-only doc)", () => {
    expect(parseNightfox({ system: { stats: goodStats } })?.mechanics).toEqual({ version: 1 });
    const m = parseNightfox({ system: { mechanics: { version: 1, modifiers: { m1: { stat: "dex", op: "add", value: 1 } } } } });
    expect(m?.stats).toEqual({});
    expect(m?.mechanics.modifiers?.m1.stat).toBe("dex");
  });
  it("both directories absent -> null (not a Nightfox-bearing doc)", () => {
    expect(parseNightfox({ system: {} })).toBeNull();
    expect(parseNightfox({ system: { grid: {} } })).toBeNull();
    expect(parseNightfox(null)).toBeNull();
  });
  it("fails closed when a PRESENT directory is malformed", () => {
    expect(parseNightfox({ system: { stats: { x: { type: "number" } } } })).toBeNull();
    expect(parseNightfox({ system: { stats: goodStats, mechanics: { version: 2 } } })).toBeNull();
    expect(parseNightfox({ system: { stats: 7 } })).toBeNull();
  });
  it("fails closed on non-finite numbers", () => {
    const b = structuredClone(goodStats);
    (b.dex as { base: number }).base = Infinity;
    expect(parseNightfox({ system: { stats: b } })).toBeNull();
  });
});

describe("validateStatKey", () => {
  it("accepts ordinary keys", () => {
    for (const k of ["dex", "hp", "base_attack_bonus", "damage", "total"])
      expect(validateStatKey(k)).toEqual([]);
  });
  it("rejects reserved vocabulary", () => {
    for (const k of ["parent", "base", "current", "min", "max", "floor", "ceil", "round"])
      expect(validateStatKey(k).length).toBeGreaterThan(0);
  });
  it("rejects dice-notation collisions (exact keyword or keyword+digit)", () => {
    for (const k of ["d", "kh", "e", "t", "r", "d20", "kh3", "e5"])
      expect(validateStatKey(k).length).toBeGreaterThan(0);
  });
  it("rejects bad patterns", () => {
    for (const k of ["Dex", "2hp", "hp-max", "", "a".repeat(33)])
      expect(validateStatKey(k).length).toBeGreaterThan(0);
  });
});

describe("validateNightfox", () => {
  it("returns structured issues for sheet display, including per-key failures", () => {
    const bad = structuredClone(goodStats) as Record<string, unknown>;
    bad["kh3"] = { type: "number", order: 5, base: 1 };
    const r = validateNightfox({ stats: bad });
    expect("issues" in r && r.issues.some((i) => i.includes("kh3"))).toBe(true);
  });
});
```

- [ ] **Step 2: Run — expect FAIL. Step 3: Implement.** Zod: `StatSchema` discriminated union on `type`, every numeric field `z.number().finite()`; `StatsDirSchema = z.record(StatSchema)` with `superRefine` applying `validateStatKey` to every key + the `MAX_STATS` cap; `MechanicsSchema = z.object({ version: z.literal(1), modifiers: z.record(ModifierSchema).optional(), active: z.boolean().optional(), transfer: z.boolean().optional() }).strict()` with the `MAX_MODIFIERS` cap; `.strict()` on all objects (unknown fields fail closed — mirrors `deny_unknown_fields`). `parseNightfox`: read `(doc?.system as {stats?: unknown; mechanics?: unknown})`; both `undefined` → `null`; safeParse each present side, either failing → `null`; absent sides default (`{}` / `{version: 1}`). `validateStatKey`: pattern `/^[a-z][a-z0-9_]*$/` + length ≤ 32 + `RESERVED_STAT_KEYS` membership + notation collision — a key collides when its maximal `[a-z_]+` prefix ∈ `NOTATION_KEYWORDS` (import from `@shadowcat/formula`) AND (the prefix is the whole key OR the next char is a digit). `RESERVED_STAT_KEYS = new Set(["parent","base","current","min","max","floor","ceil","round", ...NOTATION_KEYWORDS])`. `validateNightfox` = same schemas, returning issue strings (sheet-facing).

- [ ] **Step 4: Run (PASS). Step 5: Commit** — `feat(nightfox): system.stats/system.mechanics Zod schema + fail-closed parse + reserved-key validation`

---

### Task 3: Contribution collection (`contributions.ts`)

**Files:**
- Create: `src/modules/nightfox/src/contributions.ts`
- Test: `src/modules/nightfox/src/contributions.test.ts`

**Interfaces:**
- Consumes: `parseNightfox`, `NightfoxBlock`, `Modifier` (Task 2); `WireDocument` from `@shadowcat/core`.
- Produces:

```ts
export interface ModifierContribution {
  modId: string;
  carrierId: string;   // doc whose mechanics declare the modifier (bare-identifier scope)
  targetId: string;    // doc whose stat is modified (parent.* scope in magnitudes)
  modifier: Modifier;
}
export interface NightfoxWarning { docId: string; kind: "host-modifiers-inert" | "dangling-modifiers"; detail: string }
export interface DocEntry { doc: WireDocument; block: NightfoxBlock; parentId: string | null }
export interface Collected { docs: Map<string, DocEntry>; contributions: ModifierContribution[]; warnings: NightfoxWarning[] }
export function collectNightfox(host: WireDocument): Collected;
```

- [ ] **Step 1: Write the failing tests** — plain `WireDocument`-shaped fixtures via helpers (these three helpers are the canonical fixture idiom; Tasks 4/5 duplicate them):

```ts
import { describe, expect, it } from "vitest";
import { collectNightfox } from "./contributions";

const sys = (stats: Record<string, unknown> = {}, mech: Record<string, unknown> = {}) =>
  ({ stats, mechanics: { version: 1, ...mech } });
const doc = (id: string, doc_type: string, system: unknown, embedded: Record<string, unknown[]> = {}) =>
  ({ id, doc_type, system, embedded }) as never;
const mod = (stat: string, op = "add", value: string | number = 1) => ({ stat, op, value });

describe("collectNightfox", () => {
  it("collects actor-embedded item modifiers targeting the actor", () => {
    const host = doc("A", "actor", sys(), {
      item: [doc("I", "item", sys({}, { modifiers: { m1: mod("dex") } }))],
    });
    expect(collectNightfox(host).contributions).toEqual([
      { modId: "m1", carrierId: "I", targetId: "A", modifier: mod("dex") },
    ]);
  });
  it("actor-embedded effects target the actor; item-embedded effects target the item unless transfer", () => {
    const host = doc("A", "actor", sys(), {
      item: [doc("I", "item", sys(), {
        effect: [
          doc("E1", "effect", sys({}, { modifiers: { m: mod("damage") } })),                 // -> item
          doc("E2", "effect", sys({}, { transfer: true, modifiers: { m: mod("str") } })),    // -> actor
        ],
      })],
      effect: [doc("E3", "effect", sys({}, { modifiers: { m: mod("ac") } }))],               // -> actor
    });
    const targets = Object.fromEntries(collectNightfox(host).contributions.map((c) => [c.carrierId, c.targetId]));
    expect(targets).toEqual({ E1: "I", E2: "A", E3: "A" });
  });
  it("active gating: inactive carrier suppresses its modifiers AND its embedded effects", () => {
    const host = doc("A", "actor", sys(), {
      item: [doc("I", "item", sys({}, { active: false, modifiers: { m: mod("dex") } }), {
        effect: [doc("E", "effect", sys({}, { transfer: true, modifiers: { m: mod("str") } }))],
      })],
    });
    expect(collectNightfox(host).contributions).toEqual([]);
  });
  it("inactive effect suppresses only itself", () => {
    const host = doc("A", "actor", sys(), {
      effect: [
        doc("E", "effect", sys({}, { active: false, modifiers: { m: mod("x") } })),
        doc("F", "effect", sys({}, { modifiers: { m: mod("y") } })),
      ],
    });
    expect(collectNightfox(host).contributions.map((c) => c.carrierId)).toEqual(["F"]);
  });
  it("modifiers on an actor host are inert + warned; on a standalone item host, dangling", () => {
    const a = collectNightfox(doc("A", "actor", sys({}, { modifiers: { m: mod("dex") } })));
    expect(a.contributions).toEqual([]);
    expect(a.warnings[0]).toMatchObject({ docId: "A", kind: "host-modifiers-inert" });
    const i = collectNightfox(doc("I", "item", sys({}, { modifiers: { m: mod("dex") } })));
    expect(i.warnings[0]).toMatchObject({ docId: "I", kind: "dangling-modifiers" });
  });
  it("docs without a parseable block contribute nothing and break nothing", () => {
    const host = doc("A", "actor", sys(), { item: [doc("I", "item", { stats: 7 })] });
    expect(collectNightfox(host).contributions).toEqual([]);
    expect(collectNightfox(host).docs.has("I")).toBe(false);
  });
});
```

- [ ] **Step 2: Run — expect FAIL. Step 3: Implement.** Walk exactly: host → `embedded["item"] ?? []` → each item's `embedded["effect"] ?? []`, plus host's `embedded["effect"] ?? []`. `parseNightfox` each doc; `null` ⇒ the doc is skipped entirely (fail-closed). Effective activity: `mechanics.active !== false` AND carrier-chain active. Targets per spec §5.3 (the test table above is normative). Host-level `mechanics.modifiers` ⇒ `host-modifiers-inert` warning, except a host of doc_type `item`/`effect` (standalone sheet preview) ⇒ `dangling-modifiers`.

- [ ] **Step 4: Run (PASS). Step 5: Commit** — `feat(nightfox): modifier contribution collection with active/transfer gating`

---

### Task 4: The resolver (`resolve.ts`)

**Files:**
- Create: `src/modules/nightfox/src/resolve.ts`
- Test: `src/modules/nightfox/src/resolve.test.ts`

**Interfaces:**
- Consumes: `Collected` (Task 3); `parseFormula`, `evaluate`, `resolveAll`, `FormulaValue`, `isFormulaError` from `@shadowcat/formula`.
- Produces:

```ts
export type ResolvedStat =
  | { type: "number"; order: number; label?: string; base: number; final: FormulaValue }
  | { type: "resource"; order: number; label?: string; current: number;
      max: FormulaValue; effectiveCurrent: FormulaValue }
  | { type: "text"; order: number; label?: string; value: string }
  | { type: "boolean"; order: number; label?: string; value: boolean };
export interface ResolveWarning { docId: string; kind: "inert-missing-stat" | "inert-unmodifiable-stat"; detail: string }
export interface ResolvedDocs {
  byDoc: Map<string, Map<string, ResolvedStat>>;   // docId -> statKey -> resolved
  warnings: (ResolveWarning | import("./contributions").NightfoxWarning)[];
}
export function resolveNightfox(host: WireDocument): ResolvedDocs;
/** Reference resolver over one resolved doc, for roll templates (M13d) and sheet formula
 * previews: bare key -> number final / resource effectiveCurrent / boolean 1|0;
 * key.max, key.current, base.key supported; text -> type error; parent.* -> unknown-ref. */
export function statRefResolver(
  resolved: Map<string, ResolvedStat>,
  block: NightfoxBlock,
): (path: string[]) => FormulaValue;
```

- [ ] **Step 1: Write the failing tests** (the normative spec-§5 semantics table; fixture helpers `sys`/`doc`/`mod` duplicated from Task 3):

```ts
import { describe, expect, it } from "vitest";
import { resolveNightfox } from "./resolve";
// sys/doc/mod helpers exactly as in contributions.test.ts

const finalOf = (r: ReturnType<typeof resolveNightfox>, docId: string, key: string) => {
  const s = r.byDoc.get(docId)?.get(key);
  return s && "final" in s ? s.final : s && "max" in s ? s.max : undefined;
};
const num = (base: number, formula?: string) => ({ type: "number", order: 0, base, ...(formula ? { formula } : {}) });

describe("resolveNightfox — spec §5 semantics", () => {
  it("L2: derived formula; self-ref reads own base (D8)", () => {
    const host = doc("A", "actor", sys({ dex: num(3, "dex + 1") }));
    expect(finalOf(resolveNightfox(host), "A", "dex")).toBe(4);
  });
  it("cross-refs read FINAL values: attack sees the str belt", () => {
    const host = doc("A", "actor", sys({ str: num(2), attack: num(0, "str + 1") }), {
      item: [doc("I", "item", sys({}, { modifiers: { m: mod("str", "add", 2) } }))],
    });
    expect(finalOf(resolveNightfox(host), "A", "attack")).toBe(5); // (2+2) + 1
  });
  it("bucket pipeline: (derived + Σadd) × (1 + ΣmulAdditive) × ΠmulCompound", () => {
    const host = doc("A", "actor", sys({ dex: num(10) }), {
      item: [doc("I", "item", sys({}, { modifiers: {
        a: mod("dex", "add", 2), b: mod("dex", "mulAdditive", 0.1),
        c: mod("dex", "mulAdditive", 0.2), d: mod("dex", "mulCompound", 2) } }))],
    });
    expect(finalOf(resolveNightfox(host), "A", "dex")).toBeCloseTo((10 + 2) * 1.3 * 2);
  });
  it("magnitude formulas: bare = carrier stats, parent.* = target finals (D4/§5.3)", () => {
    const host = doc("A", "actor", sys({ str: num(4), damage: num(0) }), {
      item: [doc("I", "item", sys(
        { quality: num(3) },
        { modifiers: { m: mod("damage", "add", "quality + floor(parent.str / 2)") } }))],
    });
    expect(finalOf(resolveNightfox(host), "A", "damage")).toBe(5); // 3 + floor(4/2)
  });
  it("magnitude referencing its own target's final is a cycle error (§5.2)", () => {
    const host = doc("A", "actor", sys({ dex: num(1) }), {
      item: [doc("I", "item", sys({}, { modifiers: { m: mod("dex", "add", "parent.dex") } }))],
    });
    expect(finalOf(resolveNightfox(host), "A", "dex")).toMatchObject({ error: "cycle" });
  });
  it("cross-stat cycles mark every participant errored", () => {
    const host = doc("A", "actor", sys({ a: num(0, "b"), b: num(0, "a") }));
    const r = resolveNightfox(host);
    expect(finalOf(r, "A", "a")).toMatchObject({ error: "cycle" });
    expect(finalOf(r, "A", "b")).toMatchObject({ error: "cycle" });
  });
  it("resource: modifiers hit max; effectiveCurrent clamps at read time; no low clamp", () => {
    const host = doc("A", "actor", sys({ hp: { type: "resource", order: 0, current: 12, maxBase: 10 } }), {
      item: [doc("I", "item", sys({}, { modifiers: { m: mod("hp", "add", -4) } }))],
    });
    expect(resolveNightfox(host).byDoc.get("A")!.get("hp")).toMatchObject({ max: 6, effectiveCurrent: 6 });
  });
  it("clampToMax: false keeps current unclamped", () => {
    const host = doc("A", "actor",
      sys({ hp: { type: "resource", order: 0, current: 12, maxBase: 10, clampToMax: false } }));
    expect(resolveNightfox(host).byDoc.get("A")!.get("hp")).toMatchObject({ effectiveCurrent: 12 });
  });
  it("booleans coerce to 1/0 in formulas; text refs are type errors (D7)", () => {
    const host = doc("A", "actor", sys({
      inspired: { type: "boolean", order: 0, value: true },
      class: { type: "text", order: 1, value: "bard" },
      bonus: num(0, "2 * inspired"),
      bad: num(0, "class + 1") }));
    const r = resolveNightfox(host);
    expect(finalOf(r, "A", "bonus")).toBe(2);
    expect(finalOf(r, "A", "bad")).toMatchObject({ error: "type" });
  });
  it("tolerance: modifier to a missing or text/boolean stat is inert + warned (§5.4)", () => {
    const host = doc("A", "actor", sys({ class: { type: "text", order: 0, value: "bard" } }), {
      item: [doc("I", "item", sys({}, { modifiers: { m1: mod("ghost"), m2: mod("class") } }))],
    });
    const r = resolveNightfox(host);
    expect(r.warnings.map((w) => w.kind).sort()).toEqual(["inert-missing-stat", "inert-unmodifiable-stat"]);
  });
  it("item stats are themselves resolved (non-transfer effect modifies the item)", () => {
    const host = doc("A", "actor", sys(), {
      item: [doc("I", "item", sys({ damage: num(2) }), {
        effect: [doc("E", "effect", sys({}, { modifiers: { m: mod("damage") } }))] })],
    });
    expect(finalOf(resolveNightfox(host), "I", "damage")).toBe(3);
  });
  it("derived formulas' parent.* reads the embed parent (item formula sees actor stat)", () => {
    const host = doc("A", "actor", sys({ str: num(6) }), {
      item: [doc("I", "item", sys({ heft: num(0, "parent.str / 2") }))],
    });
    expect(finalOf(resolveNightfox(host), "I", "heft")).toBe(3);
  });
  it("an errored magnitude poisons the target stat (never silently drops)", () => {
    const host = doc("A", "actor", sys({ dex: num(1) }), {
      item: [doc("I", "item", sys({}, { modifiers: { m: mod("dex", "add", "ghost") } }))],
    });
    expect(finalOf(resolveNightfox(host), "A", "dex")).toMatchObject({ error: "unknown-ref" });
  });
});
```

- [ ] **Step 2: Run — expect FAIL. Step 3: Implement.** One `resolveAll` call over node keys `f:<docId>#<key>` (final) and `c:<docId>#<key>` (resource effectiveCurrent). Node eval for `f:` — (1) *derived*: `formula` present ⇒ `evaluate(parseFormula(formula), derivedRefs)` else `base` (`maxFormula`/`maxBase` for resources); (2) *buckets*: partition this stat's contributions by op, magnitudes evaluated with `magnitudeRefs` (a numeric `value` is used directly); any errored magnitude poisons the final (propagate — never silently drop a belt); (3) fold `(derived + Σadd) * (1 + ΣmulAdd) * ΠmulComp`, `Number.isFinite` gate ⇒ `non-finite`. `derivedRefs` (spec §5.2/§5.3): `[selfKey]` ⇒ own base directly (NOT via `get`); `["base", k]` ⇒ k's base (resource ⇒ `maxBase`); `[k]` ⇒ boolean 1/0 · text `{error:"type"}` · resource `get("c:…")` · number `get("f:…")`; `[k,"max"]` ⇒ `get("f:…")`; `[k,"current"]` ⇒ `get("c:…")`; `["parent",...rest]` ⇒ same rules against the embed parent (host ⇒ `unknown-ref`). `magnitudeRefs`: identical EXCEPT bare identifiers scope to the CARRIER doc and `parent.*` scopes to the TARGET doc. `c:` node: `clampToMax !== false` ⇒ min(current, final-max) with error propagation, else `current`. Inert rules (§5.4) checked at partition time (target stat missing / text / boolean ⇒ skip + `ResolveWarning`; resource target ⇒ applies to max). `statRefResolver` reuses `derivedRefs` over the finished maps.

- [ ] **Step 4: Run (PASS). Step 5: Commit** — `feat(nightfox): one-graph stat resolver with commutative bucket pipeline`

---

### Task 5: Permutation + property battery

**Files:**
- Test: `src/modules/nightfox/src/permutation.test.ts`

- [ ] **Step 1: Write the suite** (seeded PRNG — copy the 10-line `rng` helper from the M13a plan's Task 7 into this file). Build a randomized actor: 8 number/resource stats (formulas drawn from a safe DAG template over earlier keys), 4 items × up to 3 effects with random modifiers (random target key, random op, magnitude from `{literal, "parent.<k>", "<carrierStat>"}`, random `active`/`transfer`). For 100 seeds, construct the SAME logical document three ways — (a) shuffled `embedded.item`/`embedded.effect` array orders, (b) shuffled object-key insertion order for `stats` and `modifiers` records (`Object.fromEntries(shuffledEntries)`), (c) shuffled `order` field values — and assert `resolveNightfox` output deep-equals across all three (Maps converted to sorted arrays). This is the D3/D12 invariant — the test the bucket revision exists for.
- [ ] **Step 2:** Also assert: no construction throws; every resolved value is `number | FormulaError`.
- [ ] **Step 3: Run (PASS — a failure is a Task 4 bug; fix the source). Commit** — `test(nightfox): permutation-invariance property battery`

---

### Task 6: Module export + barrel + full gate

**Files:**
- Modify: `src/modules/nightfox/src/index.ts`
- Test: `src/modules/nightfox/src/index.test.ts`

**Interfaces:**
- Produces: `export const nightfox: Module` (manifest `{ id: "nightfox", version: "0.1.0", dependencies: {}, requires: [], provides: [] }`, empty `register()` — headless; M13c's sheets package will declare `dependencies: { nightfox: "^0.1.0" }`), plus the full pure API barrel: Tasks 2–4 exports and re-exports of `NOTATION_KEYWORDS`, `MAX_FORMULA_LENGTH`.

- [ ] **Step 1: Test** — import `{ nightfox }`, assert it typechecks as `Module` (from `@shadowcat/core`, same import as `src/modules/factions/src/index.ts`) and `nightfox.manifest.id === "nightfox"`.
- [ ] **Step 2: Implement barrel. Step 3: Full gate:** `pnpm -r typecheck && pnpm -r test && pnpm lint`. Expected: green.
- [ ] **Step 4: Commit** — `feat(nightfox): module manifest + public API barrel`

---

### Task 7: Docs + codebase-skill gate

**Files:**
- Modify: `docs/PLAN.md` (M13b done-entry, house style)
- Modify: `docs/design/ARCHITECTURE.md` (document the D13/D14/D15 conventions: the reserved `system.stats`/`system.mechanics` directories, the singleton-system premise, and — once M13-0 lands — the three-band document shape; ARCHITECTURE is the durable home since CLAUDE.md is git-ignored)
- Modify: `.claude/skills/shadowcat-codebase-nightfox/SKILL.md` (extend with: stat model, bucket pipeline + permutation invariant, §5.2/§5.3 scope rules, tolerance rules, fail-closed parse conventions)

- [ ] **Step 1:** Update all three; dispatch `shadowcat-spec-reviewer` on the skill diff (reviewed skill-update gate).
- [ ] **Step 2:** Commit — `docs(m13b): PLAN + ARCHITECTURE sync, nightfox codebase-skill update`

---

## Model/Effort directives

- Plan authored mainline on Fable 5, effort high (user directive 2026-07-15; tier-switch checkpoint outcome).
- Execution: **subagent-driven-development** — implementers `shadowcat-coder` (sonnet, `effort: medium`), reviewers `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (`effort: high`), `-opus` twins on BLOCKED/shallow findings (project CLAUDE.md tiering).
- **Execution gated on M12 completion and M13a merged** (user decision 2026-07-15) — per-checkpoint branch in a git worktree (shared tree with the M12 session).

## Buddy-check directives

- **Pre-authorized task-level buddy-check: Task 4 (the resolver/bucket pipeline)** — the correctness core of the whole system.
- Standard two-reviewer gates on all other tasks (Task 5's permutation battery serves as the resolver's independent oracle); customary whole-branch buddy-check before the checkpoint merge.
