# M14c-4 · Dice References + Chat Channel — Design

**Status:** Drafted (2026-08-31, Kimi session taking over the M14c campaign). Fourth of six M14c
sub-projects per the umbrella decomposition
([M14c-1 design](2026-08-30-m14c-1-server-formula-engine-design.md) §1). Consumes M14c-1's server
formula engine and M14c-2's `combat::eval`. Amends the
[M14 design](2026-08-28-m14-combat-tracker-design.md) D13 ("Client composes notation exactly as the
M13d roll wire does") and the [M14b design](2026-08-28-m14b-combat-clock-design.md) `CombatRoll`
row — where they disagree with this spec, this spec wins; each carries a pointer.

## 0. The gap this spec closes

The M14c-1 audit (Appendix A, group 4) found dice-notation reference resolution living entirely on
the client: `@shadowcat/formula`'s `resolveNotationTemplate` rewrites a template like
`1d20 + str` into pre-substituted literals (`1d20 + 3[str]`) before anything reaches the server,
and the wire (`SendMessage.content`, `CombatRollEntry.notation`) carries only the substituted text.
Under the corrected invariant 6 — by default computation runs on the server; the client requests —
that is the same misreading M14c-1/-2/-3 corrected elsewhere, with one extra consequence the other
groups did not have: the server cannot resolve a reference over data the rolling client is not
allowed to see (a hidden NPC's initiative modifier), so client-side resolution is not merely
misplaced but *insufficient*. Same audit row: `MessageEngine.channel` is stored verbatim and never
validated, yet `chat::settings::resolve_dice_context` reads it to select a per-channel
`ParseContext` — an unvalidated input silently steering dice resolution.

The audit premise needs one correction the reconnaissance for this spec surfaced: **no in-repo
client code calls `resolveNotationTemplate` today.** The composer sends the author's text verbatim
and the server executes it; the template machinery ships as library surface for external system
modules (it is on the shell's import map) with no in-repo caller. So there is no client
substitution to *remove* — the work is to give the server the capability the wire protocol was
always going to need once a system module composes statted rolls, before M14d's tracker UI becomes
the first in-repo producer of notation with references.

## 1. Decisions

| # | Decision |
|---|---|
| R1 | **The server gains `formula::template`, a behavioural twin of the TS `template` module's rewrite half.** Same recognizer chain, same keyword reservation, same `1d` count synthesis, same labeled substitution (`3[str]`, `-2[str]` for negatives), same integer-only and i32-magnitude rules, same error kinds and `detail` wording, same UTF-16 position counting. Pinned by the shared conformance corpus (R6), never by convention. `checkNotationKey` is NOT ported — it is an authoring aid with no server consumer; the server answers "does this reference run" by running it. |
| R2 | **Resolution is a pre-parse substitution at the transport boundary, inside the one execution path.** `chat::rolls::execute_roll`/`execute_roll_with_seed`/`validate_formula` gain a host parameter; the template is resolved to labeled-literal notation FIRST and the existing `notation::parse` + caps run on the result. The dice grammar itself is untouched — a substituted reference is exactly the labeled-constant shape `1d20 + 3[str]` the parser already accepts, and `RollOutcome.labeled_consts` keeps the reference as display provenance. The stored `RollEmbed.formula` keeps the author's template text verbatim; `spec`/`raw` describe the substituted roll, so a later GM recalculation re-derives from the stored naturals and never re-resolves (a stat change after the roll must not rewrite history). |
| R3 | **The binding a roll resolves against already exists on every roll frame — this sub-project gives it resolution semantics, not new wire.** `SendMessage.actor_owner` (already ownership-validated at ingest) binds chat rolls: `Actor` → that document; `TokenInstance` → the token's embedded actor copy, else its linked actor — the SAME host precedence `combat::eval::formula_host` declares, shared by promoting the embedded-copy extraction, never re-derived. `CombatRoll` binds per-entry through `combatant_id` → `formula_host`. No binding (plain `/roll` with no speak-as) ⇒ every reference is `unknown-ref` and the send fails with the usual System notice — fail-closed, and the author is told which reference was needed. No wire-shape change: `ClientMsg` is untouched. |
| R4 | **No new authorization gate.** The binding was already ownership-validated (chat: owner-or-GM at ingest; combat: `owns_combatant`/`authorize`), and the resolved values are numbers the roller is entitled to read through that relationship. Recipients see substituted values as labeled breakdown chips (`7[dex]`) — that is roll provenance, the same visibility the `outcome` already has; the genuinely secret case is already covered, because a hidden combatant's roll posts GM-only (M14 D13). |
| R5 | **Roll buttons are templates by nature: stored raw, syntax-checked at ingest, resolved per click.** `[[roll:...]]` ingest validation substitutes a placeholder zero for every identifier (`0[path]`) — faithful because a substituted reference is always a labeled const factor, never a dice count or sides — then runs the existing `validate_pre_roll` on the result. Value-dependent failures (an unknown ref for THIS clicker) are per-clicker and surface at click time as the standard whispered System notice. The click resolves against the CLICKER's current speak-as: the composer's sticky speak-as selection lifts from `Composer`-local state into a ui-kit session-level selection (a sibling of the existing one-shot `SpeakAsToken`), and `MessageCard`'s button click passes it. |
| R6 | **The conformance corpus grows a template section.** `src/client/formula/src/__fixtures__/conformance.json` gains a `templates` array: `{ src, bindings, expect }` cases where `bindings` maps dotted paths to numbers and `expect` is the rewritten notation string or `{ error, detail }`. `conformance.test.ts` runs them through `resolveNotationTemplate`; `formula::tests::conformance` runs them through the Rust twin; both resolve against a sorted-map stub whose miss wording is the corpus's. Number rendering inside `detail` strings is pinned only for values whose shortest-round-trip rendering agrees in JS and Rust (the corpus never uses a value where they differ — e.g. nothing past JS's `1e21` exponent threshold). |
| R7 | **`NOTATION_KEYWORDS` becomes a three-declaration parity.** The Rust twin declares the keyword list it cannot import from `dice::notation::parser`'s match arms; `scripts/check-notation-modifier-parity.mjs` is extended to extract the Rust template declaration and require all three sources to agree (TS list = Rust template list = `P::modifiers` arms + `d`). |
| R8 | **`MessageEngine.channel` is validated at ingest against the world's `channel-registry`.** `handle_send_message` and the `CombatRoll` arm both refuse a channel that is not a registry key — the check runs AFTER the flood-budget check (the cheap guard stays ahead of the DB read). A new validation-class `SendMessageError::UnknownChannel` is player-presentable (`unknown channel` — the sender supplied the string, so refusing it discloses nothing); an absent/unreadable registry fails closed as `Data` (the world is seeded at creation and reseeded on join since M14c-3, so absence means corruption, not a state to accommodate). `CombatRoll` gets the same refusal mapped through `CombatError`. Server-authored notices (roll-error notices, combat eval notices) are internal posts that never cross this gate. `ChannelRegistryEngine` gains `validate` (non-empty channel map — an empty registry would wedge all chat; non-empty channel names; keys within `MAX_CHANNEL_CHARS`), wired into the `channel-registry` arm of `normalize_engine` the same way `world-settings` was in M14c-3. |
| R9 | **The TS `resolveNotationTemplate`/`checkNotationKey` stay client-side, re-scoped to preview/authoring aids.** They remain on the import map for external system modules (a sheet previewing "what will this roll" against locally-visible data). The contract change is documented: the wire now carries RAW templates and the server resolves authoritatively at ingest; pre-substituted text still rolls correctly (a `3[str]` literal is already valid notation), so nothing breaks, but substitution is no longer how a roll is *sent*. No in-repo composer preview is built — the composer deliberately never parses commands client-side, and detecting a roll to preview would cross that line. |

## 2. `formula::template` — the server twin

New unit `src/server/src/formula/template.rs` (+ sibling `template/tests.rs`), declared in
`formula/mod.rs` beside `lexer`/`parser`/`evaluate`/`graph`/`resolver`, under the same
`#![deny(missing_docs)]` pair. Section-by-section twin of `src/client/formula/src/template.ts`:

- **Character predicates** — the lexer's private `is_digit`/`is_word_start`/`is_word_char`
  (`formula/lexer.rs`) promote to a `pub(crate)` `chars` submodule shared by lexer and template,
  mirroring the TS `chars` module's one-declaration rule.
- **Source model** — the TS scan indexes a UTF-16 string (`src[i]`, `slice`, `indexOf`); the Rust
  twin scans `Vec<char>` (or `char_indices`) while tracking the UTF-16 offset of each position, so
  the one position-bearing error (`unterminated '[' label at position N`) reports the same N the
  TS side would. The M14c-1 lexer solved the identical problem; same approach.
- **`NOTATION_KEYWORDS`** — the same 15-entry list (`d` first, declared once via the same
  `DICE_OPERATOR` idiom) with the same "this list is not the set of unsafe stat keys" doc warning.
- **The recognizer chain** — `Claim { kind, text }`, `NotationClaimKind`
  (`Label`/`Integer`/`Keyword`/`Identifier`/`Literal`, serde kebab-case iff ever serialized — it is
  not exported), the four recognizers in the same order with the same observability rules
  (`claimLabelSpan`'s rejecting branch, `claimNotationKeyword` ahead of `claimIdentifierSpan` being
  the one observable adjacency), `claim_at` total via the literal fallback.
- **`emit_claim` + `substitute_identifier`** — the `1d` synthesis when the dice operator follows a
  non-integer claim; the resolver boundary is the typed `Resolve` trait, so the TS-only
  `"resolver-error"` and thrown-callback arms have no twin (same carve-out M14c-1 made for
  `evaluate`); the integer-only rule (`type` error, `'hp.max' = 3.5: roll templates require
  integers (use floor/round in the stat formula)`), the asymmetric i32-magnitude cap (`cap` error,
  `'x' = 3000000000: out of i32 range`), and the negative-emission form `-2[str]`.
- **`resolve_notation_template(src: &str, resolve: &dyn Resolve) -> Result<String, FormulaError>`**
  — the `MAX_FORMULA_LENGTH` cap counted in UTF-16 code units; the scan; the emission. Returns the
  notation string rather than the TS `{ notation }` wrapper (idiomatic Rust; the corpus pins
  behaviour, not the wrapper).
- **Post-substitution size needs no new cap.** Substitution emits only flat atoms
  (`-N[path]`), never parens or dice groups, so parse depth cannot grow and parser cost stays
  linear; the worst-case expansion (~15×512 ≈ 7.7k chars) is bounded by construction and the
  record/dice caps apply downstream as today.
- **Never panics** — the crate rule; `formula::proptests` gains a template arm feeding random
  sources and random resolver outputs, asserting no panic and no non-finite-derived text.

## 3. The resolver plumbing

- `combat::eval`'s `NoHostResolver` moves to `formula::resolver` (`pub(crate)`), wording still
  shared through `resolver::unknown`; `combat::eval` re-imports it. One no-host rule for every
  consumer.
- `embedded_actor_copy` (currently private in `combat::eval`, shared by `formula_host` and
  `effect_host_doc`) promotes to `crate::data::document` as `pub(crate)
  embedded_actor_copy(token: &Document) -> Option<&Document>` — the token→embedded-copy step then
  has ONE definition across combat evaluation, effect discovery, and chat's `TokenInstance`
  binding.
- `chat` gains `host_for_actor_owner(repo, world_id, &ActorOwnerRef) ->
  Result<Option<Document>, DataError>`: `Actor` → the actor doc; `TokenInstance` → the token doc's
  embedded copy, else the linked actor doc. Returns the DOCUMENT the resolver reads, mirroring
  `formula_host`'s rule that the token case resolves to the embedded child doc itself. The ingest
  gate in `handle_send_message` already fetches the bound document for validation — it keeps that
  fetch and only the `TokenInstance`-without-embedded-copy case costs a second query (the linked
  actor).

## 4. Roll-path wiring

`execute_roll(formula, ctx, host: Option<&Document>)`,
`execute_roll_with_seed(formula, ctx, host, seed)`, `validate_formula(formula, ctx, host)`:

1. `resolve_notation_template(formula, &resolver)` where `resolver` is
   `SystemLeafResolver::new(doc)` for `Some(doc)`, else `NoHostResolver`.
2. The resolved notation goes through `notation::parse` + `validate_pre_roll` (+ the roll itself
   for the execute variants) exactly as today.
3. The returned/displayed formula string remains the author's ORIGINAL template (the embed shows
   what was asked; the breakdown chips show what each reference read).

`RollError` gains `Reference(FormulaError)` (name settled at implementation) with a clean
player-presentable `Display` arm rendering the formula error's `detail` (`unknown reference
'stats.str'` tells the roller exactly what failed); the variant joins the `no-debug-artifacts`
test's iteration. Resolution failure ⇒ the standard refusal shape on every path: the whispered
`MessageKind::System` notice instead of the message in chat (flood budget stays 1:1), and
`CombatError::Roll`'s existing `#[from]` for `CombatRoll` (all-or-nothing per intent, unchanged).

Call-site deltas:

- **`handle_send_message`** — `/roll` and inline `[[...]]` chunks resolve against
  `host_for_actor_owner(actor_owner)` (computed at most once, lazily, beside the existing lazy
  `dice_ctx`); button chunks call `validate_formula(formula, ctx, PLACEHOLDER)` where the
  placeholder path is a zero-resolver over the template scan (R5), so ingest catches structural
  breakage without binding the stored button to the author's stats.
- **`combat::build_ops`'s `CombatRoll` arm** — per entry, `host =
  combat::eval::formula_host(&snap.hosts, combatant_kind)` and `execute_roll(&entry.notation,
  dice_ctx, host)`. `CombatRollEntry.notation`'s doc comment is rewritten: raw template text,
  references resolved server-side against the combatant's formula host.
- **Recalc (`handle_recalc_roll`)** — untouched: it re-derives from the stored `spec`/`raw`, which
  already encode the substituted constants.

## 5. Client changes

- **`@shadowcat/formula`** — doc-only: `resolveNotationTemplate`'s contract re-scoped to
  preview/authoring (R9); the module docs name the server as the authoritative resolver.
- **Speak-as lifts to session state (ui-kit).** The composer's sticky actor `<select>` becomes a
  ui-kit-held selection (stable-instance/mutate-in-place class, the `SpeakAsToken` sibling shape):
  the composer reads/writes it, and `module-chat-card`'s `sendRollButton` sends
  `actorOwner` from it (the one-shot `SpeakAsToken` still takes precedence, same rule the composer
  already applies). This is what makes a statted button useful to anyone but its author.
- **`module-chat`** — the GM pseudo-channel's `postTarget` no longer hardcodes `"general"`: it
  targets the lowest-sorted registry channel id (the registry is guaranteed non-empty by R8's
  validate), falling back to `"general"` only while the registry doc itself is absent (pre-seed
  worlds mid-join). The GM channel editor refuses to remove the LAST channel client-side (the
  server would reject the resulting empty registry anyway; the client error is the kinder one).
- **No wire or Zod-mirror change** — `ClientMsg` is untouched (R3); the wire drift-guard suites
  stay green untouched. No ts-rs regeneration is required.

## 6. Channel validation

- New `pub(crate) async fn validate_channel(repo, world_id, channel) -> Result<(),
  SendMessageError>` in `chat` (home of every other send-time rule): loads the world's
  `channel-registry` singleton; `Ok` iff `channel` is a key of `channels`. Absent doc, query
  error, or a body failing decode ⇒ `SendMessageError::Data` (fail-closed, generic wording).
  Unknown key ⇒ `SendMessageError::UnknownChannel` — a validation-class variant, so its `Display`
  is a specific player-presentable reason under the existing `[sec]` classification.
- Called from `handle_send_message` AFTER `rate.check` and BEFORE the attribution gate (a cheaper
  rejection than the ownership queries), and from the `CombatRoll` arm of `combat::build_ops`
  before any roll executes (mapped to a new `CombatError` arm with its own safe wording — same
  precedent as `DuplicateRoll`: the caller supplied the channel).
- `resolve_dice_context` itself is unchanged: after R8 every ingested channel is registered, so
  its `channel_overrides` lookup is always meaningful; its fail-closed default for a malformed
  dice-settings doc stays as the last line.
- `ChannelRegistryEngine::validate` (non-empty map, non-empty names, key length ≤
  `MAX_CHANNEL_CHARS`) wired into `normalize_engine`'s `channel-registry` arm — the
  `world-settings`/`scene` pattern from M14c-3, so the registry can never be written into the
  wedged state this gate would refuse everything over.

## 7. Docs, skills, gates

- **Amendment pointers** ("*Amended (M14c-4)*" + one-line supersession): M14 design D13; M14b
  design's `CombatRoll` row; the M13d roll-wire plan's composition note if it names client
  substitution.
- **`docs/site/guides/creating-a-system.md`** — the dice-and-chat section gains the raw-template
  rule (send `1d20 + attributes.str`, the server resolves against your speak-as) and the
  `checkNotationKey` section notes the server runs the same grammar at ingest.
- **`docs/site/protocol.md`** — the `SendMessage`/`CombatRoll` frame notes gain the binding and
  channel-validation semantics.
- **PLAN.md** marks M14c-4 done on merge; **HISTORY.md** gains the delivery entry;
  **POST_WORK_FINDINGS.md** — the `pnpm docs:api:ts` finding ("M14 owns the types") is marked
  Resolved: verified passing on post-M14c-2 main during this sub-project's baseline.
- **Skills** (plugin checkout, reviewed skill-update gate + `check-skill-symbol-refs-cli`):
  `shadowcat-codebase-dice` (references in notation; the corpus's template section; the parity
  gate's third declaration), `shadowcat-codebase-formula` (the `template` twin; preview-only
  rescope), `shadowcat-codebase-chat` (channel validation, the binding's resolution semantics,
  `RollError::Reference`), `shadowcat-codebase-combat` (`CombatRoll` per-entry resolution),
  `shadowcat-codebase-client-shell` (speak-as session state) — each a small amendment, not a
  rewrite.
- **Gates:** `cargo test`/`clippy`/`fmt`, `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint`,
  `pnpm lint:docs`, `pnpm lint:props`, `pnpm lint:comments`, `pnpm lint:file-size`,
  `pnpm lint:inline-tests`, `pnpm docs:check-examples`, `node
  scripts/check-skill-symbol-refs-cli.mjs`, `pnpm run test:scripts` (the extended parity gate
  included). SQL schema unchanged.

## 8. Non-goals

- No attack/damage automation (system-owned, M14 excludes).
- No composer roll preview UI (R9 — the composer never parses client-side).
- No per-recipient redaction of labeled breakdown chips (R4).
- No initiative-formula configuration source — what notation the tracker composes is M14d's
  decision; this sub-project makes whatever it composes resolvable.
- `checkNotationKey` stays TS-only (R1).
