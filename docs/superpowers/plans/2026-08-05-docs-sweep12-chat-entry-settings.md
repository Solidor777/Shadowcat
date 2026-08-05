# Docs Sweep 12 — chat, entry, settings, sheets, examples Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development.
> **REQUIRED READING for every implementer and reviewer:** `docs/design/doc-sweep-truthfulness-rules.md`
> — hand it over BY PATH, never pasted into a brief (pasting is how the rules drift between
> dispatches).

**Goal:** Drive the last eleven unratcheted source roots — `chat` (51), `chat-card` (23),
`entry` (22), `examples/` (19), `settings` (13), `topbar` (7), `sheet-actor` (5), `assets` (4),
`chat-composer` (4), `game-settings` (3), `sheet-item` (3) — from **154 `lint:docs` problems to 0**,
then ratchet **every remaining package** to `error` in `eslint.docs.config.js` (both the `.ts` and
the `.svelte` block). This is the final content sweep of the Phase-1 documentation campaign.

**Architecture:** Seven content tasks grouped by subsystem rather than by size, because the claim
surface — not the warning count — is what costs review rounds. Chat splits three ways (pure logic /
panel / card+composer); `entry` is its own task (auth + invite surface); the small panels group into
two; `examples/` is deliberately alone, because it is the one place in this sweep where a ` ```ts `
fence is correct and required. Then ratchet-and-ship. Same shape as Sweeps 7 (620→0), 8 (339→0),
9 (276→0), 10 (217→0), 11 (157→0).

**Tech Stack:** TypeScript, Svelte 5 runes, ESLint + `eslint-plugin-jsdoc`, Vitest, `@shadowcat/core`
wire types, `@shadowcat/formula`.

## Global Constraints

- **Comment-only.** No runtime change in a content task. If you find a real defect, report it with
  reachability bounded — do not fix runtime code in a docs task. (Sweep 8 found a rendering bug, two
  docs-gate defects, and a sibling divergence exactly this way; Sweep 9 found a silent-no-op branch;
  Sweep 10 found a dead seam; Sweep 11 found two.) Correcting a **stale comment** is not a runtime
  change and IS in scope — see Rule 7.
- **Gate:** `npx eslint --config eslint.docs.config.js <file>` per file, plus a whole-package sweep
  before each task reports. The ratchet fails on ANY file left above 0, including one not enumerated
  here — **enumerate live counts, never trust this document's numbers.**
- **Per-task gates:** scoped counts at 0, `pnpm -r typecheck`, the package's own tests
  (`pnpm --filter <pkg> test`), `pnpm docs:check-examples`, `pnpm lint`.
- **Reports:** write to `.superpowers/sdd/2026-08-05-docs-sweep12-chat-entry-settings/task-N-report.md`
  and **`ls` it to confirm it exists before reporting back.** A Sweep-8 task reported writing one and
  had not; both reviewers lost their claims-table audit and that task's citation metrics are
  unrecoverable.
- **Staging:** stage only the files you edited, by explicit path. Never `git add -A`.
- **Reviewer dispatch contract (dispatcher-side, every task):** reviewers run read-only —
  Read/Grep/Glob/Skill, **no Write and no Bash** by standing user directive. So (a) pre-generate the
  diff to a file and hand over the path, and (b) **name the delivery channel explicitly: "send your
  findings back with SendMessage."** Omitting (b) makes a reviewer go idle having produced nothing
  recoverable.
- **The Rule 7 re-scan covers EVERY comment in range, not just the JSDoc blocks** (Rule 14). This
  sweep's files are unusually comment-dense in exactly the ungated forms: `ChatPanel.svelte` carries
  ~14 standalone `//` blocks and only 9 gated sites, and `MessageCard.svelte`'s single most
  security-relevant claim sits on a `const` (`ROLL_COMMAND_PREFIXES`), which `require-jsdoc` never
  visits. **Inventory the inline comments explicitly and state the count as a number, not an
  impression.**
- **Report contract:** a CLAIMS TABLE (claim → enforcing `file:line`, **path-qualified**, Rule 13),
  an explicit list of what the pre-existing-prose re-scan covered ("none found" only with a list),
  and any bug found with its reachability bounded.

## Baseline, measured at `7c0dbb9` (re-measure; do not trust these)

```
pnpm lint:docs   → 154 problems (0 errors, 154 warnings) across 27 files / 100 documentation sites
pnpm docs:check-examples → 332 TS doc examples typecheck OK
```

| Root | Problems | Sites | Task |
|---|---|---|---|
| `src/modules/chat` (`channels.ts` 26, `unread.ts` 12, `unreadBadge.ts` 4) | 42 | 15 | 1 |
| `src/modules/chat` (`ChatPanel.svelte`) | 9 | 9 | 2 |
| `src/modules/chat-card` (`MessageCard.svelte` 17, `RollTooltip.svelte` 6) + `chat-composer` (4) | 27 | 18 | 3 |
| `src/modules/entry` (`entryApi.ts` 13, `WorldSelect` 5, `Entry` 2, `Login` 1, `Setup` 1) | 22 | 19 | 4 |
| `src/modules/settings` (13) + `topbar` (7) | 20 | 16 | 5 |
| `src/modules/assets` (4) + `sheet-actor` (5) + `sheet-item` (3) + `game-settings` (3) | 15 | 11 | 6 |
| `examples/` (`rules.ts` 6, `CharacterSheet.svelte` 6, `index.ts` 4, `InitiativePanel.svelte` 3) | 19 | 8 | 7 |

## `@example` fences: this sweep INVERTS Sweep 11's rule, per-root

Getting this backwards fails `pnpm docs:check-examples`, in one direction silently.

**Modules (Tasks 1–6): UNTAGGED, every fence.** Each of the ten in-scope module packages exports
exactly one symbol from its `index.ts` — the module manifest const (`chat`, `chatCard`,
`chatComposer`, `settings`, `topBar`, `sheetActor`, `sheetItem`, `assets`, `gameSettings`) — except
`entry`, which exports only `export { default as Entry } from "./Entry.svelte"`, a component.
Verified by reading all ten `index.ts` files. **Nothing in `channels.ts`, `unread.ts`,
`unreadBadge.ts`, or `entryApi.ts` is reachable by workspace package name.** Follow the established
private-function idiom: a one-line note that the symbol is module-private plus the call shape.
`src/modules/panels/src/PanelHost.svelte:52-58` is the reference example.

**`examples/` (Task 7): ` ```ts ` fences, and they must genuinely typecheck.** `examples/*` ARE
pnpm workspace packages (`pnpm-workspace.yaml:5`) with `main: src/index.ts`, and
`workspacePackageDirs` walks `examples` explicitly (`scripts/extract-ts-examples.mjs:81`), so their
exports resolve by package name. Two of the four sites already do this and are the pattern to copy:
`examples/module-initiative-tracker/src/index.ts:12` and `:25`. `abilityMod` and `evalFormula` are
re-exported from `examples/system-minimal/src/index.ts:5`, so both are importable as
`shadowcat-example-system-minimal`.

**The count moves, and by exactly one.** The extractor scans `.ts` and `.svelte.ts` only — never
`.svelte` (`candidateFiles`, `scripts/extract-ts-examples.mjs:99`). Of the eight example-package
sites, exactly one needs a NEW `@example` on an importable `.ts` symbol: `abilityMod`
(`examples/system-minimal/src/rules.ts:4`). `evalFormula`, `rollInitiative` and `sortEntries` already
carry ` ```ts ` fences and need only `@param`/`@returns`. The two `.svelte` sites are not scanned.

> **Therefore this sweep must end at exactly 333 examples, not 332.** A different number means
> something else changed: investigate, do not proceed. Any number other than 333 in a task report is
> a finding. Task 7 owns this transition and must state the before/after explicitly; Tasks 1–6 must
> each still report **332**, unchanged.

## What makes this sweep different from the last five

Sweep 11 was the client's authoring surface for server-authoritative state. This is **the client's
account, membership, and message-history surface** — and its doc failure modes cluster into three
shapes, all of which have already produced shipped defects in this campaign.

- **Chat's client/server split is a two-axis contract, and the axes are asymmetric.**
  `channel` is a **purely client-side label**; `audience` is what the server enforces
  (`src/modules/chat/src/channels.ts:1-5` asserts this today, uncited). `inView`
  (`channels.ts:17-21`) makes "all" and per-channel views read every message regardless of audience,
  while the "gm" view filters on `audience.kind === "gm_only"`. **The load-bearing subtlety is that
  the client filter is not the secrecy gate** — a player's store never held a `gm_only` message in
  the first place, because redaction is per-recipient and server-side. Any doc phrased as "the GM
  view hides gm_only messages from players" inverts cause and effect and would invite someone to
  treat `inView` as a security boundary. See `shadowcat-codebase-chat` and
  [[fog-is-the-secrecy-gate-fail-closed]] for the posture. **Cite the server, or make no server
  claim** (Rule 2).
- **`ChatDerivationCache` is a memoization cache whose correctness rests on an immutability claim
  about the SERVER.** `channels.ts:30-38` states that a message's `channel`/`audience` are frozen at
  creation, "copied verbatim from the stored doc on edit, never re-derived" — attributed to "chat
  skill", which is not a citation (Rule 13). Everything downstream depends on it: membership and
  sorted position are cached per id and never recomputed (`deriveVisibleDocs`, `:83-116`). If the
  claim is false for any edit path — **including the tombstone path**, since chat's Delete is applied
  server-side as an `Operation::Update` tombstone (`src/server/src/chat/mod.rs:986-988`), not a hard
  Delete — the cache silently serves a stale view that a fresh cache would not. **Verify the claim
  against `src/server/src/chat/mod.rs` before restating it, and check specifically what a tombstone
  does to the `engine` band.** This is the sweep's highest-value verification.
- **The stalest comments are on `const`s and inline `//` lines, which no gate has ever counted**
  (Rule 14, promoted by Sweep 11 after two false `const` comments). Two concrete leads, both already
  located:
  - `src/modules/chat/src/ChatPanel.svelte:139-141` — "set_pointer cannot delete an object key, so
    genuine removal means dispatching the full map minus the removed key". The first clause is still
    true. The inference is **stale**: `FieldChange.remove` exists, `remove_pointer` handles object
    keys server-side, and there is a production client dispatcher —
    `unsetField` (`src/client/ui-kit/src/sheetEdit.ts:36`). Whole-field replace is now one of two
    options, not a forced consequence. Task 2 owns correcting this. **Do NOT recompose a new
    mechanism** — Rule 4, and [[corrections-are-riskier-than-originals]] measured seven consecutive
    rounds where the defect was in the correction. State what is true, delete what is not.
  - `src/modules/chat-card/src/MessageCard.svelte:107-110` — `ROLL_COMMAND_PREFIXES` claims "exact,
    case-sensitive match" with `chat::parse_command` (`src/server/src/chat/commands.rs`). Task 3
    must diff the two lists token-by-token (Rule 6) rather than restate the claim.
- **Rule 10 has a live target this sweep.** `removeChannel`'s comment names the "FactionsPanel
  idiom", and `src/modules/factions` is **already ratcheted and at 0 warnings**, so no task here is
  built to touch it. If the same stale inference sits in `FactionsPanel.svelte`, this sweep is the
  last chance to catch it before the campaign closes. Task 2 must check it and report the result
  either way — a bounded, named check, not an open-ended sweep.

---

### Task 1: `chat` pure logic — `channels.ts`, `unread.ts`, `unreadBadge.ts` (42)

15 documentation sites. The densest claim surface in the sweep and the only task whose subject is
framework-neutral logic.

- `src/modules/chat/src/channels.ts` (26): `postTarget` (:10), `inView` (:17), `byCreation` (:23),
  `createChatDerivationCache` (:48), `getParseCallCount` (:53), `resetParseCallCount` (:57),
  `insertionIndex` (:61), `deriveVisibleDocs` (:73), `computeVisibleWindow` (:121).
- `src/modules/chat/src/unread.ts` (12): `isAfter` (:15), `computeUnreadCount` (:25),
  `markAllRead` (:40).
- `src/modules/chat/src/unreadBadge.ts` (4): `ChatUnreadBadge` (:9), `get` (:13), `set` (:17),
  `subscribe` (:23).

- [ ] Enumerate live count. Document every symbol. Gates. Commit.

**Hot spots:**

- **The `ChatDerivationCache` immutability claim** — the bullet above. This is the task's main event.
  Open `src/server/src/chat/mod.rs`, find the edit and delete handlers, and establish what each
  writes to `engine.channel` / `engine.audience`. Then either cite it path-qualified or drop to a
  narrower statement the code here actually enforces.
  **The delete half is already located** and is a citation, not a search: `handle_delete_message`'s
  own doc comment states the tombstone leaves
  `channel`/`user_owner`/`actor_owner`/`audience`/`kind`/`edited_at` untouched
  (`src/server/src/chat/mod.rs:982-988`). Confirm it against the function body rather than its
  comment — a doc comment is not evidence for a doc comment — and note that this leaves the **edit**
  path (`handle_edit_message`) as the genuinely open half. Both halves must hold for the cache's
  "fixed for the id's lifetime" claim to be true; if only one does, say which.
- **`deriveVisibleDocs`'s removal branch** (`:98-113`) opens with "Removal never happens in practice
  (messages are soft-tombstoned in place, never hard-deleted from the store)" — a Rule 5 absolute
  guarding a branch that then exists anyway. Two separate questions, and the comment currently blurs
  them: (a) does any path remove a `message` doc from the store, and (b) is the branch nonetheless
  correct? `applyOperation`'s `case "delete"` (`src/client/core/src/store.ts:178-180`) is real
  receive-side code; whether a `message` doc ever reaches it is the reachability question (Rule 8).
  A world switch discarding the whole store is a third case and is not "removal" in this sense —
  say which you mean.
- **`getParseCallCount`/`resetParseCallCount` are labelled "Test-only instrumentation"** (`:53`) but
  `parseCallCount++` runs unconditionally inside `deriveVisibleDocs` (`:91`) in production. Document
  the accessor honestly: the counter is always live, the ACCESSORS are test-only. Check whether
  anything outside tests reads them before writing an absolute.
- **`computeVisibleWindow` claims "never mounts fewer rows than fit"** (`:128`). `visibleCount` is
  `ceil((clientHeight / scrollHeight) * totalCount)` (`:141`) — a proportional estimate over rows of
  variable height. Enumerate whether a run of shorter-than-average rows can make the estimate
  under-count before restating the guarantee; if it can, the honest statement is about the fallback
  branch (`:139`), which genuinely returns the full range. Rule 5.
- **`unread.ts`'s self-post asymmetry.** `computeUnreadCount` excludes the reader's own posts
  (`:33-34`, `sys.user_owner === selfId`); `markAllRead` (`:43-52`) does not exclude them and folds
  them into the frontier. Both are correct; the asymmetry is deliberate and currently undocumented.
  State it once, on the function where it matters, rather than twice (Rule 3).
- **`isAfter`'s tie-break claim** (`:15-18`) says it uses "the same created_at-then-id tie-break as
  `channels.ts`'s `byCreation`". That is a Rule 6 "mirrors X" claim across two files that must be
  diffed condition-by-condition — `byCreation` (`channels.ts:24-26`) returns a comparator number,
  `isAfter` returns a strict boolean, and they handle the equal-id case differently by construction.
  Verify or narrow; do not restate.
- **`ChatUnreadBadge` is a module singleton that outlives a world switch.** `chatUnreadBadge`
  (`:29-31`) claims `ChatPanel.svelte` is "the only writer" — a Rule 5 absolute, one grep to check.
  Also document `set`'s equality early-return (`:18`, no notify on an unchanged count) and what
  `subscribe`'s returned disposer does. If listeners accumulate across remounts, that is a finding to
  report with reachability bounded, not to fix here.

### Task 2: `chat` panel — `ChatPanel.svelte` (9)

9 gated sites, but the work is inverted: this file's gated surface is small and its **ungated
comment surface is the largest in the sweep**. Budget accordingly.

Gated sites: `:65`, `:98`, `:119`, `:126`, `:133`, `:182`, `:198`, `:203`, `:215`.

- [ ] Enumerate live count. Inventory EVERY comment (count it). Document all. Gates. Commit.

**Hot spots:**

- **`removeChannel`'s stale inference** (`:139-141`) — the bullet above. Correct it by deletion plus
  a true statement, not by composing a replacement mechanism.
- **The Rule 10 FactionsPanel check.** `src/modules/factions/src/FactionsPanel.svelte` is at 0
  warnings and outside every task's file list. Check whether it carries the same inference, and
  **report the result either way** — "checked, clean" is a valid finding; silence is not.
- **`isVisible`'s `offsetParent !== null` proxy** (`:211-217`) claims the panel host hides an
  inactive panel via `display: none` on an ancestor "(never `{#if}`)". Verify against
  `src/modules/panels`. Note that `offsetParent` is also null for a `position: fixed` element — if
  the panel host can ever produce one, the proxy has a second failure mode and the comment's
  "standard proxy" framing is too confident.
- **`readState`'s initializer runs once, non-reactively** (`:57`): a persisted marker from
  `ctx.uiState.getChatRead()` wins as-is; with none, the baseline is every message visible at mount.
  The comment states both halves already — verify each against the code before restating (Rule 7),
  particularly "otherwise a first-ever open would misreport the whole existing history as unread".
- **The visibility `$effect` (`:75-79`) writes `readState`, which `unreadCount` (`:58-61`) reads.**
  Document why that is not a reactive loop. Get the actual reason from the code — do not compose one.
- **`avgRowHeight`** (`:196`) derives from `scrollHeight / visibleDocs.length`, but `scrollHeight`
  measures the mounted window plus spacers, whose heights are themselves derived from
  `avgRowHeight`. Whatever the docs say here must be true of that circularity; the safe statement
  describes what the value is FOR (scrollbar proportion stability, `:194-195`) rather than asserting
  it equals a real average.
- **`prevMessageCount` is deliberately NOT `$state`** (`:166-169`) and the comment says why. Preserve
  the reason; it is a real reactivity invariant, not a style note.
- **The GM registry seed** (`:102-115`) cites the "FactionsPanel idiom" and
  [[contribution-seed-reactive-before-resync]] is the memory behind it. `seeded` is set before the
  dispatch, so the seed is once-only even if the dispatch fails — state that consequence plainly.

### Task 3: `chat-card` + `chat-composer` — `MessageCard.svelte`, `RollTooltip.svelte`, `Composer.svelte` (27)

18 documentation sites. The sweep's security-adjacent task.

- `src/modules/chat-card/src/MessageCard.svelte` (17): `:51`, `:63`, `:75`, `:90`, `:102`, `:138`,
  `:150`, `:155`, `:159`, `:162`.
- `src/modules/chat-card/src/RollTooltip.svelte` (6): `:18`, `:21`, `:25`, `:35`, `:49`.
- `src/modules/chat-composer/src/Composer.svelte` (4): `:62`, `:68`, `:86`, `:91`.

- [ ] Enumerate live count. Document all. Gates. Commit.

**Hot spots:**

- **`ROLL_COMMAND_PREFIXES`** (`MessageCard.svelte:107-110`) — diff `["/roll ", "/r "]` against
  `chat::parse_command` in `src/server/src/chat/commands.rs`, token by token, including the trailing
  space and the case sensitivity. Rule 6: open both sides and line them up. The existing comment's
  final sentence ("A bare `/NdM` shorthand matches neither prefix and is displayed verbatim") is a
  second, separable claim — check it separately.
- **`safeHref`** (`:75-88`) is the file's load-bearing security comment and it makes three distinct
  claims: (1) the server only ever stores an `http`/`https` preview URL, via `fetch_preview`'s scheme
  guard; (2) a stored `javascript:`/`data:` URL must never become a live anchor; (3) Svelte escapes
  attribute values but does not filter URL schemes. Claim (1) is a server claim with no `file:line`
  (Rule 2) — cite it or scope it. Claim (2) is the function's own contract and is fine. Claim (3) is
  a framework claim; keep it only if you can support it. **Do not weaken the defense-in-depth
  framing** — the point of the re-check is precisely that it does not trust claim (1).
- **`resolveActorOwnerName`** (`:51-61`) is documented at `:29-33` as inheriting "the OwnerOrGm
  name-redaction and dangling-reference fail-closed behavior" by reusing `resolveTokenActor` +
  `actorDisplayName`. Two things to verify: that the reuse is real for BOTH branches, and — the
  subtle half — that the `owner.kind === "actor"` branch's **synthetic** `WireDocument`
  (`:53`, an `as unknown as` cast with only `engine.actor_id`/`engine.overrides`) really traverses
  the same path. A cast-built stand-in that skips a field the read-through consults would make the
  inheritance claim false for exactly one branch. Redaction itself is server-side
  ([[server-mirrors-client-resolver-semantics]]) — the claim here is about code reuse, so say that,
  not that the client redacts.
- **`actorOpenRef`** (`:40-49`) cites "§5.4" with no path — Rule 13, unverifiable by construction.
  Resolve it to a real document path or drop the reference. Its substantive claim — presence in
  `ctx.documents` implies this recipient has READ — should be checked against whether an optimistic
  local insert can put a doc there ahead of the server.
- **`rollBlock`** (`:124-136`) already carries a precise, correct-looking rationale for testing
  `sys.content.length` against the RAW list rather than the filtered one. Verify it still holds and
  preserve the framing; do not "improve" it (Sweep 11's ToolRail lesson).
- **`hostOf` / `formatTime`** (`:67`, `:90`) — `formatTime` uses `getHours`/`getMinutes`, i.e. the
  viewer's local timezone, while `timeTitle` (`:95`) uses `toLocaleString`. Document the timezone
  behavior; it is the kind of thing a future agent will otherwise assume is UTC.
- **`canModerate`** (`:145`) is client-side affordance only — the server re-authorizes every edit and
  delete. Use the established "advisory, the server gates the data" framing rather than inventing a
  new one.
- **`doDelete`** (`:162-164`) sends chat's dedicated delete frame, which the server applies as an
  `Operation::Update` tombstone, **not** an `Operation::Delete`
  (`src/server/src/chat/mod.rs:986-988`). This is documented in `docs/OPEN_BUGS.md` and is the
  correct framing to reuse — verify the citation still lands before repeating it (Rule 12: a relayed
  finding is an uncited claim).
- **`Composer.svelte` is already the best-documented file in this sweep; the job here is mostly
  verify-and-preserve, plus four gated sites.** Three of its existing claims are worth the check:
  - `:49-53` — the cap divergence: JS `.length` counts UTF-16 code units, the server counts Unicode
    scalar values (`chars().count()`), "so the client can only over-block near the cap, never
    under-block." That directional argument is the whole reason the divergence is acceptable.
    Confirm the direction rather than assuming it, and keep the reasoning; a bare "known divergence"
    would leave a future reader unable to tell whether it is safe.
  - `:71-72` — "/-commands ride verbatim — the server (`chat::parse_command`) is the **sole** parser;
    the composer **never** inspects or branches on content shape." Two Rule 5 absolutes. Note the
    apparent tension with `MessageCard`'s `ROLL_COMMAND_PREFIXES`, which *does* inspect content
    shape — for DISPLAY, not for parsing. Both files are in this task: make sure the two docs
    together cannot be read as contradicting each other.
  - `:13-14` — "own actors only for a Player, ALL actors for a GM (spec §8)". The §8 reference is
    unresolvable as written (Rule 13), and the filter is a client affordance: the server
    independently gates actor attribution on send. Cite the server gate, or say nothing about
    authorization here.
- **`RollTooltip`'s touch rationale is precise and load-bearing** (`:25-29`): iOS Safari moves
  neither focus nor `mouseenter` on tap, and toggling on click for hover-capable devices would
  immediately re-close a hover-just-opened popover because `mouseenter` fires before `click`. Keep
  both halves — the second is why the `(hover: hover)` gate exists rather than an unconditional
  toggle. The claim that this media query is "already used for touch-affordance decisions elsewhere
  in this module family" is one grep; verify or drop it.
- **`RollTooltip`'s two Escape paths are deliberately redundant** (`:35-41` and `:43-54`): a
  hover-opened popover never focused the trigger, so `onKeydown` has no target and the
  document-level listener covers it. Documenting only one of them would make the other look like
  dead code to the next reader.
- **`$props.id()` is a shared convention across this task and Task 5.** `RollTooltip:10-13` cites
  `LauncherMenu.svelte` as the precedent for the stable per-instance id, and `LauncherMenu:7-9`
  cites the WAI-ARIA APG pattern. State the convention once, on the file that owns it, and have the
  other point at it — do not write two independent explanations that can drift apart.

### Task 4: `entry` — `entryApi.ts` and the four views (22)

19 documentation sites. The unauthenticated surface: config probe, login, first-run setup, world
list/create/delete, invite redemption.

- `src/modules/entry/src/entryApi.ts` (13): `getJson` (:3), `postJson` (:9), `getConfig` (:17),
  `getMe` (:21), `login` (:29), `setup` (:34), `listWorlds` (:45), `deleteWorld` (:49),
  `createWorld` (:54), `acceptInvite` (:60).
- `src/modules/entry/src/Entry.svelte` (2): `:20`, `:36`.
- `src/modules/entry/src/views/WorldSelect.svelte` (5): `:15`, `:21`, `:37`, `:46`, `:63`.
- `src/modules/entry/src/views/Login.svelte` (1): `:11`. `views/Setup.svelte` (1): `:12`.

- [ ] Enumerate live count. Document all. Gates. Commit.

**Hot spots:**

- **The error contract is inconsistent by design and must be documented per function, not once.**
  `getJson` throws on any non-2xx (`:5`); `getMe` maps 401 to `null` and throws otherwise (`:24-25`);
  `login` returns a bare boolean and swallows the status (`:31`); `setup` returns `{ok, status}` and
  swallows nothing (`:42`); `acceptInvite` maps every failure to `null` (`:72`). Five different
  shapes. Resist writing one shared sentence — that is the Rule 3 failure (extra legs) in reverse.
- **`acceptInvite`'s indistinguishable-404 property** (`:60-66`) is a real security design and is
  already stated well. It is a Rule 7 verify-then-preserve item, not a rewrite: confirm the server
  actually collapses unknown/malformed/expired/revoked/used into one 404 before restating it, and
  cite where. If you cannot confirm all five cases, narrow the list to the ones you checked rather
  than repeating the set.
- **`acceptInvite`'s body-not-URL comment** (`:68-70`) enumerates browser history, `Referer`, proxy
  logs, and the server's trace span. That is four legs supporting one conclusion (Rule 3) — but they
  are illustrative rather than load-bearing, so the fix if any is trimming, not verifying each.
  Decide once and say which.
- **The no-oracle rationale is stated THREE times across two tasks, and that is the Rule 3 problem
  in its purest form.** `entryApi.ts:60-66` (every rejection is one indistinguishable 404),
  `views/WorldSelect.svelte:60-62` ("inferring a reason would re-create the oracle the invite flow
  exists to remove"), and `settings/InviteManager.svelte:16-19` (the GM never names an account) are
  three copies of one design. **Do not delete any of them** — each sits on a different decision a
  reader is about to make, so all three earn their place. What must NOT happen is three independently
  worded restatements that drift: pick `entryApi.ts` as the statement of record, and make the other
  two point at it rather than re-argue it. Task 5 owns the third copy; coordinate through the report,
  and if the wordings already disagree today, that disagreement is a finding.
- **`setup` is the first-run bootstrap** and takes an optional `token`. Document what the token is
  for and what the caller should conclude from the returned `status` — it is returned specifically so
  the view can distinguish cases, so the view's use of it (`views/Setup.svelte`) is the contract.
- **`deleteWorld` is destructive and unconfirmed at this layer.** The confirmation is a
  type-the-exact-name gate in the view: `confirmDelete` returns early unless
  `deleteName === world.name` (`views/WorldSelect.svelte:22`), with `armDelete` (`:15-19`) toggling
  the confirm row. Document that the guard lives there, so nobody reads `deleteWorld` as safe to call
  directly.
- **`WorldSelect` swallows the error detail on all four paths, but only one of them does so for a
  security reason.** `redeem` (`:68-70`) must report generically — that is the oracle property above.
  `refresh`/`create`/`confirmDelete` (`:40-42`, `:55-57`, `:30-32`) swallow the thrown message purely
  for UX. Conflating the two would teach a reader that the generic-error habit is cosmetic
  everywhere, which is exactly the wrong lesson to leave on the invite path (Rule 8: pin the claim to
  the path that actually carries it).
- **`getConfig`/`listWorlds` return `ServerConfig`/`WorldEntry[]` from `@shadowcat/types`** — ts-rs
  generated. Do not restate field semantics here; point at the generated type. Duplicating them
  creates the second copy that drifts.

### Task 5: `settings` + `topbar` (20)

16 documentation sites across seven files.

- `src/modules/settings/src/InviteManager.svelte` (5): `:42`, `:57`, `:72`, `:82`, `:89`.
- `src/modules/settings/src/ModuleManager.svelte` (4): `:23`, `:28`, `:42`, `:49`.
- `src/modules/settings/src/UserManager.svelte` (3): `:20`, `:34`, `:46`.
- `src/modules/settings/src/Settings.svelte` (1): `:9`.
- `src/modules/topbar/src/LauncherMenu.svelte` (5): `:21`, `:27`, `:31`, `:36`, `:39`.
- `src/modules/topbar/src/TopBar.svelte` (1): `:13`. `Presence.svelte` (1): `:11`.

- [ ] Enumerate live count. Document all. Gates. Commit.

**Hot spots:**

- **`InviteManager` is the write side of Task 4's `acceptInvite`.** The two tasks describe one
  lifecycle from opposite ends and must not contradict each other. Read `entryApi.ts:60-74` before
  writing, and keep the authorization statement in ONE place — the redemption side — rather than
  restating it here (Rule 3).
- **`InviteManager`'s three uncited server claims, in descending value.** All three are load-bearing
  and none carries a `file:line` today (Rule 2):
  1. `:29-31` — "the server stores only a **hash** of it, so it is unrecoverable after this render."
     This is why the UI shows the code exactly once; if it were false the single-render behavior
     would be pointless ceremony. Cite the storage site or drop to what this component does.
  2. `:16-19` — "The GM **never** names an account — naming one would make the membership route a
     username-existence oracle." A Rule 5 absolute carrying a real security rationale. Verify the
     route genuinely takes no username before restating the oracle argument.
  3. `:12-14` — `WorldRole` "mirroring the server's `WorldRole`. Structurally assignable to it — a
     server-tier value is not expressible." A Rule 6 mirrors-claim: diff the three literals against
     the server enum rather than restating.
- **`ctx.members` is claimed to be a session-start snapshot — and TWO files in this task depend on
  that being right.** `:33-36` says the roster "must NOT come from AppContext's `members` map: that
  is a session-start snapshot, so a seat added during the session would never appear."
  `Presence.svelte:11` renders from that same map. Establish the map's real update behavior ONCE,
  then make both files agree with it. If the snapshot claim is true, `Presence` has a staleness
  window worth stating; if it is false, `InviteManager`'s whole `refresh()` rationale is stale prose.
  Do not document either file's view of `ctx.members` without checking the other. **Whichever way it
  resolves, say so in the report** — this is the task's main event.
- **`status()` vs `spent()` disagree about expiry, deliberately.** `status` (`:82-87`) treats
  `expires_at <= Date.now()` as "expired"; `spent` (`:89-91`) counts only `consumed_at`/`revoked_at`,
  so an expired-but-unspent invite still renders its revoke button. Also note `status`'s precedence
  (consumed → revoked → expired) and that `Date.now()` makes the expiry label a **client-clock
  guess** — the server decides redemption. Two findings sharing the expiry dimension is Rule 11's
  trigger: walk the dimension rather than documenting each in isolation.
- **`refresh()`'s `allSettled` rationale** (`:38-41`) — a failed roster read must not blank the
  invite list and with it the revoke buttons. Verify the code still matches (`:47-53` applies each
  result independently, then surfaces the first rejection) and preserve the reason; it is a real
  availability invariant, not a style note.
- **`UserManager` mutates users and roles**; every affordance is GM-gated client-side and
  re-authorized server-side. Use the established advisory framing. Do not assert a specific
  capability name unless you cite it (`cap::` constants live in `src/server/src/data/permission.rs`).
- **`ModuleManager` toggles module enablement.** Sweep 9's Rule-11 lesson landed on exactly this
  surface — "arms on module enablement" was corrected twice and was still a notch too wide the
  second time. Whatever gating claim you make, enumerate the conditions from the code.
- **`LauncherMenu:11-14` claims the panel list is "already gmOnly-filtered by the bound
  PanelsController (the host is the one place role filtering happens)"** — a Rule 5 absolute about
  where filtering lives, and the same advisory-vs-authoritative distinction as everything else in
  this sweep: the filter is a UI affordance, not a permission. Verify the "one place" half by grep.
- **`LauncherMenu`'s keyboard behavior is spec-anchored and should stay that way.** `:7-8` cites the
  WAI-ARIA APG Menu Button pattern for `aria-controls`; `:47-50` explains that Enter/Space on an
  already-open trigger closes rather than re-opens **so an empty menu is never a focus trap**. That
  reason is load-bearing — preserve it. Point at `createMenuKeyboard` in `@shadowcat/ui-kit` for the
  arrow-key contract rather than re-describing it (it is already documented there; a second copy is
  the one that drifts).

### Task 6: `assets` + `sheet-actor` + `sheet-item` + `game-settings` (15)

11 documentation sites across four small files.

- `src/modules/assets/src/Assets.svelte` (4): `:12`, `:29`, `:42`, `:55`.
- `src/modules/sheet-actor/src/ActorSheet.svelte` (5): `:58`, `:65`.
- `src/modules/sheet-item/src/ItemSheet.svelte` (3): `:49`, `:57`.
- `src/modules/game-settings/src/GameSettingsPanel.svelte` (3): `:87`, `:125`, `:133`.

- [ ] Enumerate live count. Document all. Gates. Commit.

**Hot spots:**

- **Run ONE Rule 11 dimension across all four files: does every `$derived.by` that reads
  `ctx.documents` call `subscribe()`?** `ActorSheet.svelte:23-26` states the rule and the
  consequence of breaking it — the derived "freezes at first read and never observes later edits,
  **corrupting the OCC `old` on any second edit**" — and names `GameSettingsPanel` as the pattern
  source. That is a data-corruption failure mode, not a reactivity nit
  ([[sheet-reactive-bridge-missing-subscription]]). Work from the real member list: enumerate every
  `$derived.by` in all four files, state for each whether it reads `ctx.documents` directly or
  through an already-subscribed derived, and **report the negative result too** — "these N are
  correct" is what makes the claim credible. A miss here is a live bug to report, not to fix.
- **`ActorSheet`'s header comment carries three separate claims** (`:6-13`), each needing its own
  check: that reads come from the OPTIMISTIC store so redaction and OwnerOrGm naming are inherited;
  that **every** edit's `old` is the RAW current stored value (Rule 5 absolute); and that
  `systemPrefix` **always** ends in `/system`, making `basePrefix` the sibling root for `/engine` and
  `/name`. The third is the one the code depends on structurally (`:19-21` derive from it by regex
  strip) — if it can be violated, `setEngine`/`setName` write to the wrong node.
- **`ActorSheet` and `ItemSheet` are sibling implementations of the same sheet contract**
  (`sheetContract("actor")` / `ITEM_DOC_TYPE`). If the Rule 11 pass above turns up a second
  divergence between them, walk that dimension too rather than documenting instances. Their edit
  paths go through `@shadowcat/ui-kit`'s `sheetEdit.ts` — `setField` and `unsetField` are already
  documented there (`src/client/ui-kit/src/sheetEdit.ts:23-37`); point at that contract rather than
  restating the OCC pre-image rule, which is where a third copy would drift.
- **`ActorSheet`'s `inventory` fails safe by design** (`:46-56`): only embedded items directly under
  an actor doc (`systemPrefix === "/system"`) are one-level openable, so a deeply-nested
  instanced-token actor shows no inventory rather than a broken ref. State that this is deliberate;
  an undocumented empty section reads as a bug to the next reader.
- **`GameSettingsPanel` was corrected in Sweep 11** — a skill claim that its raw-old-value bug was
  "found but NOT fixed" was itself stale and got fixed in `52e60d2`. All three of
  GameSettingsPanel/FactionsPanel/ConditionsPanel now read the raw stored value. Verify that is still
  true at HEAD before documenting the OCC pre-image behavior here; do not carry the claim forward on
  the strength of the commit message (Rule 12).
- **`Assets.svelte`** is the client half of the asset REST surface. Upload/replace ordering has a
  documented invariant — commit the DB row before swapping the file
  ([[commit-db-row-before-swapping-file]]) — but that is server-side. Say what this component does;
  do not import the server's invariant into a client doc unless this code depends on it.

### Task 7: `examples/` — the tutorial system and module (19)

8 documentation sites. **The only task in this sweep that writes ` ```ts ` fences.** Re-read the
`@example` fences section above before starting; a mis-tagged fence here either fails the gate loudly
(a fence that cannot resolve its import) or passes it silently while documenting nothing (an untagged
fence where a real one belonged).

- `examples/system-minimal/src/rules.ts` (6): `abilityMod` (:4) — needs `@param`, `@returns`, and a
  NEW ` ```ts ` `@example`; `evalFormula` (:9) — needs `@param` ×2 and `@returns` only, its fence
  already exists at `:15-19`.
- `examples/system-minimal/src/CharacterSheet.svelte` (6): `:18`, `:31`. **Untagged fences** — not
  scanned by the extractor.
- `examples/module-initiative-tracker/src/index.ts` (4): `rollInitiative` (:12), `sortEntries` (:25)
  — `@param`/`@returns` only; both fences already exist and typecheck.
- `examples/module-initiative-tracker/src/InitiativePanel.svelte` (3): `:25`, `:41`. **Untagged.**

- [ ] Enumerate live count. Document all. Gates — **including the 332 → 333 transition, stated
  explicitly**. Commit.

**Hot spots:**

- **These files are read by module authors as the canonical example.** A wrong claim here propagates
  into third-party modules, which is a strictly worse blast radius than an internal helper's doc.
  Weight verification accordingly.
- **`evalFormula`'s "never throws" contract** (`:9-13`) rests on a claim about `@shadowcat/formula`:
  the comment at `:26-27` says the library never throws and returns fail-closed error VALUES. That is
  a cross-package claim with no citation (Rule 2) — verify against `src/client/formula` and cite, or
  narrow to what this function itself guarantees (it catches nothing, so the guarantee is genuinely
  inherited, which makes the citation load-bearing rather than decorative).
- **`abilityMod`'s new example must be honest about the domain.** `Math.floor((score - 10) / 2)` is
  stated as "d20-family" — it is a system convention, not an engine rule. The example should not
  imply Shadowcat enforces it.
- **`InitiativePanel`'s `labelKey` comment** (`index.ts:57-59`) says keys absent from the host catalog
  fall back to their literal value because "community modules have no i18n registration seam yet".
  That is a time-scoped claim about a missing feature (Rule 5's "today" guidance). Verify the seam is
  still absent — per the standing directive, an unverified "blocked on X" claim is not acceptable —
  and keep the time-scoping.
- **`sortEntries` returns a copy** (`[...entries].sort(...)`, `:36`); document non-mutation
  explicitly, since `Array.prototype.sort` mutating in place is the assumption a reader brings.

### Task 8: ratchet every remaining package, and ship

- [ ] Re-run the FULL census: `pnpm lint:docs` must report **0 problems** repo-wide before any config
      edit. If any file is above 0, that file is the task — not the ratchet.
- [ ] Add every remaining package to BOTH ratcheted blocks in `eslint.docs.config.js`. The `.ts`
      block and the `.svelte` block are separate; a package with components ratchets in **two**
      places. Remaining, beyond the ten in-scope packages: `src/modules/core-ui`,
      `src/modules/statusbar`, `src/modules/sheet-fallback` (all already at 0 and never ratcheted),
      `src/types/index.ts`, and `examples/**`.
- [ ] **Verify each new glob individually at `error` severity** — a glob that matches nothing passes
      vacuously and reads identical to one that matches and is clean.
- [ ] **Mutation-prove both blocks:** add an undocumented function to one newly-ratcheted `.ts` file
      and one newly-ratcheted `.svelte` file, confirm each reports as an ERROR, revert. A green run
      over a block that silently visits nothing is the failure this step exists to catch.
- [ ] Full gate matrix: `pnpm lint:docs` (0), `pnpm lint`, `pnpm -r typecheck`, `pnpm -r test`,
      `pnpm docs:check-examples` (**333**), `pnpm build`, then from `src/server/`: `cargo test`,
      `cargo fmt --check`, `cargo clippy --all-targets`. Client build precedes any cargo build
      ([[embed-dist-compile-ordering]]).
- [ ] Docs sync: `docs/PLAN.md` (Sweep 12 COMPLETE; the content campaign closed), `docs/TODO.md`,
      `docs/OPEN_BUGS.md` for anything found, `docs/design/doc-sweep-truthfulness-rules.md` if this
      sweep earns a Rule 15.
- [ ] Reviewed skill-update gate: update the affected `shadowcat-codebase-*` skills (`chat` at
      minimum) and dispatch `shadowcat-spec-reviewer` on the skill diff. If no skill knowledge
      changed, **state that explicitly** — silence does not satisfy the gate.
- [ ] Commit. Whole-branch review. Merge `--ff-only`.

**Scope note, deliberate and stated rather than assumed:** this task ratchets every package to
`error` inside `eslint.docs.config.js`. It does **not** merge those rules into `eslint.config.js` —
that consolidation is the campaign's separately-listed "final repo-wide ratchet" step, and folding it
in here would batch a repo-wide lint-config change into a docs sweep.

**Known gap to log, not to close here:** `pnpm lint:docs` passes `scripts` as a path, but no config
block in `eslint.docs.config.js` has a `files` glob matching `scripts/**` — and the scripts are
`.mjs`, which no glob in the file matches either. Every `scripts/*.mjs` is therefore silently
unlinted by the docs gate and always has been. They are well-documented in practice
(`scripts/extract-ts-examples.mjs` carries full JSDoc), so this is a coverage gap rather than a
documentation gap. Log it in `docs/TODO.md` against the consolidation step, which is where the
`files` globs get rewritten anyway.
