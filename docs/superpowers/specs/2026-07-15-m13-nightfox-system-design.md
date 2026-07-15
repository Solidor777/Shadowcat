# M13 · Nightfox — First-Party Generic System Module — Design

**Status:** Approved (brainstorm 2026-07-15). Cross-cutting spec; each checkpoint gets its own
plan cycle (M13e and M13f additionally get their own sub-specs).

## 1. Goals & placement

Nightfox is Shadowcat's first-party generic game system. Two purposes, in priority order:

1. **A playable generic system** — users run most tabletop games with minimal overhead:
   per-document configurable stats, derived formulas, rolls to chat, items/effects that modify
   stats, and template documents (base Player, base Monster) a table can build a simple homebrew
   system from without writing code.
2. **The reference implementation for system/module builders** — Nightfox is built ONLY against
   public seams. Every friction point is logged in `docs/POST_WORK_FINDINGS.md` as an API bug
   report (the M12 sheets discipline), and it is the second internal system the Phase-4 API
   freeze gate wants as evidence.

**Placement: M13**, after M12 (needs M12c's sheet registry + `item` doc_type) and after M12.5,
so the dogfood-alpha gate opens with backups *and* a playable system. M13a/M13b are pure
headless packages with zero M12 dependency and may start while M12 wraps up; only M13c gates on
M12c.

## 2. Decisions locked

| # | Decision |
|---|---|
| D1 | **Formula engine is a shared library** (`@shadowcat/formula`), engine-owned, framework-neutral — not Nightfox-private. Community systems reuse it instead of hand-rolling evaluators. |
| D2 | **Two-role split**: the *parser* computes values (derived stat formulas, roll templates — free-form arithmetic); *typed operation buckets* are the only way another document modifies a stat. A modifier never injects a formula into its target. |
| D3 | **Bucket pipeline** per stat: `final = (derived + Σ add) × (1 + Σ mulAdditive) × Π mulCompound`. Fixed stage order; commutative within each stage ⇒ result is independent of modifier/embed order, by construction. |
| D4 | **Magnitudes are formulas**: a bucket operation's magnitude is itself a parser formula (e.g. add `parent.str / 2` to damage), evaluated through the same cycle-guarded graph, then applied through the bucket. |
| D5 | **Templates = provenance stamping + on-command 3-way merge** (pull / push / revert), reusing the previously-devised merge model ([[document-inheritance-merge-model]]). No live inheritance; no read-time merge. |
| D6 | **Validation is phased**: tier 1 (this milestone, client) — Zod write-validation in sheets + fail-closed readers; tier 2 (final checkpoint, server) — a declarative, data-driven schema registry (M6b Capability-Phase-2 precedent; invariant 6 intact — the server never runs system code). |
| D7 | **Four stat types**: `number`, `resource` (hand-edited `current` + formula-bearing `max`), `text`, `boolean`. Formulas/modifiers apply to numbers and resource max only; booleans coerce to 1/0 in formulas; text in a formula is an error. |
| D8 | **Self-reference reads base**: a stat's own derivation formula referencing its own key reads the stat's base value (layering, not recursion). All other references read the referenced stat's **final** (post-bucket) value. True cycles fail closed. |
| D9 | **`effect` becomes a client-semantics doc_type** (exactly as M12c introduces `item`; zero server change), embeddable in actors and items; item-embedded effects reach the owning actor only when the effect sets `transfer: true`. |
| D10 | **Package family**: `@shadowcat/formula` (library) + `@shadowcat/module-nightfox` (headless rules) + `@shadowcat/module-nightfox-sheets` (sheets). Sheet replacement by community modules happens via `shadowcat.sheet:<doc_type>` priority — no module load-order mechanism. |
| D11 | **Stats and modifiers are maps, not arrays** (`Record<key, …>` + per-stat `order` field for display), so adds/removes/edits are single-key field-Updates — the M10b faction-registry precedent (`set_pointer` cannot grow arrays). Modifiers need no order field at all (D3 makes order meaningless). |
| D12 | Display **order is presentation-only**: reordering stats (sheet drag/drop) can never change any evaluated value. Enforced by design (dependency-ordered evaluation) and by a permutation property test. |
| D13 | **`system.stats` is the engine-reserved variables directory** (user decision 2026-07-15): the universal, system-agnostic *location* where the active game system keeps its dereferenceable variables — the contract is location-only; entry SHAPE is system-defined and validated by the system's registered schema (D6/M13f). Variables and model data are never mixed (the Foundry dual-meaning `system` problem, avoided one level down too). The formula library stays path-agnostic (resolver-injected); the convention binds the *system's* resolver. |
| D14 | **One system per world; `system.mechanics` is the system's non-variable directory** (user decision 2026-07-15). The system role is singleton — a new convention this milestone establishes (manifest `system` flag + loader enforcement deferred, §12) — so system data is NOT module-branded: no `system.nightfox`. Non-variable model data (Nightfox: `version`, `modifiers`, `active`, `transfer`) lives in the sibling reserved `system.mechanics` directory: location universal, shape system-defined + schema-validated (M13f registers both directories). Per-doc `version` stays inside it because documents travel across worlds and system versions (compendium copies must self-describe). |
| D15 | **Three-category document shape** (user decision 2026-07-15, checkpoint **M13-0**): the document separates by schema ownership into **envelope** (engine-structural: identity/authz/provenance/containment), **`engine`** (engine game data — the engine-known + server-enforced band today squatting the system-body root: position, size/shape, visionModes, conditions, grid, bounds, wall flags, …; Rust-typed per doc_type, ts-rs-generated, ending the hand-mirrored client Zod / server resolver drift risk), and **`system`** (exclusively the game system's directory: `stats` + `mechanics`, engine-opaque, schema-validated as data). Kills the root-collision hazard structurally; server-gated fields gain one authz prefix. Pre-v1 hard cutover — NO migration code (no shipped worlds exist; the M2 no-migrations-in-v1 stance holds). M13-0 gets its own spec cycle after M12 completes; M13a/b/d do not depend on it, M13c does. |

## 3. `@shadowcat/formula` (M13a)

New framework-neutral TS package (sibling of `client/core`; no Svelte, no document/store
dependency in its closure). Pnpm-workspace member.

### 3.1 Grammar (v1 — deliberately minimal; every addition is forever)

- **Literals**: integers and decimals.
- **Identifiers**: opaque to the library — any `[a-z][a-z0-9_]*` word (matched
  case-insensitively) is lexed as an identifier and handed to the injected resolver; the library
  attaches no meaning to any name. The *Nightfox* reserved-key list (`parent`, `base`,
  `current`, `min`, `max`, `floor`, `ceil`, `round`, plus any key that collides with a
  dice-notation atom — `d20`, `kh`, …) is enforced by tier-1 authoring validation in
  `module-nightfox`, NOT by the library (exact mechanical check defined in the M13b plan).
  Other systems may reserve a different vocabulary — or replace the parser outright (D1/D2 are
  Nightfox conventions; the library is the only shared piece).
- **Dotted references**: `parent.<key>` (see §5.3 for scope rules), `base.<key>` (the referenced
  stat's base value; for a resource this is `maxBase`), resource sub-refs `<key>.current` /
  `<key>.max` (a bare resource reference resolves to `current`, documented).
- **Operators**: `+ - * / %`, unary minus, parentheses. `/` is float division; `%` is the
  truncated remainder (JS semantics), documented. No implicit rounding anywhere.
- **Functions**: `min(…)`/`max(…)` (n-ary, ≥1 arg), `floor(x)`/`ceil(x)`/`round(x)` — explicit
  rounding is required for "half str, rounded down" games; without it, integer-y systems are
  unauthorable.
- **Booleans**: flag stats coerce to `1`/`0`. No boolean literals, no comparison operators, no
  conditionals in v1 (see §12 — in a VTT the effect's `active` toggle is the condition
  evaluator; the human is the trigger engine).

### 3.2 Evaluator contract

- Pure: parse → AST → evaluate with an **injected identifier resolver**. The library knows
  nothing about documents, layers, or Nightfox; consumers define what a name means.
- **Fail-closed error values**: parse failure, unknown identifier, text-typed reference,
  division/mod by zero, cap breach, or a reference to an errored value all yield a marked error
  value (never 0, never a throw, never a hang) that propagates through consuming expressions.
- **DoS bounds**: formula length cap, AST node cap, resolution-depth cap (the M11 dice-caps
  discipline applied client-side; exact numbers in the M13a plan).
- **Cycle guard**: evaluation is driven by a dependency graph with cycle detection; every node
  in a cycle resolves to an error value.

### 3.3 Notation-template mode (for rolls)

Tokenizes a string in the **M11 dice-notation superset grammar extended with stat
identifiers**; notation atoms take precedence over identifiers; each identifier is substituted
with its resolved value as a labeled constant. `d20+dex` → `1d20+3[dex]`. Output is pure M11
notation the server already parses, caps, and executes — the client never rolls.

## 4. Nightfox data model (M13b; client-semantics only, zero server change)

Nightfox data lives in the **two reserved directories of the system band** (D13/D14). Under
the D15 three-category shape (M13-0), `system` belongs to the game system outright — the
engine-known fields (shape/size/conditions/visionModes/grid/bounds/…) move to the document's
`engine` block, so nothing else ever shares `system`'s root:

- **`system.stats`** — the reserved variables directory (universal location; Nightfox defines
  the entry shape below). Formula references (`dex`, `hp.max`, `parent.str`) dereference into
  this directory via Nightfox's resolver. Actors, items, AND effects all carry one.
- **`system.mechanics`** — the reserved non-variable directory for the world's (singleton)
  system. A replacement system defines its own shape at the same location; both directories
  are validated by the active system's registered schemas (M13f).

```
system.stats     = Record<key, Stat>          // the variables directory (D13)
system.mechanics = {                          // the system model directory (D14)
  version: 1,
  modifiers?: Record<id, Modifier>,       // items + effects only
  active?:    boolean,                    // items + effects; default true
  transfer?:  boolean,                    // effects only; default false
}

Stat  = { order: number, label?: string, type: "number"|"resource"|"text"|"boolean", … }
        number:   { base: number, formula?: string, roll?: string }
        resource: { current: number, maxBase: number, maxFormula?: string,
                    clampToMax?: boolean /* default true */, roll?: string }
        text:     { value: string }
        boolean:  { value: boolean }

Modifier = { stat: string,               // target stat key on the target document (§5.3)
             op: "add" | "mulAdditive" | "mulCompound",
             value: string | number }    // magnitude: literal or formula (D4)
```

- `key` is the formula shorthand term (map key, unique by construction); `label` is the
  optional display name — sheets show `label ?? key`; the key remains the term.
- `order` is display-only (D12); drag/drop rewrites `order` values via small single-key
  updates.
- Resource semantics: formulas/modifiers target `max`; `current` is the hand-edited play value.
  With `clampToMax` (default), the resolver reports `effectiveCurrent = min(current, finalMax)`
  at **read time** — the stored value is never auto-rewritten (no write amplification, no merge
  noise; max recovering restores current). No low clamp (negative pools are game-legal).
- `active: false` on an item suppresses the item's modifiers AND everything its embedded
  effects contribute (an effect applies iff `effect.active && carrier.active`).

## 5. Evaluation model (M13b)

### 5.1 One dependency graph

Not three passes — one graph. Per stat: `base` → `derived` (formula, else base) → `final`
(derived pushed through the D3 bucket pipeline). Edges come from formula references (derived
formulas and modifier magnitudes alike). Topo-sort once, evaluate once; any cycle anywhere
(through any mix of formulas and magnitudes) marks every participating stat as errored,
identically.

### 5.2 Reference semantics (the one rule + two reads-down exceptions)

- A reference resolves to the referenced stat's **final** value — so `attack = dex + str`
  reflects a str-boosting belt, and a magnitude of `parent.str / 2` scales with buffed str.
- Exception 1: a stat's own formula referencing its own key reads its **base** (D8).
- Exception 2: `base.<key>` always reads base, from anywhere.
- A magnitude referencing its own **target's** final value is a cycle → error (the bucket
  already gives you additive/multiplicative access to the target; self-scaling composes via
  `mulAdditive`).

### 5.3 Scope rules

- **Bare identifiers** resolve against the document that owns the formula (an item's derived
  formula reads the item's stats; an effect's magnitude can read the effect's own stats).
- **`parent.*` in derived-stat formulas** resolves against the embed parent (item-in-actor →
  the actor; effect-in-item → the item).
- **`parent.*` in modifier magnitudes** resolves against the modifier's **target document** —
  for a transferring effect that is the actor, not the carrying item. ("Parent" = the thing
  being modified.)
- **Modifier targets**: an item's modifiers target the owning actor. An effect's modifiers
  target its host (actor-embedded → the actor; item-embedded → the item; item-embedded with
  `transfer: true` → the item's owning actor).
- Standalone (unembedded) items/effects: modifiers are naturally inert; `parent.*` is an error
  value in previews, rendered as a warning, never a crash.

### 5.4 Tolerance rules

- A modifier whose target stat does not exist on the target document — or whose target is a
  `text`/`boolean` stat (not modifiable per D7) — is **inert + surfaced as a sheet warning**
  (community content mixing must not error the whole actor). A modifier targeting a resource
  applies to its `max` (D7).
- A formula referencing a missing stat is a **hard error value** (fail-closed; visible chip).
- Malformed `system.stats` / `system.mechanics` data: readers fail closed around it (existing
  project convention), sheets show the raw tree via the generic fallback affordances.

### 5.5 Where it runs

The resolver is a pure function over a document + its embedded children (agnostic of whether
the actor is world-level or a token's embedded copy — the `resolveTokenActor`/`EffectiveActor`
read-through supplies the doc). Client-only: the server never evaluates Nightfox formulas
(ARCHITECTURE §2 invariant 6); server-side rule enforcement remains the Phase-3
sandboxed-validator roadmap item, untouched by this milestone.

## 6. Sheets & UX (M13c — gates on M12c)

`@shadowcat/module-nightfox-sheets` registers actor/item/effect sheets into
`shadowcat.sheet:<doc_type>` at a priority above the generic sheets; a community sheet outbids
it with a higher priority (D10). A world with the rules package but a community sheet package
works unchanged.

- **Actor sheet**: stat table — per-type controls, touch-friendly drag/drop reorder
  (presentation-only per D12; 44px handles, cross-platform directive), formula inputs with live
  validation + computed-value preview, error/warning chips (cycle, missing ref, inert
  modifier); per-stat roll buttons; inventory list (active toggles, `openDocument` into item
  sheets); effects list (active + transfer toggles). Template actions (create-from-template,
  pull/push/revert buttons) are added to these sheets by **M13e**, not built here.
- **Item / effect sheets**: own stat block + modifiers editor (target key, op picker, magnitude
  field accepting literal or formula; validated against the owner when embedded, warned when
  dangling); active/transfer toggles.
- **Hard rules inherited from the M12 spec**: sheets read the optimistic store; edits are
  field-path Updates with **real OCC pre-images** (raw stored `old`, never `old: null`);
  editability advisory via `AppContext.canEdit`; server stays authoritative. Write-site
  resolution (linked actor vs token-embedded copy) via the `openDocument`/`conditionTarget`
  precedent.
- **Tier-1 validation**: sheets validate every write against the module-nightfox Zod schemas
  before dispatch; invalid input never leaves the sheet.
- Sheet chrome is i18n-keyed; user-authored stat labels/keys are data, not localized.

## 7. Rolls to chat (M13d)

- Optional per-stat `roll` template in the extended notation grammar (§3.3); default roll =
  the stat's flat value as a labeled constant.
- Roll button → notation-template resolution → pure M11 notation → posted as `/roll` through
  the existing composer/chat seam (`AppContext.chat`), with speak-as attribution riding the
  M11d-2 attribution authz unchanged.
- The message content carries the unresolved template (`d20+dex`) for transparency; the
  `RollEmbed` shows the resolved notation and outcome.
- A roll referencing any errored stat is blocked at the button (visible reason), never sent.
- **Zero new wire frames; zero server change.** The server's existing caps/entropy/validation
  on `/roll` ingest are the security boundary.

## 8. Templates + merge engine (M13e — engine-level; own sub-spec)

Realizes the deferred provenance-based merge model ([[document-inheritance-merge-model]]) as
generic engine machinery (client core + whatever minimal server assist its sub-spec justifies);
Nightfox is its first consumer, compendium pull/push rides it later.

- **Stamp on create**: creating from a template deep-clones the document
  ([[embedded-copy-needs-deep-clone]]) and records envelope `source {id, version}` provenance.
- **On-command operations** (never automatic): **pull** — child re-bases onto the parent's
  current state, preserving local diffs via 3-way merge; **push** — parent applies its changes
  to all children, preserving each child's diffs; **revert** — child discards local diffs back
  to the parent's current state.
- **Base retrieval — design lean, to be confirmed in the sub-spec**: store the base snapshot on
  the child at stamp/last-sync time. Self-contained, survives parent deletion, requires no
  server version-history machinery (the event ring buffer is time-bounded and non-durable);
  cost is document size, policed by the existing size caps.
- Sub-spec also resolves: conflict UX v1 (field-level), authorization (pull = child owner;
  push = parent owner/GM), interaction with the M5 field-level merge machinery, and how
  `system.nightfox` map-shaped data merges (maps chosen partly for mergeability, D11).

## 9. Server declarative schema registry (M13f — tier-2 validation; own sub-spec)

Modules declare a schema as **data** (JSON-Schema subset), scoped to a `(doc_type, system-body
subtree)` pair — subtree scoping keeps schemas composable across modules (Nightfox registers
schemas for `system.stats` AND `system.mechanics` — the D13/D14 "validated against a
system-defined data model" contract — never the whole body). The server enforces registered schemas on write —
structural enforcement only, invariant 6 intact (M6b declarative-capability precedent).
Unregistered doc_types/subtrees stay unrestricted; violation rejects the write shaped like a
capability denial. Enforce-on-write means pre-existing invalid docs linger until next touched —
consistent with the migrate-at-the-boundary philosophy, stated explicitly.

Sub-spec resolves: schema wire format + versioning, declaration channel (module manifest vs
world config), schema-upgrade behavior on a live world, composition with size caps and
`deny_unknown_fields`. Nightfox ships schemas for its actor/item/effect subtrees — the first
server-guarded system data.

## 10. Security & permissions

- Stat blocks are ordinary system-body data: the existing per-recipient permission model
  (property overrides, `OwnerOrGm`, redaction-before-transmission) applies unchanged. No new
  egress paths.
- Formula evaluation is client-side over data the recipient was already sent — it can neither
  widen visibility nor leak (nothing hidden reaches the evaluator).
- Untrusted notation still executes only server-side under the M11d-2 caps; Nightfox only
  *composes* notation strings client-side.
- The formula evaluator itself is a DoS surface on the *local* client only (a hostile doc can
  at worst error its own sheet); caps + fail-closed error values bound it (§3.2).
- M13f is the milestone's only new server enforcement surface and gets its own security-lens
  review (buddy-check pre-authorization recommended at plan level, like every prior
  wire/enforcement checkpoint).

## 11. Testing strategy

- **M13a**: exhaustive unit + property/fuzz — parser round-trips, cap enforcement, cycle
  detection (incl. magnitude→target cycles), error-value propagation, no-throw/no-hang under
  fuzz; notation-template output validated against a golden corpus of M11-legal notation.
- **M13b**: the **permutation invariance property test** — shuffling stat map iteration order,
  modifier ids, and embed order never changes any resolved value (D3/D12, the load-bearing
  invariant); transfer/active gating matrix; tolerance rules (§5.4); resource clamp semantics.
- **M13c**: component tests + Playwright e2e — author stats → derived value visible → equip
  item → value changes → toggle effect → value reverts; drag/drop reorder changes nothing but
  order; OCC pre-image correctness (raw stored `old`).
- **M13d**: e2e roll flow — roll button → chat card shows template + resolved RollEmbed;
  errored-stat roll blocked; notation substitution differentially checked against server
  acceptance (a rejected `/roll` is a test failure).
- **M13e**: merge property tests — pull/push/revert idempotence, diff preservation, stamp
  deep-clone independence; conflict-path coverage per sub-spec.
- **M13f**: server integration tests — accept/reject matrices per schema, subtree scoping,
  unregistered-doc_type passthrough, upgrade behavior per sub-spec.

## 12. Rejected alternatives & deferred candidates

**Rejected (with reasons):**
- *Live inheritance / read-time template merge* — user decision: reuse the provenance 3-way
  pull/push model instead (D5); one inheritance mechanism, not two.
- *Swappable per-actor base-stat tables* (from the video-game reference system) — a live-
  reference mechanism that would coexist badly with D5; archetype swap = re-parent + pull.
- *Free-formula modifiers in embed order* (the pre-revision design) — order-dependent results
  from incidental embed order ("reordering inventory changes your stats"); replaced by D3
  buckets, which also merge far better than formula strings.
- *`resource` as two loose number stats* — the current/max pairing is what UI, clamping, and
  modifier targeting hang off; as a naming convention (`_max` suffix) it's fragile.
- *Module load/priority-order world settings* — per-contribution priority (sheet registry) is
  the targeted mechanism; a global order is a blunt instrument nothing currently needs.
- *A single namespace holding both variables and mechanics* (`system.nightfox.stats` — the
  pre-D13 shape, or Foundry's bare `system`) — mixes dereferenceable variables with model
  data and couples every stat consumer to the active system's identity; D13's two-location
  split is the durable shape.
- *A universal stat ENTRY shape* — the D13 contract is deliberately location-only; entry shape
  stays system-defined (an earlier user directive). An optional progressive contract (e.g.
  engine token bars reading any `{current, max}`-shaped entry) remains open behind it.

**Deferred candidates (logged, not built):**
- Manifest `system: true` flag + loader enforcement of the one-system-per-world convention
  (D14 premise; convention documented now, enforced when a second system exists to conflict).
- `override` bucket (fourth pipeline stage; admits cleanly later without breaking D3).
- Comparison operators / `if()` in the grammar; boolean literals. The `active` toggle is v1's
  conditional (human-in-the-loop, tabletop-appropriate).
- `default(ref, value)` fallback function (silent fallbacks hide authoring bugs; revisit on
  evidence).
- GM-facing "preferred sheet per doc_type" world setting (only if priority-racing proves too
  blunt in practice).
- Effect durations/triggers — Phase-2 combat tracker territory; v1 effects are
  on-while-present-and-active.

## 13. Checkpoint decomposition

| Checkpoint | Contents | Depends on |
|---|---|---|
| **M13-0** | Three-category document shape (D15): `engine` block relocation, typed + ts-rs-generated, hard cutover, no migration — own spec cycle | M12 complete (avoids the running M12 session; M12c sheets read these fields) |
| **M13a** | `@shadowcat/formula`: grammar, evaluator, caps, cycle guard, notation-template mode | nothing (startable pre-M12-completion) |
| **M13b** | `@shadowcat/module-nightfox`: schema + Zod (tier-1), dependency-graph resolver, buckets, items/effects semantics (`active`/`transfer`), `effect` doc_type | M13a |
| **M13c** | `@shadowcat/module-nightfox-sheets`: actor/item/effect sheets, stat editors, drag/drop order, warning/error chips | M13b, **M12c**, **M13-0** |
| **M13d** | Roll wire: per-stat roll templates → `/roll` posts, blocked-on-error, attribution | M13b, M11 (done) |
| **M13e** | Templates + 3-way merge engine (pull/push/revert) — own sub-spec first | M13b (consumer exists) |
| **M13f** | Declarative server schema registry + Nightfox schemas — own sub-spec first | M13b (schemas exist to declare) |

Each checkpoint: plan → execute per project conventions (per-task review gates; buddy-check
pre-authorization recommended for the M13a evaluator core, the M13e merge engine, and the M13f
enforcement path). Reviewed skill-update gate applies at each checkpoint's completion; a new
`shadowcat-codebase-nightfox` (or `-formula`) skill is created when the first checkpoint opens
the subsystem.
