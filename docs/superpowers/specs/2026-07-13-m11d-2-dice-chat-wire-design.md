# M11d-2 · Dice → Chat Wire Integration — Checkpoint Design

> Parent specs: `2026-07-03-m11-dice-engine-design.md` (dice), `2026-07-03-m11-chat-system-design.md`
> §5 roll embeds. M11a/b shipped the pure dice library (no wire coupling); M11c+M11d-1 shipped the
> chat core + display. This checkpoint connects them: rolls execute server-side at chat ingest,
> outcomes ride the message body as new segment kinds, and the card renders them. It also closes
> every dice wire-boundary hardening item deferred "to M11d" in `docs/TODO.md`.

## 0. Load-bearing shape (decided)

**No new protocol, no ts-rs.** A roll enters as ordinary `SendMessage` content (`/roll …`,
`[[…]]`, `[[roll:…]]`); the outcome leaves as segments inside the opaque `system` body,
mirrored by `chat-docs.ts` Zod like every other body type. The dice crate stays pure — all
transport policy (caps, seeding, settings, error surfacing) lives in a new
`src/server/src/chat/rolls.rs` glue module. The client never constructs or sends a `RollSpec`;
notation strings are the only wire form, so the notation parser (+ the boundary caps below) is
the entire untrusted surface.

## 1. Wire-boundary hardening (closes the TODO.md "Server / dice" items)

All enforced in `chat/rolls.rs` before/around `roll()`; the dice crate gains only the two
overflow guards (its own TODO items):

- `MAX_ROLL_DICE = 100` — sum of `DiceGroup.count` across the parsed `Expr` (walk before
  rolling). Rejects the unbounded-`count` DoS/overflow class at the source.
- `MAX_ROLL_RECORDS = 1_000` — post-roll guard on `raws.records.len()` (explosion chains are
  random; `CHAIN_CAP=100/die` × 100 dice could reach 10k records — reject oversized results,
  the roll simply fails with a "roll too large" error).
- `MAX_EXPERTISE = 100` — cap `SuccessConfig.expertise` (the `O(N·E²)` DP DoS vector).
- `MAX_DIE_SIDES = 10_000` — cap `Numeric.max - min` span (with records ≤1000 and faces ≤10⁴,
  every i64 sum is overflow-free by construction).
- `DieKind::validate()` called on every parsed group (future-proofs the `Faces{[]}` panic
  surface even though notation can't construct `Faces` today).
- Dice crate: `RawRoll::push` gets a `checked_add` guard on `next_id` (documented unreachable
  under the boundary caps; debug-asserted + saturating in release), `eval::sum::fold` sums via
  `checked_add` with a saturating fallback — exactly the two TODO'd sites.
- `#[serde(default)]` added to `RollSpec`'s optional fields (`SuccessConfig.required_successes`
  etc. — the TODO'd partial-JSON deserialization gap), since `RollSpec` now serializes into
  stored embeds.
- `ParseError` gets the TODO'd `Display` impl (player-presentable messages) — errors are now
  user-visible (§4).

## 2. Execution flows (all server-side, at ingest only)

`handle_send_message` gains a roll stage between `parse_command` and `sanitize`:

1. **Result message** — `parsed.kind == Roll`: the whole body is the formula. Parse
   (`notation::parse(body, ambient_ctx)`) → caps → `roll(spec, NoiseRng::new(seed))` →
   `evaluate` → content becomes `[Segment::RollEmbed{…}]`. `source` keeps the raw `/roll …`
   text as usual.
2. **Precomputed inline** — for `Normal`/`Emote` bodies, `[[formula]]` spans (max
   `MAX_INLINE_ROLLS = 8` per message) are extracted from the RAW body BEFORE shortcodes/
   sanitize; each executes exactly like (1) and becomes an inline `RollEmbed` segment. The
   text chunks between spans run through the normal shortcode+sanitize pipeline EACH AS THEIR
   OWN unit, producing interleaved `Text`/`Html` segments. Documented v1 limitation: a
   markdown construct spanning an inline roll (`**bold [[1d6]] end**`) does not survive the
   split — each chunk sanitizes independently.
   **Span grammar (pinned):** dice notation itself contains `[label]` brackets, so the span
   scanner is BALANCED, not scan-to-first-`]]`: inside `[[…]]`, a single `[` increments and a
   single `]` decrements a nesting depth; `]]` terminates the span only at depth 0
   (`[[4d6[atk]]]` parses as the labeled formula `4d6[atk]`). An unterminated span is a
   visible roll error (§4), never silent.
3. **Roll button** — `[[roll:formula]]` / `[[roll:formula|Label]]` spans produce
   `Segment::RollButton{formula, label}` WITHOUT executing. The formula is parse-validated at
   ingest (caps included) so a stored button is never broken; the label is plain data (the
   card renders it Svelte-escaped). Clicking sends an ordinary
   `SendMessage{content: "/roll <formula>"}` to the carrying message's channel with `Public`
   audience — a button roll is a new, public, sender-attributed roll message; no new frame.

**Roll immutability (anti-cheat, decided):** rolls happen ONLY at send.
- `handle_edit_message` REJECTS an edit when the stored `kind == Roll` (a roll's outcome is
  immutable; delete remains available) and rejects an edit whose new content parses to
  `kind == Roll` (no editing a message INTO a roll). New `SendMessageError::RollImmutable`.
- Edited content never executes inline spans or produces buttons: `[[…]]` in an edit stays
  literal text through the normal sanitize path. One rule — "edits never create roll
  segments" — keeps the whole re-roll-by-edit cheat class closed.
- An edit is likewise rejected when the stored content already carries any roll segment — an
  executed inline roll's audit record cannot be erased by editing around it.

## 3. Ambient dice settings

`ParseContext{mode, direction}` is caller-supplied. New world-scoped singleton `dice-settings`
config doc (`chat/settings.rs` sibling pattern, fail-closed): `{mode: "total"|"success_count",
direction: "high_wins"|"low_wins"}` → `resolve_dice_context(repo, world) -> ParseContext`,
defaulting to `Total`/`HighWins` on absent/malformed. GM authoring: a small "Dice" section in
the existing `module-game-settings` panel (two selects, the panel's established config-doc
idiom). This makes `t<N>` notation usable in success-count systems per the M11b design.

## 4. Roll errors are visible (System notices via existing machinery)

A failed roll (parse error, cap violation) does NOT create the message. Instead the server
authors its first `MessageKind::System` message (the reserved variant finally gets its
producer — NOT via `parse_command`, preserving that invariant): `audience:
Whisper{[sender]}`, same channel, `user_owner = sender` (they own + may delete it), content =
one `Text` segment with the `Display`-formatted error. Flood budget was already consumed by
the attempted send; the notice itself bypasses the limiter (server-authored, 1:1 with an
already-budgeted attempt). The card renders `kind: system` with a distinct muted "System"
style (the M11d-1 reserved styling becomes real).

## 5. New segments + client mirror

Server `Segment` gains two kind-tagged variants (serde snake_case, like `text`/`html`):

```rust
/// A completed roll: the formula + the full deterministic outcome. `outcome`
/// embeds the evaluated RollOutcome (records included — natural faces make the
/// roll reproducible/auditable); spec/raws are NOT stored (recalc-from-chat is
/// deferred; pre-release, no data-continuity promise).
RollEmbed { formula: String, outcome: RollOutcome },
/// An unexecuted, parse-validated formula the card renders as a button.
RollButton { formula: String, label: Option<String> },
```

(`RollOutcome`/`DieRecord` are already `Serialize`; they ride the opaque body — still no
ts-rs.) Client `chat-docs.ts`: `ChatSegmentSchema` gains `roll_embed` (with a
`RollOutcomeSchema` Zod mirror of the outcome fields the card renders: `total`, `successes`,
`pass`, `margin`, `tier_label`, `tier_value`, `crit_successes`, `crit_fails`,
`positive_counter`, `negative_counter`, `symbol_counts`, `records[]` with `value`, `natural`,
`kept`, `exploded`, `crit_success`, `crit_fail`, `label`, `symbols`, `expertise`,
`group_index`) and `roll_button` variants; **`UnknownSegmentSchema`'s refine must extend its
exclusion list to the new kinds** (the Task-1 fail-closed pattern: a malformed known-kind
segment fails the whole message, never gets rescued as "unknown"). `isKnownSegment` widens
accordingly.

## 6. Entropy seeding (the flagged M11a caveat, closed)

Per-roll seed = OS entropy, zero new dependencies: `Uuid::new_v4()` (already used for every
document id; v4 = 122 random bits from OS-backed `getrandom`) folded to `u64`
(`(u128 >> 64) ^ u128 as u64`). Fresh per roll — no process-lifetime key to recover, nothing
persisted (a stored `RawRoll`'s naturals reproduce the outcome without the seed, per the
engine's design). A `RollSeed` helper in `rolls.rs` with the rationale comment.

## 7. Card rendering (extends `module-chat-card`)

- **`roll_embed` (block form, kind=roll result message):** a bordered roll block — formula
  line, prominent total (or `successes` + pass/tier when present), per-die chips (value;
  dimmed when `!kept`, ⭐/💥-styled when `crit_success`/`crit_fail`, label chip when labeled,
  symbols listed), counters + `symbol_counts` rows when non-zero.
- **`roll_embed` (inline form, inside a Normal/Emote message):** a compact highlighted chip
  (total or successes) with `title` tooltip = formula + per-die values (v1 tooltip via the
  native `title` attribute).
- **`roll_button`:** a real `<button>` (label ?? formula, 44px target) → `ctx.chat.send({
  channel: sys.channel, content: "/roll " + formula })` (Public audience). Disabled state not
  needed (formula pre-validated at ingest).
- `kind: system` messages: muted style + "System" badge (replaces the placeholder styling).
- All new strings via i18n keys; all data renders Svelte-escaped (segments carry NO HTML —
  the `{@html}` single-sink invariant is untouched; roll segments render as plain
  interpolations only).

## 8. Speaking-as-actor (composer attribution)

The composer gains an actor picker (dropdown listing actors the user may speak as: own actors
— `doc.owner == selfId` — plus ALL actors for the GM; "myself" default = no attribution),
sending the existing `actor_owner: ActorOwnerRef::Actor{actor_id}` wire field the whole stack
already supports (storage, redaction, card header rendering all shipped). Per-message, sticky
per session (local state, not persisted). Token-instance attribution (speak-as-token) stays
deferred (needs token selection context in chat — logged).

## 9. Testing strategy

- **Server (`rolls.rs` + integration):** cap enforcement (dice count, records, expertise,
  sides — each rejects with the right error); `/roll 2d6+3` end-to-end (message created,
  content = one RollEmbed, outcome arithmetic verified against a fixed-seed NoiseRng in unit
  form + structure-only over the entropy path); inline `[[…]]` splitting (text/embed
  interleave, markdown chunks sanitize independently, MAX_INLINE_ROLLS rejection); button
  parse-validation (bad button formula rejects the send); ambient `dice-settings` resolution
  (fail-closed + both modes drive `t<N>` correctly); System error notice (whisper-to-sender,
  right channel, sender-owned, delete-able; NOT rate-limited; parse_command still can never
  produce System — the existing exhaustive test must keep passing); roll immutability (edit
  of kind=Roll rejected; edit INTO /roll rejected; `[[…]]` in an edit stays literal); serde
  round-trip of both new segments incl. a stored pre-M11d-2 message; overflow guards.
- **Client:** chat-docs mirror (both new kinds parse; malformed roll_embed fails the whole
  message — the refine exclusion pinned; unknown kinds still opaque); card rendering (block
  embed with kept/dropped/crit chips, successes vs total display, inline chip + tooltip,
  button click sends the exact `/roll` content to the message's channel, system styling);
  composer actor picker (options = owned actors / all for GM, sends actor_owner, header
  renders attribution — the M11d-1 name-privacy tests already cover redaction).
- **Cross-cutting:** a full ingest→broadcast→render fixture: `/roll` produces a message whose
  parsed body renders a roll block; a whispered roll stays recipient-only (audience machinery
  unchanged — one pinning test).

## 10. Explicitly out of scope (logged)

- Recalculate-from-chat (reroll failures etc.) — the embed deliberately omits `spec`/`raws`;
  Phase 2 revisits with a persistence decision (TODO.md).
- Notation syntax for crit-event configs (`CritSuccess`/`CritFail` structs stay
  RollSpec-authored only — pre-existing gotcha, unchanged).
- Rich tooltips (popover with full die table) beyond the native `title` — TODO.md.
- Speak-as-token-instance attribution (needs token context in chat) — TODO.md.
- Per-channel/per-message dice-settings overrides (world-level only).

## 11. Codebase-skill gate

`shadowcat-codebase-dice` (wire boundary now exists: caps, entropy, serde-default,
Display, overflow guards, first production `validate()` caller) and `shadowcat-codebase-chat`
(rolls.rs stage, new segments, System producer, immutability seam, dice-settings doc, client
mirror + card/composer changes) both update before merge, reviewed per the standing gate.

## Buddy-check directives

- **Roll-execution ingest unit** (the `rolls.rs` stage + caps + immutability seam + System
  notice): ONE buddy-check over the combined diff after those tasks land (PHASE = code) —
  untrusted-input execution is this checkpoint's security core.
- Card/composer tasks: standard two-reviewer gate.
