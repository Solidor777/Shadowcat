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
| `engine` | Engine | **Real server-side ingress validation**, typed structs, unknown fields rejected — present only for the 17 engine-defined doc types (tokens, actors, scenes, walls, regions, lights, drawings, templates, messages, and the config-docs) |
| `system` | **You** (the game system) | **Structural only** — size, field-path shape. The server never semantically validates `system` content, ever, because the server runs no third-party code |

Your system owns the `system` band outright: attributes, resources, class
features — any JSON shape you like. The trade is that *your client code* is the
only semantic authority over that shape, so validate and fail closed in your own
sheets and rules (as `evalFormula` below does).

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
- [`TemplatesApi`](/api/ts/interfaces/_shadowcat_ui-kit.TemplatesApi.html) ·
  [`ChatApi`](/api/ts/interfaces/_shadowcat_ui-kit.ChatApi.html)
