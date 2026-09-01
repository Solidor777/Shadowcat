# Creating a system

A game system in Shadowcat — the rules, sheets, and content model for a
particular game — is not a separate kind of artifact. It is a module that claims
game-facing surfaces. This tutorial builds a minimal d20-style system from
`examples/system-minimal/` in the Shadowcat repository (CI-built and tested, like
every sample on this page) and closes with the layering a full-scale system
grows into.

## Systems are modules

Everything in [Creating a module](/guides/creating-a-module) applies unchanged:
the same `module.json` manifest, the same Vite library build with engine packages
external, the same install/enable/dev loop. Read that guide first — this one does
not repeat it.

What makes a module a *system* is which contracts it claims:

- **Sheets** — it replaces generic document sheets (actor, item) with
  game-specific ones, by registering higher-priority providers.
- **Rules** — it computes derived values from the opaque `system` band of
  documents, typically with `@shadowcat/formula`.
- **Content** — it ships templates (statted monsters, item catalogs) that GMs
  stamp into instances; provenance-tracked pull/push keeps them in sync.

## The three-band document

The core mental model for system authors. Every Shadowcat document has three
bands:

| Band | Owner | Validation |
|---|---|---|
| `name` (envelope) | Engine | Real — universal display name, redactable |
| `engine` | Engine | **Real server-side ingress validation**, typed structs, unknown fields rejected — present only for the 23 engine-defined doc types (tokens, actors, scenes, walls, regions, lights, drawings, templates, messages, the config-docs, and the combat family: `combat`, `combatant`, `resource-registry`, `effect`, `combat-history`) |
| `system` | **You** (the game system) | **Structural only** for meaning — size, field-path shape, declared schemas. The server never decides what a `system` value *means*; it does evaluate the engine's formula grammar over numeric leaves your formulas name, so persist the derived values you want the engine to act on |

Your system owns the `system` band outright: attributes, resources, class
features — any JSON shape you like. The trade is that *your system* defines what
that shape means, so validate and fail closed in your own sheets and rules (as
`evalFormula` below does) — and persist any derived value the engine should act on
(a speed, a hit-point maximum) as a numeric leaf, because the server reads leaves,
never your sheet's computed state.

## Claiming the actor sheet

The sheet registry keys on `shadowcat.sheet:<doc_type>` contracts. The built-in
generic actor sheet registers at priority 0; a system takes over the doc type by
registering higher:

<<< @/../../examples/system-minimal/src/index.ts#manifest

<<< @/../../examples/system-minimal/src/index.ts#sheet-registration

Every sheet component receives the same three props:

```ts
let { docId, systemPrefix, close }: { docId: string; systemPrefix: string; close: () => void } = $props();
```

`systemPrefix` exists because "the actor" is not always a top-level document: a
top-level actor's system band lives at `/system`, but an instanced token carries
an embedded actor copy whose band lives at `/embedded/actor/0/system`. Your sheet
must build every read and write path from `systemPrefix` — never hardcode
`/system` — and the same sheet then works for both cases for free.

## Declaring world-setting defaults

Every world-configurable setting (scene defaults, pathfinding, animation, and the combat clock)
resolves through a fixed chain: an engine-shipped fallback, then a world's `system-defaults`
singleton, then `world-settings`, then a per-scene override — narrowest wins. Your system supplies
the second tier by declaring `systemDefaults` in its `module.json` manifest, alongside the
`shadowcat.system` contract it provides:

```json
{
  "id": "your-system",
  "version": "0.1.0",
  "engines": { "shadowcat": "^0.1.0" },
  "provides": [{ "contract": "shadowcat.system", "cardinality": "singleton" }],
  "systemDefaults": {
    "combat": { "movementResource": "movement", "interpretation": "spaces" }
  }
}
```

The SERVER reads this declaration: its installed-module scanner validates the object against the
engine's `SystemDefaultsEngine` shape (an invalid declaration is logged and ignored; the module
itself still loads), and the world-config seed path writes the world's `system-defaults`
singleton from it — at world creation, at every world join, and whenever the enabled-module set
changes. A world admits at most ONE enabled module providing `shadowcat.system` (the enable
route rejects a second), so there is never a losing claimant to resolve. Every server-side
resolver (e.g. the combat clock's `resolve_combat_rules`) reads the stored singleton ahead of
`world-settings` and behind any per-scene override. Neither you nor the GM's client ever writes
`system-defaults` — the server rejects any client write to it; declare the shape in the
manifest, and the seed path does the rest.

## Reading and writing the system band

<<< @/../../examples/system-minimal/src/CharacterSheet.svelte#sheet-read

<<< @/../../examples/system-minimal/src/CharacterSheet.svelte#sheet-write

The disciplines here are the module guide's, applied to sheet work: JSON-pointer
paths built from `systemPrefix`, the `createSubscriber` bridge on
`ctx.documents`, the raw current stored value as `setField`'s concurrency
pre-image, and `ctx.canEdit` to disable (not fake-enforce) controls.

## Rules via @shadowcat/formula

`@shadowcat/formula` is a framework-neutral expression library: lexer, parser,
evaluator, dependency-graph resolution. Formulas reference stats as bare dotted
paths (`attributes.str`), and the library **never throws and never emits
NaN/Infinity** — every failure is a typed error *value* (`parse`, `unknown-ref`,
`div-zero`, `cap`, ...), so a malformed monster stat cannot crash a sheet.

<<< @/../../examples/system-minimal/src/rules.ts#rules

The resolver callback is where your system's data model plugs in: it maps a
dotted path to a number from the `system` body, or returns the library's own
`unknown-ref` error to fail closed.

### Check your stat keys before you ship them

Roll templates are rewritten by a second grammar, which reads dice notation and
stat references out of the same text. Some keys are claimed by that grammar
before they are ever offered to your resolver, and **the failure downstream is
not reliably loud** — claimed text can be rewritten into notation with no error
on any path, so the roll runs and the number changes.

Do not try to derive which keys are safe. Ask:

```ts
import { checkNotationKey } from "@shadowcat/formula";

checkNotationKey("hp.max").intact; // true  — reaches your resolver as written
checkNotationKey("kh.max").intact; // false — "kh" is claimed as dice notation
```

`intact` is the verdict. `segments` shows what each part of the key was claimed
as, so an authoring UI can tell the author *why* a name was refused rather than
only *that* it was, and `rejects` carries the error for a key the grammar refuses
outright. What each of those outcomes does downstream is documented on
[`NotationKeyCheck`](/api/ts/interfaces/_shadowcat_formula.NotationKeyCheck.html).
The server runs the same grammar at ingest when it resolves a roll's references,
so a key that passes here behaves identically when the server reads it.

Run it wherever your system accepts a key an author can name — a sheet's stat
editor, a compendium importer, a migration. A key that fails this check is a
name to refuse at authoring time, because the failure downstream is not reliably
loud.

## Templates: shipping content

Any document can be a template — templating is provenance (`source`), not a doc
type. A GM (or your system's compendium UI) **stamps** a template into an
instance; later edits flow with field-level 3-way merge:

- **pull** — update an instance from its template (conflicts open a
  mine-vs-theirs modal),
- **push** — send template changes to every visible instance,
- **revert** — reset an instance's mergeable bands to the template.

Sheets get the controls for free from the sheet host chrome. Programmatic access
is `ctx.templates` ([`TemplatesApi`](/api/ts/interfaces/_shadowcat_ui-kit.TemplatesApi.html)):
`stampInstance`, `pull`, `push`, `revert`, `findInstances`, `syncState`.

## Dice and chat

Rolls are server-side and immutable: clients submit dice *notation* through chat
(`/roll 2d6+3`-style commands), the server evaluates with its own entropy, and
the result lands in the message stream as roll segments no client can edit.
Notation may be a **raw template with stat references** (`/roll 1d20 + attributes.str`):
the server resolves each reference at ingest against the send's actor binding —
the sender's speak-as selection (or, for `CombatRoll`, each named combatant's
formula host) — and the breakdown shows what each reference read as a labeled
chip. An unbound send (no speak-as) that names a reference fails with an
`unknown-ref` notice. `resolveNotationTemplate` is still shipped, but only as a
preview/authoring aid: the wire carries the raw template, never a
client-substituted one. A message's `channel` is validated against the world's
channel registry at ingest — post to channels the registry declares.
Systems integrate at two points: composing notation (e.g. a sheet button that
sends a roll for `attributes.str`'s modifier via
[`ChatApi`](/api/ts/interfaces/_shadowcat_ui-kit.ChatApi.html)) and rendering
custom chat content. See the [wire protocol](/protocol) page for the frame-level
picture.

## The full-scale shape

`examples/system-minimal/` fits in one package because it is small enough to. A
complete ruleset lives in its own repository, on its own release cycle, and
separates three concerns:

- **Formula layer** — `@shadowcat/formula`, shipped by the engine: the
  expression engine this guide already used. Systems consume it rather than
  writing their own.
- **Rules layer** — the system's document layer: doc-type definitions,
  dependency-resolved derived stats, effects. No UI.
- **Sheets layer** — sheet models, stat tables, modifier editors, and the
  actor/item/effect sheet components that claim the sheet contracts.

When this tutorial's single file stops being enough, that is the split to make:
sheets read values the rules layer computed, instead of computing them.

## Reference

- [`sheetContract`](/api/ts/functions/_shadowcat_core.sheetContract.html) ·
  [`SheetMeta`](/api/ts/interfaces/_shadowcat_core.SheetMeta.html)
- [`parseFormula`](/api/ts/functions/_shadowcat_formula.parseFormula.html) ·
  [`evaluate`](/api/ts/functions/_shadowcat_formula.evaluate.html) ·
  [`isFormulaError`](/api/ts/functions/_shadowcat_formula.isFormulaError.html) ·
  [`FormulaValue`](/api/ts/types/_shadowcat_formula.FormulaValue.html)
- [`checkNotationKey`](/api/ts/functions/_shadowcat_formula.checkNotationKey.html) ·
  [`NotationKeyCheck`](/api/ts/interfaces/_shadowcat_formula.NotationKeyCheck.html)
- [`TemplatesApi`](/api/ts/interfaces/_shadowcat_ui-kit.TemplatesApi.html) ·
  [`ChatApi`](/api/ts/interfaces/_shadowcat_ui-kit.ChatApi.html)
