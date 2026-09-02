# M14c-4 Dice References + Chat Channel — Implementation Plan

> **For agentic workers:** Execute task-by-task in order; each task's steps use checkbox
> (`- [ ]`) syntax. THIS RUN: executed by the Kimi session taking over the M14c campaign,
> mainline in worktree `C:/Dev/Shadowcat-m14c4` (branch `m14c-4-dice-references`).

**Goal:** The server resolves dice-notation references itself — `1d20 + str` on the wire,
resolved at ingest against the roll's actor binding through the M14c-1 formula engine — and
`MessageEngine.channel` is validated against the world's channel registry at ingest.

**Architecture:** New `formula::template` unit (behavioural twin of the TS `template` module,
corpus-pinned); a pre-parse substitution inside `chat::rolls`'s one execution path; bindings are
the frames' existing `actor_owner` / `combatant_id`; channel validation at the two ingest
chokepoints.

**Tech Stack:** Rust (server crate), ts-rs (unchanged — no wire-shape delta), Vitest, node
scripts.

**Spec:** `docs/superpowers/specs/2026-08-31-m14c-4-dice-references-chat-channel-design.md`
(decisions R1–R9; read it first).

## Execution directives

Kimi session, not Fable — `mainline-plan-execution` does not apply. Dispatch coder/explore
subagents freely for well-scoped tasks. **Every dispatched agent's first prompt MUST contain this
paragraph verbatim:**

> The iron rule is no deferrals, of existing work, or new work as it comes up - we fix this now
> unless I give my EXPRESS authorization. The only exception is if a bug or to-do has a genuine
> blocker that is already logged in a milestone in PLAN.md that has not been started yet. Another
> iron clad is rule is that when faced with a design fork, determine the best long term shape in
> keeping with our plans and goals, and implement accordingly. You only need to ask me if the
> question "what is the best long term shape in keeping with our plans and goals?" is not able to
> answer the question. Churn is not a concern. This paragraph must be copied verbatim to any
> agents dispatched in this campaign.

…plus the reporting rule: a subagent must deliver its report as the Agent tool result OR write it
to a named document; state which in the prompt.

## Buddy-check directives

Pre-authorized by the user ("You may use buddy checking as seems appropriate", carried from the
M14c-2 session). Run buddy-checking (two blind reviewers + brokered debate) at:
1. After Task 4 — the template twin + corpus + parity gate (Tasks 1–3 diff).
2. After Task 6 — the roll-path wiring + channel validation (Tasks 4–6 diff).
3. Final: two-reviewer branch review before merge.

## Global Constraints

- No lint suppressions of any kind (`#[allow]`, `#[expect]`, `eslint-disable`, `@ts-ignore`); no
  file-size allowlist entries — split instead (soft 5,000 / hard 10,000 lines).
- Rust test bodies in sibling files (`pnpm lint:inline-tests`); comments cite symbols, never
  files/lines; no milestone ids, spec pointers, dates, or history narration in code comments or
  test names (`check-comment-refs`).
- `dist/` must exist before any cargo build (already built in this worktree).
- Deletions via `trash`, never `rm`/`Remove-Item`; commits always `git commit -- <paths>`.
- Doc gates at completion: `cargo test`/`clippy`/`fmt`, `pnpm -r test`, `pnpm -r typecheck`,
  `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props`, `pnpm lint:comments`, `pnpm lint:file-size`,
  `pnpm lint:inline-tests`, `pnpm docs:check-examples`, skill checkers
  (`node scripts/check-skill-symbol-refs-cli.mjs`, `pnpm run test:scripts`).
- SQL schema unchanged. `ClientMsg` wire shapes unchanged (R3) — no ts-rs regeneration expected;
  if a type does change, regenerate and commit the bindings in the same commit.

---

### Task 1: conformance corpus — template section (failing harnesses)

**Files:**
- Modify: `src/client/formula/src/__fixtures__/conformance.json` (new `templates` array)
- Modify: `src/client/formula/src/conformance.test.ts` (template cases through
  `resolveNotationTemplate`)
- Modify: `src/server/src/formula/tests/conformance.rs` (template cases through the not-yet-built
  `formula::template::resolve_notation_template`)

**Interfaces:**
- Case shape: `{ "src": "1d20 + str", "bindings": { "str": 3 }, "expect": { "notation":
  "1d20 + 3[str]" } }` or `"expect": { "error": "unknown-ref", "detail": "unknown reference
  'str'" }`. `bindings` maps the dotted path to a finite number; a path absent from the map
  resolves to `unknown-ref` with the shared wording. Cases cover: identifier substitution
  (positive/negative/zero), keyword reservation (`kh` claimed, `str` intact), `1d` synthesis
  (after start / after identifier / after integer), label spans (kept verbatim; unterminated
  rejects with the UTF-16 position), dotted paths, split claims (`2hp` shape), the integer-only
  `type` error (3.5 value), the i32-magnitude `cap` error, the template-length `cap`, a
  non-ASCII template (UTF-16 position counting), and resolver pass-through of a well-formed
  `FormulaError` produced by the map itself.
- Both harnesses build the same stub resolver from `bindings` (sorted-map lookup; miss ⇒ the
  shared `unknown reference '<path>'` wording).

- [ ] **Step 1:** write the corpus cases + both harness sections. TS side should PASS (the
  function exists); Rust side FAILS to compile (twin absent) — commit corpus+TS only if the Rust
  compile break cannot be scoped to `#[cfg]`; preferred: keep the branch compiling by landing
  corpus+TS harness in this task and the Rust harness WITH the twin in Task 2.
- [ ] **Step 2:** `pnpm --filter @shadowcat/formula test` PASS (TS harness green against the
  existing implementation — this also audits the corpus cases for correctness).
- [ ] **Step 3:** `git commit -m "test(formula): notation-template conformance cases, TS harness" -- src/client/formula/`

### Task 2: `formula::template` — the server twin

**Files:**
- Create: `src/server/src/formula/chars.rs` (predicates promoted from `lexer.rs`; lexer imports
  them), `src/server/src/formula/template.rs`, `src/server/src/formula/template/tests.rs`
- Modify: `src/server/src/formula/mod.rs` (declare `chars`, `template`; re-export
  `resolve_notation_template`, `TemplateError` shape if any), `src/server/src/formula/lexer.rs`
  (use the shared predicates), `src/server/src/formula/proptests.rs` (template arm),
  `src/server/src/formula/tests/conformance.rs` (template harness — now compiles)

**Interfaces:**
- `pub(crate) fn resolve_notation_template(src: &str, resolve: &dyn Resolve)
  -> Result<String, FormulaError>` — spec §2: recognizer chain, `NOTATION_KEYWORDS`
  (`DICE_OPERATOR` idiom), `1d` synthesis, labeled substitution incl. `-N[path]`, integer-only
  and i32-magnitude rules, UTF-16 position counting, `MAX_FORMULA_LENGTH` in UTF-16 units.
  Never panics; never emits non-finite-derived text.
- Scan internals (`Claim`, recognizers, `claim_at`, `emit_claim`, `substitute_identifier`)
  private, mirroring the TS layout so the two can be read side by side.

- [ ] **Step 1:** implement per spec §2 (the conformance harness from Task 1 is the failing
  test; add unit tests in `template/tests.rs` for what the corpus cannot reach — the keyword
  list's exact contents, the `d`-adjacency rules, the placeholder-resolver independence).
- [ ] **Step 2:** `cargo test -p shadowcat formula` — PASS; `cargo clippy` clean; `cargo fmt`.
- [ ] **Step 3:** proptest arm: random sources + random resolver outputs ⇒ no panic, and any
  `Ok` output parses-or-fails only through the notation parser (never a template-crate panic).
- [ ] **Step 4:** sabotage evidence: flip one recognizer ordering (identifier before keyword) ⇒
  at least one corpus case fails on BOTH sides' harnesses once the TS twin is equivalently
  perturbed; restore byte-identical. Record the run in the commit message.
- [ ] **Step 5:** `git commit -m "feat(formula): notation-template twin over the shared conformance corpus" -- src/server/src/formula/`

### Task 3: three-declaration parity gate

**Files:**
- Modify: `scripts/check-notation-modifier-parity.mjs` (extract the Rust template list; 3-way
  diff), `scripts/check-notation-modifier-parity.test.mjs`

**Behavior:** TS `NOTATION_KEYWORDS` = Rust `formula::template`'s list = `P::modifiers` arm
heads + the dice operator `d`. Any pairwise difference fails `pnpm run test:scripts` naming the
divergent entries.

- [ ] **Step 1:** extend the extractor + tests (mutation: remove one keyword from the Rust
  template list ⇒ gate fails naming it; restore).
- [ ] **Step 2:** `pnpm run test:scripts` PASS.
- [ ] **Step 3:** `git commit -m "test(scripts): notation-keyword parity becomes three-declaration" -- scripts/`

- [ ] **Step 4: BUDDY-CHECK checkpoint 1** — buddy-check the Tasks 1–3 diff
  (`git diff origin/main...HEAD`), PHASE=code, packet = spec §§1–3. Fold fixes per protocol.

### Task 4: resolver plumbing

**Files:**
- Modify: `src/server/src/formula/resolver.rs` (`NoHostResolver` lands here, `pub(crate)`),
  `src/server/src/combat/eval.rs` (re-import; delete its private copy)
- Modify: `src/server/src/data/document.rs` (`pub(crate) fn embedded_actor_copy`), 
  `src/server/src/combat/eval.rs` (use it)
- Modify: `src/server/src/chat/mod.rs` or a new `src/server/src/chat/host.rs` —
  `pub(crate) async fn host_for_actor_owner(repo, world_id, &ActorOwnerRef)
  -> Result<Option<Document>, DataError>`
- Sibling test files for each moved/new piece.

**Behavior:** pure moves for `NoHostResolver`/`embedded_actor_copy` (callers updated, no
behaviour change — existing suites stay green). `host_for_actor_owner`: `Actor` → the doc;
`TokenInstance` → embedded copy else linked actor doc; world-pinned reads (the caller's ingest
gate already validated the ref; this function re-fetches and must not re-authorize — document
that contract on it).

- [ ] **Step 1:** failing test for `host_for_actor_owner` (token-with-embedded beats linked
  actor; token-without falls to linked; bare actor; absent docs ⇒ None).
- [ ] **Step 2:** implement; `cargo test -p shadowcat` PASS.
- [ ] **Step 3:** `git commit -m "refactor(formula,chat): shared no-host resolver, embedded-copy extraction, actor-owner host" -- src/server/src/`

### Task 5: roll-path wiring

**Files:**
- Modify: `src/server/src/chat/rolls.rs` (`execute_roll`/`execute_roll_with_seed`/
  `validate_formula` gain `host: Option<&Document>`; substitution first; `RollError` gains the
  reference variant with a clean `Display` arm + the exhaustive-variant test updated)
- Modify: `src/server/src/chat/mod.rs` (`handle_send_message`: lazy host resolution beside lazy
  `dice_ctx`; `/roll` + inline chunks pass the real host; button chunks validate through the
  placeholder-zero resolver)
- Modify: `src/server/src/combat/mod.rs` (`CombatRoll` arm: per-entry
  `combat::eval::formula_host` host), `src/server/src/ws/protocol.rs` (`CombatRollEntry.notation`
  doc rewritten: raw template, server-resolved)
- Tests: `src/server/src/chat/tests/*`, `src/server/src/ws/conn/tests/combat_intents.rs`,
  `src/server/tests/chat_ingress.rs`/`chat_delivery.rs` as the existing suites dictate.

**Behavior (spec §4):** stored `RollEmbed.formula` = the author's template verbatim; `spec`/`raw`
describe the substituted roll; unbound + referencing ⇒ `unknown-ref` System notice (chat) /
`CombatError::Roll` (combat); buttons store the raw template and ingest-validate via placeholder
substitution; recalc untouched.

- [ ] **Step 1:** failing tests — a `/roll 1d20+str` with a speak-as actor whose `system`
  carries the leaf resolves and the embed shows the template with a labeled chip; without
  speak-as ⇒ whispered notice naming `str`; inline roll in a Normal body same; a button with
  references stores raw and validates; `CombatRoll` notation resolves per combatant host
  (token-embedded copy case included).
- [ ] **Step 2:** implement. **Step 3:** `cargo test -p shadowcat` PASS.
- [ ] **Step 4:** `git commit -m "feat(chat,combat): server-side reference resolution at the roll boundary" -- src/server/`

### Task 6: channel validation

**Files:**
- Modify: `src/server/src/chat/settings.rs` or `chat/mod.rs` — `validate_channel`; 
  `src/server/src/chat/mod.rs` (`SendMessageError::UnknownChannel`, call after the rate check);
  `src/server/src/combat/mod.rs` (`CombatRoll` arm + new error arm)
- Modify: `src/server/src/data/engine/registries.rs` (`ChannelRegistryEngine::validate`),
  `src/server/src/data/engine/mod.rs` (`channel-registry` arm validates)
- Tests beside each.

**Behavior (spec §6):** unknown channel ⇒ player-presentable refusal; absent/unreadable registry
⇒ fail-closed generic; registry validate = non-empty map, non-empty names, keys ≤
`MAX_CHANNEL_CHARS`.

- [ ] **Step 1:** failing tests (send to an unregistered channel refused; registered posts; a
  tampered empty registry write rejected at ingress; `CombatRoll` to an unknown channel refused).
- [ ] **Step 2:** implement. **Step 3:** `cargo test -p shadowcat` PASS.
- [ ] **Step 4:** `git commit -m "feat(chat): message channel validated against the channel registry at ingest" -- src/server/`
- [ ] **Step 5: BUDDY-CHECK checkpoint 2** — buddy-check the Tasks 4–6 diff.

### Task 7: client — speak-as session state, button binding, channel UI guards, docs

**Files:**
- Modify: `src/client/ui-kit/src/appContext.ts` + a new sibling model file (the session-level
  sticky speak-as selection, `SpeakAsToken`'s shape), `src/modules/chat-composer/src/Composer.svelte`
  (read/write it), `src/modules/chat-card/src/MessageCard.svelte` (`sendRollButton` passes it,
  one-shot token precedence preserved)
- Modify: `src/modules/chat/src/channels.ts` (GM pseudo-channel targets the lowest-sorted
  registry id, `general` fallback only while the registry is absent), `ChatPanel.svelte`
  (`removeChannel` refuses the last channel)
- Modify: `src/client/formula/src/template.ts` (doc-only rescope to preview/authoring, R9)
- Vitest suites beside each; component tests where the module already has them.

- [ ] **Step 1:** failing tests per module. **Step 2:** implement. **Step 3:** `pnpm -r test` +
  `pnpm -r typecheck` PASS.
- [ ] **Step 4:** `git commit -m "feat(chat): speak-as as session state; roll buttons resolve as the clicker; channel guards" -- src/client/ src/modules/`

### Task 8: docs, skills, findings, gates, merge

- [ ] Amendment pointers: M14 design D13, M14b `CombatRoll` row ("*Amended (M14c-4)*").
- [ ] `docs/site/guides/creating-a-system.md` dice-and-chat + key-check sections; 
  `docs/site/protocol.md` frame notes.
- [ ] `docs/POST_WORK_FINDINGS.md`: mark the `pnpm docs:api:ts` finding Resolved (verified on
  post-M14c-2 main during baseline).
- [ ] PLAN.md marks M14c-4 done; HISTORY.md delivery entry.
- [ ] Skills in the plugin checkout (`~/.claude/skills/shadowcat-codebase`): dice, formula, chat,
  combat, client-shell amendments; `node scripts/check-skill-symbol-refs-cli.mjs` +
  `pnpm run test:scripts`; spec-reviewer dispatch on the skill diff; commit + push the plugin
  repo separately.
- [ ] Full gate run (Global Constraints list); fix anything red.
- [ ] Final two-reviewer branch review (Buddy-check directives item 3); address findings.
- [ ] Fetch + rebase onto `origin/main` (M15/M16/M17 agents land concurrently), merge `--no-ff`
  to main, run both suites on main, push, `gh run watch`.
