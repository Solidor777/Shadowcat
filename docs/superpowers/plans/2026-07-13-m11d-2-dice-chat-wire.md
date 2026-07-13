# M11d-2 · Dice → Chat Wire Integration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development
> (Sonnet `shadowcat-coder` implementers, this session as dispatcher). Steps use checkbox
> syntax. Read the REAL files first — the tree wins over any snippet here.

**Goal:** Rolls execute server-side at chat ingest (`/roll`, inline `[[…]]`, `[[roll:…]]`
buttons) with the full wire-boundary hardening, ride the message body as `RollEmbed`/
`RollButton` segments, and render on the card; plus ambient dice-settings, visible roll
errors as System notices, roll immutability, and the composer actor picker.

**Spec:** `docs/superpowers/specs/2026-07-13-m11d-2-dice-chat-wire-design.md` (committed —
every constant, rule, and grammar decision below is normative there).

## Model/Effort directives

Same as M11d-1 (recorded, do not re-ask): plan mainline; `shadowcat-coder` (sonnet/medium)
implementers as unnamed one-off dispatches; `shadowcat-spec-reviewer`+`shadowcat-code-reviewer`
(high) per task; `-opus` twins for the final whole-branch pair; escalation via twins.

## Buddy-check directives

- **Roll-execution unit (Tasks 1+3+4):** ONE buddy-check over the combined diff after all
  three land (PHASE = code) — untrusted-notation execution is the security core, and Task 1
  touches `eval/sum.rs` (buddy-check-worthy by standing dice-skill policy).
- All other tasks: standard two-reviewer gate.

## Global Constraints

- Gates per task as in M11d-1 (workspace pnpm gates for client tasks; cargo test/fmt/clippy
  for server tasks; `pnpm build` before any cargo build).
- The dice crate stays PURE (no ws/data/http imports) — all transport policy in
  `src/server/src/chat/rolls.rs`.
- `evaluate` determinism, the Total-vs-SuccessCount `oriented_margin` asymmetry, and every
  Hard Invariant in `shadowcat-codebase-dice` are inviolable.
- The card's `{@html}` single-sink invariant is untouched — roll segments render ONLY via
  escaped interpolation.
- Comments: present-tense, invariants first. Constants named exactly as in the spec
  (`MAX_ROLL_DICE=100`, `MAX_ROLL_RECORDS=1_000`, `MAX_EXPERTISE=100`, `MAX_DIE_SIDES=10_000`,
  `MAX_INLINE_ROLLS=8`).

## File Structure

```
src/server/src/dice/spec.rs          [M] serde defaults on optionals
src/server/src/dice/outcome.rs       [M] push checked_add guard
src/server/src/dice/eval/sum.rs      [M] fold checked_add/saturating
src/server/src/dice/notation/mod.rs  [M] ParseError Display (+ Token Display as needed)
src/server/src/chat/rolls.rs         [C] caps, seed, span scanner, execute, settings glue
src/server/src/chat/settings.rs      [M] dice-settings doc + resolve_dice_context
src/server/src/chat/mod.rs           [M] Segment variants, ingest roll stage, System notice,
                                         edit immutability, SendMessageError variants
src/client/core/src/chat-docs.ts     [M] roll_embed/roll_button mirrors + refine exclusions
src/modules/chat-card/src/MessageCard.svelte [M] roll block/inline/button + system styling
src/modules/chat-composer/src/Composer.svelte [M] actor picker
src/modules/game-settings/src/GameSettingsPanel.svelte [M] Dice section
src/client/ui-kit/src/locales/en.ts  [M] keys (per task)
```

---

### Task 1: Dice crate hardening

**Files:** `src/server/src/dice/{spec.rs,outcome.rs,eval/sum.rs,notation/mod.rs}` (+ the
notation error type's home if it lives elsewhere — find it).

- `spec.rs`: add `#[serde(default)]` to `Tier.label`, `Tier.tier_value`,
  `TotalConfig.difficulty`, `SuccessConfig.required_successes`, `SuccessConfig.crit_success`,
  `SuccessConfig.crit_fail` (and any other `Option<…>` field of a type reachable from
  `RollSpec` that lacks it — enumerate by reading the file). Test: a JSON `SuccessConfig`
  omitting those keys deserializes (the TODO'd partial-JSON gap).
- `outcome.rs`: `RawRoll::push` (and any other `next_id` increment site) uses
  `checked_add(1)` — on overflow, `debug_assert!(false, …)` + saturate (documented:
  unreachable under the chat boundary's `MAX_ROLL_RECORDS`, guard is defense-in-depth).
- `eval/sum.rs`: the fold's running total uses `checked_add` with a saturating fallback
  (same rationale comment). NOTE: this file is on the dice skill's buddy-check-by-default
  list — change ONLY the arithmetic wrapper, byte-identical group-boundary logic; the
  existing test suite must pass unmodified.
- `ParseError`: implement `Display` with player-presentable messages for every variant
  (no `{:?}` interiors — where a variant wraps a `Token`, render the token readably). Test:
  every variant's Display output contains no `{`/`Some(`/debug artifacts (iterate variants
  explicitly).
- Gates: `cargo test` whole crate; fmt; clippy. Commit:
  `feat(dice/m11d-2): wire-boundary hardening (serde defaults, overflow guards, Display)`

---

### Task 2: `dice-settings` ambient config doc

**Files:** `src/server/src/chat/settings.rs` (+ mod.rs export), tests.

**Interfaces (produced):**
```rust
pub const DICE_SETTINGS_DOC_TYPE: &str = "dice-settings";
/// Fail-closed like ChatContentPolicy: absent/malformed → Total + HighWins.
pub async fn resolve_dice_context(repo: &dyn Repository, world: Uuid) -> ParseContext;
// Body shape (serde, all #[serde(default)]):
// { "mode": "total" | "success_count", "direction": "high_wins" | "low_wins" }
```
Model on `resolve_content_policy` exactly (same fail-closed contract, same query idiom).
Tests: absent doc → Total/HighWins; each explicit combination; malformed body → default.
Commit: `feat(chat/m11d-2): ambient dice-settings config doc`

---

### Task 3: `chat/rolls.rs` — execution core

**Files:** Create `src/server/src/chat/rolls.rs` (+ `mod rolls;`).

**Interfaces (produced — the ingest task consumes these exact names):**
```rust
pub(crate) const MAX_ROLL_DICE: u32 = 100;
pub(crate) const MAX_ROLL_RECORDS: usize = 1_000;
pub(crate) const MAX_EXPERTISE: u32 = 100;
pub(crate) const MAX_DIE_SIDES: i64 = 10_000;
pub(crate) const MAX_INLINE_ROLLS: usize = 8;

/// One scanned chunk of a message body: literal text between spans, an
/// inline roll to execute, or a button to validate-and-store.
pub(crate) enum BodyChunk<'a> { Text(&'a str), Inline(&'a str), Button { formula: &'a str, label: Option<&'a str> } }

/// Balanced span scanner (spec §2 grammar): `[[…]]` with single-bracket
/// nesting depth so notation labels survive (`[[4d6[atk]]]` → formula
/// `4d6[atk]`); `roll:` prefix → Button, `|` splits an optional label.
/// Errors: unterminated span, more than MAX_INLINE_ROLLS non-text chunks.
pub(crate) fn scan_body(body: &str) -> Result<Vec<BodyChunk<'_>>, RollError>;

/// Fresh OS-entropy seed per roll: Uuid::new_v4 (v4 = 122 random bits via
/// the OS getrandom already backing every document id) folded to u64.
/// Nothing persists the seed — a stored RawRoll's naturals reproduce the
/// outcome (dice-engine invariant).
pub(crate) fn entropy_seed() -> u64;

pub(crate) enum RollError { Parse(ParseError), TooManyDice(u32), TooManyRecords(usize),
    ExpertiseTooLarge(u32), SidesTooLarge(i64), TooManyInline(usize), Unterminated }
impl Display for RollError { /* player-presentable, reuses ParseError's Display */ }

/// Parse → cap-validate → roll → evaluate. The ONLY untrusted-notation
/// execution path. Cap walk: sum DiceGroup.count over the Expr (recursive),
/// per-group DieKind::validate() + Numeric span ≤ MAX_DIE_SIDES; expertise ≤
/// MAX_EXPERTISE; post-roll records ≤ MAX_ROLL_RECORDS.
pub(crate) fn execute_roll(formula: &str, ctx: ParseContext) -> Result<(String, RollOutcome), RollError>;
// returns (formula.to_owned(), outcome) — the embed's two fields.
```
Unit tests: every cap (accept at limit, reject past it — construct via notation, e.g.
`"101d6"`, `"1d20e101"`); the scanner's full grammar matrix (plain text, single inline,
multiple, button ± label, nested `[label]`, unterminated, adjacent spans, `[[roll:]]` empty
formula → Parse error, MAX_INLINE_ROLLS+1); determinism (fixed NoiseRng seed via a test-only
seam — `execute_roll_with_seed` internal fn, `execute_roll` = entropy wrapper); entropy_seed
non-repetition sanity (two calls differ).
Commit: `feat(chat/m11d-2): roll execution core (caps, entropy, balanced span scanner)`

---

### Task 4: Ingest wiring — roll stage, System notices, immutability

**Files:** `src/server/src/chat/mod.rs` (+ conn.rs only if an error plumbing seam is needed —
expected NO transport change).

- `Segment` gains the spec §5 variants verbatim (RollEmbed{formula, outcome: RollOutcome},
  RollButton{formula, label: Option<String>}) — outcome types are already Serialize.
- `SendMessageError` gains `Roll(RollError)` and `RollImmutable`.
- `handle_send_message` roll stage (after the post-parse empty check, before
  `resolve_content_policy`):
  - `parsed.kind == Roll` → `execute_roll(&parsed.body, resolve_dice_context(repo, world).await)`;
    Ok → content = `vec![Segment::RollEmbed{…}]` (skip sanitize — no text); Err → author the
    System notice (below) and return WITHOUT creating the roll message (the handler returns
    the notice's publish Command instead — one message either way, so the flood budget stays
    1:1).
  - `Normal`/`Emote` → `scan_body(&parsed.body)`: all-Text → existing pipeline unchanged
    (byte-identical fast path); otherwise per-chunk: Text → shortcodes+sanitize (each chunk
    independently, per spec §2 documented limitation), Inline → execute_roll → RollEmbed,
    Button → parse-validate (execute_roll's parse+cap stage WITHOUT rolling — expose
    `validate_formula(formula, ctx) -> Result<(), RollError>` from rolls.rs) → RollButton.
    Any RollError → System notice instead of the message.
- **System notice** (spec §4): a helper `build_roll_error_notice(world, sender, channel, err)
  -> Document` calling `build_message_doc` with `kind: MessageKind::System`, `audience:
  Whisper{vec![sender]}`, content = one Text segment of `err.to_string()`, `source: None`.
  The existing exhaustive parse_command-can't-produce-System test MUST remain green (this
  producer is not the parser).
- **Immutability** (spec §2): `handle_edit_message` — stored `sys.kind == Roll` →
  `RollImmutable`; parsed NEW content kind == Roll → `RollImmutable`; edits NEVER call
  scan_body (inline spans stay literal text through sanitize; assert via test).
- Integration tests (follow the existing chat test scaffolding): result-message end-to-end;
  inline interleave (Text/Html + RollEmbed ordering); button storage; failed roll → System
  whisper notice to sender only (assert channel/owner/kind/audience + the roll message ABSENT);
  edit-of-roll and edit-into-roll both RollImmutable; whispered `/roll` stays recipient-only;
  stored pre-M11d-2 message round-trips (serde back-compat).
- Commit: `feat(chat/m11d-2): rolls execute at ingest; System error notices; roll immutability`

> **After Tasks 1+3+4: run the pre-authorized buddy-check on the combined diff; fix; re-verify.**

---

### Task 5: Client mirror — roll segments in `chat-docs.ts`

**Files:** `src/client/core/src/chat-docs.ts`, `chat-docs.test.ts`.

- `DieRecordSchema` (value, natural, kept, exploded, crit_success, crit_fail, label nullish,
  symbols string-array, expertise, group_index — mirror the Rust field names exactly; extra
  server fields must NOT fail parse: use `.passthrough()` on the record object) and
  `RollOutcomeSchema` (total, successes/pass/margin/tier_label/tier_value nullish,
  crit_successes, crit_fails, positive_counter, negative_counter, symbol_counts
  record<string,number>, records array) — both exported.
- `ChatSegmentSchema` gains `{kind:"roll_embed", formula, outcome: RollOutcomeSchema}` and
  `{kind:"roll_button", formula, label nullish}`.
- **`UnknownSegmentSchema.refine` exclusion list extends to `roll_embed`/`roll_button`**
  (the fail-closed pattern — a malformed roll segment fails the WHOLE message parse; pin with
  tests exactly like the Task-1 M11d-1 trio). `isKnownSegment` widens.
- Tests: both kinds parse; malformed roll_embed (missing outcome / outcome.total wrong type)
  nulls the message; unknown kinds still opaque; drift-note comment updated.
- Commit: `feat(chat/m11d-2): client mirror for roll segments (fail-closed)`

---

### Task 6: Card rendering — roll block, inline chip, button, System style

**Files:** `src/modules/chat-card/src/MessageCard.svelte`, tests, locales.

Per spec §7 (all escaped interpolation — NO new `{@html}`):
- kind=roll message whose content is a single RollEmbed → block form: formula line;
  prominent `successes` (+ pass/tier line) when `successes != null` else `total`; per-die
  chips (`value`, class:dropped={!kept}, class:crit-success/class:crit-fail, label chip,
  symbols joined); counters + symbol_counts rows when non-zero.
- RollEmbed inside Normal/Emote content → inline chip (successes ?? total) with
  `title={formula + ": " + kept die values}`.
- RollButton → `<button class="roll-btn">` (label ?? formula, ≥44px) →
  `ctx.chat.send({ channel: sys.channel, content: "/roll " + s.formula })` (Public — spec §2).
- kind=system → muted card + `t("chat.systemBadge")` badge (replace placeholder styling).
- Locale keys: `"chat.systemBadge": "System"`, `"chat.roll.formula": "Formula"`,
  `"chat.roll.successes": "{n} successes"`, `"chat.roll.pass": "Success"`,
  `"chat.roll.fail": "Failure"` (+ any the implementation needs — keep keys prefixed
  `chat.roll.`).
- Tests: block embed renders total vs successes correctly; dropped/crit chip classes; inline
  chip + title; button click sends the exact content/channel; system badge; a roll_embed
  message with malformed outcome renders nothing (mirror fail-closed, from Task 5).
- Commit: `feat(chat/m11d-2): card renders roll embeds, buttons, and System notices`

---

### Task 7: Composer actor picker (speaking-as-actor)

**Files:** `src/modules/chat-composer/src/Composer.svelte`, tests, locales.

- Dropdown before the textarea: options = "Myself" (default, sends no actor_owner) + actor
  docs the user may speak as (`ctx.documents.query("actor")` filtered `doc.owner ===
  ctx.selfId`, or ALL actors when `ctx.role === "gm"`), labeled via the same fail-closed
  name accessor the card uses (read how MessageCard resolves actor names — reuse
  `actorDisplayName` on the actor doc's system).
- Selection sends `actorOwner: { kind: "actor", actor_id }` through the EXISTING
  `ctx.chat.send` (widen the composer's call to pass it; ChatApi already accepts it).
  Sticky per session (plain `$state`, not persisted).
- Locale: `"chat.composer.speakAs": "Speak as"`, `"chat.composer.myself": "Myself"`.
- Tests: default sends no actorOwner; picking an actor sends the exact ref; player options =
  own actors only, GM = all; reactive to actor creation.
- Commit: `feat(chat/m11d-2): composer speak-as-actor picker`

---

### Task 8: game-settings Dice section

**Files:** `src/modules/game-settings/src/GameSettingsPanel.svelte`, tests, locales.

- A "Dice" section (the panel's existing config-doc section idiom — read the file): seeds/
  edits the `dice-settings` singleton (mode select: Total/Success count; direction select:
  High wins/Low wins), idempotent GM seed per the established reactive-seed pattern, writes
  via single-key `/system/mode` / `/system/direction` updates.
- Locale keys `gameSettings.dice.*`.
- Tests: seed idempotence; select dispatches the right update; matches the doc shape Task 2
  resolves.
- Commit: `feat(chat/m11d-2): GM dice-settings authoring (mode + direction)`

---

### Task 9: Integration pass (dispatcher)

Full gates (pnpm -r test/typecheck/lint, pnpm build, cargo test/fmt/clippy), boot smoke
(binary serves; a scripted end-to-end is covered by Task 4's integration tests), no drift in
generated types.

### Task 10: Docs + skills gate + final review + merge

- PLAN.md M11d-2 entry; TODO.md: close the resolved dice items (mark RESOLVED like prior
  entries: serde defaults, overflow guards, Display, validate() caller, expertise bound,
  dice-count cap) + add spec §10 deferrals (recalc-from-chat, rich tooltips, speak-as-token,
  crit notation).
- Skill updates: `shadowcat-codebase-dice` (wire boundary now exists — caps table, entropy,
  Display, first validate() caller, serde defaults; "no wire coupling yet" language must go)
  + `shadowcat-codebase-chat` (rolls stage, new segments, System producer, immutability,
  dice-settings, client mirror additions, card/composer). Dispatch the reviewed gate.
- Final whole-branch review: opus pair. Fix wave if needed. Merge `--no-ff`. NO push.

## Self-review

- Spec coverage: §1→T1+T3, §2→T3+T4, §3→T2+T8, §4→T4, §5→T4+T5, §6→T3, §7→T6, §8→T7, §9→per-task,
  §10→T10 TODO, §11→T10. No gaps.
- Types consistent: `execute_roll`/`validate_formula`/`scan_body`/`BodyChunk`/`RollError`
  names used identically in T3/T4; `RollOutcomeSchema` T5→T6.
- No placeholders: novel logic (scanner grammar, caps, notice, immutability) is specified
  with exact semantics; established idioms reference the real in-tree precedent by file.
