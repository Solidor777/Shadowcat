# Debt Burndown Campaign — Design

Close every open bug and deferred to-do in Shadowcat, then build the seven follow-on feature
sub-projects that the backlog has been holding. This document is the campaign's source of truth:
its ledger enumerates every item, its adjudication rules decide what may be deferred, and its
phase design says how each cluster is built.

---

## 1. The campaign directive

The following paragraph is the user's directive verbatim. **It is copied into the first prompt of
every subagent dispatched during this campaign, without paraphrase.**

> Invoke the shadowcat core skill immediately. You goal is to close all existing bugs and to-dos
> within Shadowcat. The iron rule is no deferrals, of existing work, or new work as it comes up -
> we fix this now unless I give my EXPRESS authorization. The only exception is if a bug or to-do
> has a genuine blocker that is already logged in a milestone in PLAN.md that has not been started
> yet. Another iron clad is rule is that when faced with a design fork, determine the best long
> term shape in keeping with our plans and goals, and implement accordingly. You only need to ask
> me if the question "what is the best long term shape in keeping with our plans and goals?" is not
> able to answer the question. Churn is not a concern. This paragraph must be copied verbatim to
> any agents dispatched in this campaign.

Every dispatch additionally states its delivery channel explicitly: **return the report as the
result of the Agent tool, or deliver it via SendMessage, or write it to a named document.** The
channel is a property of the dispatch mode — an agent given a `name` is backgrounded and its
final text reaches nobody, so campaign dispatches that must report back are launched **without**
a `name`.

---

## 2. Adjudication rules

### 2.1 The blocked exception, made checkable

The directive admits exactly one deferral: a genuine blocker **already logged in a `PLAN.md`
milestone that has not been started**. Applying that needs a fact, so it was established rather
than assumed: **M1 through M13, including M8.5, M12.5 and the Pre-M10 cleanup, are all complete.**
The only un-started milestones in `PLAN.md` are **Phase 2 (Full table)**, **Phase 3
(Atmosphere)** and **Phase 4 (Platform & scale)**.

That converts a judgement call into a lookup:

> An item is validly blocked **only** if its blocking capability is named in Phase 2, Phase 3, or
> Phase 4. Anything else is built during this campaign.

A blocker naming a *completed* milestone is stale by construction and does not defer anything — one
such claim was found, and two more name as their blocker a capability that no milestone schedules
at all, which is not a blocker either but unscoped work. All three were verified against code
rather than trusted. Together with a fourth item blocked on runtime module management, the user has
ruled that **all four are built** (§4.3).

### 2.2 Design forks

A design fork is resolved by asking "what is the best long-term shape in keeping with our plans
and goals?" and implementing that answer. Churn is explicitly not a cost. Every fork this campaign
contains is resolved in §6 with its reasoning, so no phase re-litigates one mid-execution. A fork
reaches the user only when that question genuinely cannot answer it; §7 lists the ones that
qualify.

### 2.3 Keeps require the user's sign-off

There are no justified keeps, exemptions or carve-outs unless the user explicitly signs off.
An agent that concludes an item should stay open reports it as **unconverted and awaiting a
ruling** — never as `kept`, never as "the exception covers this". The first move is always to
remove the *need* for the exemption rather than to argue for it.

### 2.4 New work discovered during the campaign

New bugs and to-dos surfaced while the campaign runs are campaign work. They are not logged for
later.

- A new item is appended to the ledger (§4) with a `NEW-n` id, a one-line statement, and a phase
  assignment, in the same commit that discovers it.
- If its home phase is **current or future**, it is folded into that phase's plan.
- If its home phase has **already merged**, it is fixed immediately on the current branch — not
  parked — unless it depends on work scheduled in the current or a future phase.
- If it *is* so blocked, it is parked with an **explicit unblock trigger naming the phase that
  unblocks it**, and that phase's plan gains it as a required input. A parked item with no named
  unblocking phase is not parked; it is an unreported deferral.
- **Phase 8 cannot pass while any parked item's unblocking phase has already merged.** This is the
  structural half of the rule: without it, "picked up once unblocked" is a promise nothing checks.

Each phase begins by re-reading the ledger for parked items whose unblocker has since merged, and
ends by appending everything it discovered. The ledger is committed, so coverage is auditable
after the fact rather than reconstructed from memory.

### 2.5 Enumerated inputs, per-item disposition

Every phase plan carries its ledger ids as a **fixed input list**. A subagent may not regenerate
its own worklist from a fresh search — an agent that re-derives its worklist each round silently
drops sites that were named to it, and the resulting report reads as complete. Each task reports a
**per-id disposition line**; a phase is complete when every id in its input list has one. "Category
complete" is not an accepted report shape.

### 2.6 Repo boundary: Shadowcat engine versus Nightfox module

The user has ruled that Nightfox is its own project, out of scope for this campaign; any
Nightfox-specific item belongs in the Nightfox repo's own trackers, not here. The split is **by
which repo's source carries the defect, not by which repo surfaced it** — a rule the Nightfox
backlog itself states. An item found while exercising a Nightfox sheet is still a Shadowcat item
if the code that must change is Shadowcat's own engine or toolchain; only an item whose fix lives
in Nightfox's own source moves.

This direction is non-obvious, so four items name the worked example of a symptom surfaced through
Nightfox that stays here because the fix is Shadowcat's: PW8 (module registration cannot reach the
app context), PW10/TD50 (the module-facing i18n registration seam), PW11 (the effect
document-type constant has no engine home), and PW12 (no browser e2e harness for external
modules). All four are defects in Shadowcat's own source or toolchain that a Nightfox module
merely surfaced, so none of them move.

---

## 3. Campaign structure

**Sub-project 1 — Debt burndown.** Nine sequenced phases, one branch each. This document specs it
in full.

**Sub-projects 2–8 — the seven follow-on features** recorded in `TODO.md` under "Follow-on feature
sub-projects". They are to-dos blocked by nothing, so the directive puts them in scope; each keeps
its recorded requirement of an independent brainstorm → spec → plan cycle, run after Sub-project 1
merges. They are listed in §9 and are out of scope for *this* spec's phases.

Phase order is dependency-driven, not severity-driven. Phase 1's redaction-band classifier and
Phase 2's grid-extent symbol both replace a forked judgement with one shared symbol that later
phases read; the client phases consume Phase 1's wire changes; Phase 7's suppression audit runs
late because grouping arguments into the structs they already form rewrites server signatures that
earlier phases edit.

**Phase 1b** is its own branch and its own brainstorm → spec → plan cycle, scheduled immediately
after Phase 1 merges and before Phase 2 begins. Phase 2 does not depend on it, but it changes the
command representation, the event log, and resync — an event-schema change foundational enough
that no later phase should be built on the current shape.

| Phase | Branch scope | Ledger ids |
|---|---|---|
| 1 | Server — data, permissions, wire boundary | OB2, TD26, TD27, TD31, PW19 |
| 1b | Server — point-in-time replay redaction (event/command visibility snapshot) | PW19, NEW-2 |
| 2 | Server — scene geometry, movement, vision | PW1, PW2, PW3, PW4, PW5, PW31, TD17, TD18, TD19, TD48 |
| 3 | Server — ops, performance, asset staleness | OB4, TD4a, TD5, TD9, TD10, TD49 |
| 4 | Client — shell, session, boot, ui-state | TD3, TD4b, TD6, TD7, TD8, TD12, TD13, TD14, TD15, TD16, TD20, TD29, PW16, PW17 |
| 5 | Client — modules, UI, render | OB1, OB3, OB5, TD11, TD21, TD22, TD23, TD24, TD25, TD37, TD38, TD45, TD47, PW6, PW7, PW15, PW21, PW22, PW23 |
| 6 | Module toolchain — i18n seam, live module management | TD39, TD40, TD50, PW8, PW10, PW11 |
| 7 | Tooling, gates, test infrastructure | TD1, TD2, TD28, TD30, TD44, PW9, PW12, PW18, PW20 |
| 8 | Closeout — doc sync, skill gate, plugin, merge, CI | — |

---

## 4. The ledger

Built by reading `OPEN_BUGS.md`, `TODO.md` and `POST_WORK_FINDINGS.md` end to end. It lives here
rather than being produced by a triage phase, because a phase whose output determines the later
phases makes the plan un-writable until it runs. Phases consume these ids as fixed inputs (§2.5).

### 4.1 Open bugs

| Id | Item | Phase |
|---|---|---|
| OB1 | `PanelHost`'s `describeOp` never narrates `"open"`, on documented reasoning that no live path dispatches it — false; `PanelsApi.open` has two reachable callers, and the op can surface a minimized or closed panel into a docked group with no screen-reader announcement | 5 |
| OB2 | `property_overrides` keys are unrestricted, so a self-targeting `/permissions` pointer substitutes the fail-closed default permission set for a redacted viewer, and a nested `/permissions/...` pointer strips a required field and **panics** the per-recipient egress path — a denial of service against every reader of that document | 1 |
| OB3 | `makeTemplateTool` mixes a snapped anchor with a raw pointer point, so the near-zero-drag one-cell fallback effectively never fires on a snapping scene; it is also the only authoring tool of four with no extent guard on persist | 5 |
| OB4 | A connection that misses an out-of-band `AssetChanged{replaced}` frame keeps a stale image forever with no self-healing path, and an ordinary reconnect is enough to miss it | 3 |
| OB5 | The GM Settings hyperlinks checkbox is permanently non-functional on every world: it sends `?? false` as the optimistic-concurrency pre-image where the stored value is a literal `null`, so the server rejects every click and the field never self-heals | 5 |

**Phase 1 disposition — OB2.** DONE. Redaction now operates on content bands, never the structural
envelope, through one shared classifier (`REDACTABLE_BANDS`, `redaction_target`,
`RedactionTarget`) in the `data::permission` module. Ingress (`validate_property_overrides`)
rejects an unclassifiable pointer at all four write paths — `apply_intent`'s Create and Update,
`apply_command`'s Create and Update. Egress (`filter_properties`) returns `Result<Document,
RedactionError>`; both former panicking `.expect()`s are gone, and every caller fails closed.
Evidence: `docs/CLOSED_BUGS.md` "Server / data — unrestricted `property_overrides` pointer
substituted or panicked the envelope"; the mutation-checked test suite in `data::permission` and
`data::validation` (per-pointer ingress rejection for each envelope field, acceptance for the four
bands and their nested forms, the exact nested-permissions regression, and the
`REDACTABLE_BANDS`-removal mutation check).

### 4.2 To-dos — built this campaign

| Id | Item | Phase |
|---|---|---|
| TD1 | Suppression allowlist gate: build the allowlist and checker, then **audit** 26 existing sites into it. The reason field is the mechanism, so a site with no honest site-specific reason is a fix or a proposal, never an entry | 7 |
| TD2 | Give the e2e suite per-worker accounts so parallel workers stop contending on one account's ui-state slice | 7 |
| TD3 | `setGmViewedScene` leaves a stale cross-scene token selection; scene-scope `tokenSelection` rather than clearing it. The server rejection is correct and must not be relaxed | 4 |
| TD4a | `merge_ui_state` is grow-only; add null-removes-entry/key semantics mirroring `FieldChange.remove` | 3 |
| TD4b | Client-side ui-state pruning so an over-cap blob is recoverable | 4 |
| TD5 | `put_ui_state` opens the single-writer transaction before the merged-size check; add a route-level pre-check | 3 |
| TD6 | `sessionState` has no in-flight-PUT ordering guard; two writes for one account can be in flight with no ordering guarantee | 4 |
| TD7 | `sessionState`'s `loaded` flag is never reset on logout, so a mutation inside a re-login load window can persist pre-login state under the new session's cookie | 4 |
| TD8 | `buildGlobalPatch`/`buildWorldPatch` enumerate leaf keys by hand; drive from an exhaustive record so a widened type becomes a compile error | 4 |
| TD9 | The Welcome preamble runs a full filesystem module scan on **every** WS connect; cache it and invalidate on install/uninstall | 3 |
| TD10 | tower-sessions shares the single-connection pool, queueing every session read behind app writes; give it its own connection | 3 |
| TD11 | `Stage`'s backend-init failure path sets a data attribute silently; route it through the project logger | 5 |
| TD12 | `WsClient.open()` adopts a resolving transport without re-checking `running_` after the await, leaving an adopted-but-unwatched socket after `stop()` | 4 |
| TD13 | `App`'s `boot()` captures the route once before an await chain and resolves against the stale value | 4 |
| TD14 | `WorldSession`'s Welcome activation rethrow short-circuits every subsequent step on **every** Welcome, not just the failing one | 4 |
| TD15 | `boot()`'s worst case is ~2.4 minutes on "Loading…" with unjittered lockstep retries; add a deadline, a visible still-trying state, and jitter matching the WS backoff | 4 |
| TD16 | `effectiveOwner` omits the server's scope guard, making parity depend on an unstated invariant | 4 |
| TD17 | `Room::execute_move` re-derives `is_gm` instead of reusing the binding already in scope | 2 |
| TD18 | `SceneEcs::blocks_move` has no production caller; re-wire or delete | 2 |
| TD19 | `execute_move`'s footprint gate has a residual anchor asymmetry on off-center input (wall disc at the literal point, mask disc at the cell center) | 2 |
| TD20 | `listWorldMembers`/`WorldMember` exist twice and have already diverged three ways, with **neither a superset** of the other; one implementation combining all three properties | 4 |
| TD21 | A negative template substitution emits an unlabeled zero-minus form, so a negative modifier vanishes from the roll breakdown while a positive one is attributed | 5 |
| TD22 | `TemplatesController.push` filters on two bands but the Update it builds also emits `/embedded` changes gated by a different capability, so an affected instance receives **none** of the push and stays stale | 5 |
| TD23 | `EngineAdapter.focus` is implemented by both engines and called by neither; wire it or delete the seam, reconciling the stage-well guard that exists on only one adapter | 5 |
| TD24 | The condition-registry seed uses a random id where its faction sibling uses a deterministic one | 5 |
| TD25 | `ConditionsPanel`'s `isActive` and `toggle` disagree on which selected tokens count, producing a click that changes nothing with no user-visible reason | 5 |
| TD26 | `FieldChangeSchema` accepts a frame omitting `old`/`new`; require the keys while still permitting an explicit null value | 1 |
| TD27 | `WireSearchHit.snippet` carries no inert-text exposure note, and no consumer exists yet — which is exactly why it lands now | 1 |
| TD28 | `check-comment-refs.mjs` cannot see a skill-name repo pointer, so a green run is not evidence of compliance for that shape | 7 |
| TD29 | `WorldSession.subscribeScene`/`sendChatMessage` repeat the unbound-parameter-doc defect; give them named options types | 4 |
| TD30 | The surviving-absolute-reference check cannot see an inline `<style>` block; parse out the style regions rather than widening the predicate to whole HTML files | 7 |
| TD31 | `WireCapabilityGrants.by_role` is typed wider than its Rust source; narrow to a partial role-keyed map | 1 |
| TD37 | Rotation authoring, plus the shortest-signed-delta wrap-aware rotation lerp it makes reachable | 5 |
| TD38 | A minimal scene-background authoring UI, plus the e2e assertion its absence has been blocking | 5 |
| TD39 | Extend topology reconciliation past presence to version and contract mismatches | 6 |
| TD40 | `LauncherMenu` has no focus-recovery path if the module map mutates while the menu is open | 6 |
| TD44 | Drop-position classification fidelity against real pointer geometry, currently manual-QA-only | 7 |
| TD45 | The bespoke-fallback engine's tab strip has no panel menu, so a panel docked under it cannot leave a zone through any UI affordance | 5 |
| TD47 | Two consumer-resolver call sites in the formula package wrap near-identical try/catch handling; factor the shared helper | 5 |
| TD48 | Stored explored-fog blobs carry no grid-kind tag, so switching a live scene between square and hex reinterprets the blob | 2 |
| TD49 | Server shortcode replacement also fires inside markdown code spans | 3 |
| TD50 | External modules cannot register i18n keys, so a community module's label renders as its literal key and the authoring guide instructs authors around the gap | 6 |

**Phase 1 disposition — TD26.** DONE. `WireFieldChange`'s `old`/`new` are now required keys on the
client's inbound wire schema (`FieldChangeSchema`), rejecting a frame that omits either while still
permitting an explicit `null`/`undefined` value — matching the Rust `FieldChange` source, which
never omits them. Removed from `TODO.md`.

**Phase 1 disposition — TD27.** DONE. `WireSearchHit.snippet`'s doc gained the inert-text exposure
note ported from `crate::chat::MessageEngine`'s `source` field: every `engine` string leaf swept
into the FTS index can surface through a search hit's snippet or document, so a consumer must
render it as inert text, never innerHTML. Removed from `TODO.md`.

**Phase 1 disposition — TD31.** DONE. `WireCapabilityGrants.by_role` is narrowed from
`Record<string, string[]>` to a partial map keyed by `DocRole`
(`Partial<Record<z.infer<typeof DocRoleSchema>, string[]>>`), matching
`crate::data::document::CapabilityGrants`'s `BTreeMap<DocRole, BTreeSet<String>>`. `by_user` stays
`Record<string, string[]>` deliberately — its keys are user ids, which are genuinely open. Removed
from `TODO.md`.

### 4.3 To-dos — adjudicated

**Validly blocked** (blocker named in an un-started Phase 2/3/4 milestone). These seven stay in
`TODO.md` with their blocker text corrected to name the phase:

| Id | Item | Blocking milestone |
|---|---|---|
| TD34 | `ClientIp` has no forwarded-header handling, degrading the per-IP throttle behind a proxy | Phase 4 — hardening & distribution |
| TD35 | Execution cost and router cost are not numerically comparable under non-Chebyshev diagonal rules | Phase 2 — combat tracker |
| TD36 | Smoothed continuous routes report the pre-smoothing grid cost | Phase 2 — combat tracker |
| TD41 | Multi-provider conflict policy for singleton surface contracts | Phase 4 — module API freeze |
| TD42 | Capability version negotiation for contract dependencies | Phase 4 — module API freeze |
| TD43 | An open pop-out window has no drop subscription, so a cross-window drag would bypass the reducer | Phase 2 — layout/theming completion |
| TD46 | Live cross-animation concurrency for streamed move vision | Phase 2 — vision/lighting/movement completion |

**Stale blockers — verified against code, not trusted.** Three items claimed blockers that do not
hold. The user has ruled that all four unscoped items below are built:

- TD37 claimed rotation authoring was blocked on a milestone that shipped **without** adding it.
  Verified: no rotation authoring exists in any module. Built in Phase 5.
- TD38's heading names as its blocker the very UI it asks to build. Verified: no client path sets
  a scene background; the asset picker in the scene-tools module is token art. Built in Phase 5.
- TD50 is blocked on a seam no milestone schedules. Verified: no catalog-registration API exists
  anywhere in the client packages. Built in Phase 6.
- TD39/TD40 are blocked on live module management. Per-world enable/disable exists; runtime
  install/uninstall does not. Built in Phase 6.

**User action, not agent-doable.** A per-machine click inside another program's UI cannot be
closed by any agent. It is surfaced to the user rather than claimed:

- TD33 — install the plugin in the Kimi TUI and confirm all six agents register; two specific
  failure modes are predicted and need one real dispatch to settle. TD33 stays here: the plugin is
  Shadowcat's own artifact, and the fact that verifying it happens inside a Nightfox workspace does
  not make the item Nightfox's, per §2.6.

TD32 — accept the trust dialog once in the Nightfox workspace — moved to the Nightfox repo's own
backlog as a per-machine setup entry, per §2.6: the trust dialog gates a Nightfox workspace, not
Shadowcat's own toolchain.

### 4.4 Post-work findings — triaged

Per the user's ruling, every live entry is triaged; anything promoted to a bug is still fixed in
this campaign.

| Id | Item | Disposition |
|---|---|---|
| PW1 | `ResolvedScene.bounds` has **two contradictory unit interpretations** across two consumers of the same value, and the grid-unit reading's hex conversion is wrong — a hex continuous scene gets a navmesh ~58% of intended width, so legitimately reachable cells report unreachable | Promote to bug; Phase 2 |
| PW2 | Hex continuous weighted preview cost is ~1.73× too small, defeating the unit parity the conversion exists to guarantee | Promote to bug; Phase 2 |
| PW3 | The one hex + continuous integration test runs as GM, so the fog clip returns early and is never exercised through the real dispatch path | Add the test; Phase 2 |
| PW4 | Environment light is flat ambient rather than edge-projected, because the scene model was dimensionless. **Scene bounds now exist**, so the constraint that forced this is gone | Now unblocked; Phase 2 |
| PW5 | A vision-mode entry missing its illumination floor is silently dropped with no diagnostic | Phase 2 |
| PW6 | Lighting softens band edges with a blur rather than per-cell radial gradients | Phase 5 |
| PW7 | Darkvision render is a gray-wash approximation; the wire payload already carries the faithful hint | Phase 5 |
| PW8 | Module registration cannot reach the app context, forcing every stateful module to repeat a construct-at-mount-and-bridge dance — filed as an API bug report under the "built against the public API" rule | Phase 6 |
| PW9 | Native pointer tab-drag is not exercisable by the current automation | Phase 7 (with TD44) |
| PW10 | External-module i18n registration seam missing | Phase 6 (with TD50) |
| PW11 | The effect document-type constant has no engine home and is defined in a module's own barrel | Phase 6 |
| PW12 | No browser e2e harness for external modules | Phase 7 |
| PW13 | Drag reorder is pure HTML5 drag-and-drop, which iOS Safari does not fire from touch — a **named-platform violation of the cross-platform directive** with zero test signal | Moved to the Nightfox repo as an open bug, per §2.6 — the affected source is Nightfox's own sheet component |
| PW14 | Numeric field edits silently no-op on invalid input, and one-way bindings leave stale text with no signal | Moved to the Nightfox repo as an open bug, per §2.6 — the affected source is Nightfox's own sheet component |
| PW15 | Two comments state opposite server orderings, each citing the other as rationale; exactly one is wrong and a maintainer reasoning from it could remove the load-bearing guard | Phase 5 |
| PW16 | Invite redemption has no network-exception path, unlike its three siblings, so an offline attempt shows the user nothing | Phase 4 |
| PW17 | A controller captures session sub-objects at construction; the leave-then-enter path is proven safe, but world-to-world switching without passing through null is untraced | Phase 4 |
| PW18 | A whole-suite e2e failure mode with no captured evidence and an unknown cause; mitigated by trace retention, not fixed | Phase 7 — bounded investigation; see §7 |
| PW19 | Replayed history is redacted against the *current* permission set rather than the set in force at that sequence | Confirmed defect (buddy-check convergence); ruled fix: snapshot the relevant visibility into the event/command at commit time, so replay redacts against the policy in force at that sequence; Phase 1b |
| PW20 | The deletion deny-list may only block the naive invocation shape; the pattern-matching semantics are unverified and the only direct test is running a banned destructive command | Phase 7 |
| PW21 | No smaller caption text-size token exists; deferred to a milestone that **has now shipped** | Stale deferral; Phase 5 |
| PW22 | Config-doc seeds race resync and can double-create; deferred to a milestone that **has now shipped** | Stale deferral; Phase 5 |
| PW23 | The world-defaults editor authors only a subset of the settings that resolve at world level | Phase 5 |
| PW31 | A lenient-mode near-corner move can be spuriously rejected by an over-firing corner epsilon | Phase 2 — re-verify against the shipped corner-drift fix first |
| NEW-1 | PW19 was reached by exactly one analyst's single-pass "accepted, no action" reasoning and turned out to be a live secrecy defect on buddy-check. The same reasoning shape produced every item in the batch below, so the whole batch — not only the secrecy-adjacent entries — got the same two-blind-reviewer adversarial treatment. Complete: 13 findings re-triaged, converging after three rounds, both reviewers independently passing the embedded PW19 positive control (RT-7). Result: 10 closing arguments STAND (verified against real code); 3 OVERTURNED — PW19 itself (already tracked above), a newly confirmed defect (NEW-2), and one entry closed as a stale record describing a lint-config shape that no longer exists. | Complete |
| NEW-2 | An `Update` to a since-deleted document is redacted against a NEW document that later reuses the freed id, not dropped as the original closing analysis assumed — final-state convergence survives (the corrective Delete/Create frames follow in the same resync batch), but the stale `Update` is redacted against the wrong document's permission set in the window before those frames land, producing an over-reveal or under-reveal. Shares its root cause and its ruled fix with PW19 (point-in-time state snapshot at commit time). | Confirmed defect (NEW-1 adversarial re-triage); Phase 1b |

**Re-triaged by NEW-1 — verified dispositions, not provisional ones.** Two blind reviewers checked
each closing argument against real code rather than against the entry's own summary, converging
after three rounds. Ten **STAND**:

- The movement-gate ECS-hydration shape — unreachable: `SqliteRepository::apply_intent`'s Phase 1
  rejects any same-batch Create+Update before commit, and `Room::publish`'s `publish_guard`
  serializes every `publish` call for a room end-to-end, so a racing publish cannot land mid-hydration
  either.
- The lenient-mode near-corner over-rejection — over-inclusion can only ever over-reject, never
  admit a forbidden move.
- Capability grants targeting the no-access role — GM-authored, and no non-GM path reaches it.
- `core:delete` defaulting GM-only — a documented behavior change; nothing depends on the prior
  default.
- The dark-scene movement freeze (**working as designed — do not soften the defaults**).
- Offline-intent flush ordering ahead of the async Welcome body (eventually consistent, no
  correctness impact).
- Per-leg-greedy multi-leg parity (cost-display only; the route itself stays valid).
- The `Promise.all` in module management (**correct — "harmonizing" it to `allSettled` would let
  one click disable every module in the world**).
- The five-versus-six invite-rejection enumeration (caller-indistinguishable by construction).
- Both world-delete entries (matches the project-wide delete convention).

Two did **not** stand: the replay-drop of an update to a since-deleted document was overturned as a
confirmed defect (NEW-2, promoted to `OPEN_BUGS.md`) — final-state convergence survives, but the
stale op is redacted against the wrong document's permission set, not simply dropped. The
four-`ignores`-array entry was overturned as a stale record: the lint-config shape it described no
longer exists. Both closures are recorded in `docs/POST_WORK_FINDINGS.md`. The third overturned
entry, PW19 itself, was the embedded positive control and was already tracked as a confirmed defect
before this pass ran; every resolved e2e flake class remains unaffected by this re-triage.

**Phase 1 disposition — PW19.** Confirmed and promoted to `OPEN_BUGS.md` in this phase, as scoped;
its ruled fix (snapshotting the relevant visibility into the event/command at commit time) is
Phase 1b's, not this phase's. `OPEN_BUGS.md` still carries the PW19 entry, unmodified from the
promoting commit, per this task's carry-forward instruction not to touch it. No further action in
Phase 1.

| Id | Item | Phase |
|---|---|---|
| NEW-3 | `SqliteRepository::apply_command`'s Create and Update branches did not call `validate_property_overrides` at all — only `apply_intent`'s two branches did, so the band classifier's ingress gate would not have bound the trusted undo/replay substrate. Structural classification, not a capability/schema/size check, and the same class the pre-existing `/engine` normalization gate was already extended to `apply_command` to close. | 1 — folded into the same task, DONE: `apply_command`'s Create and Update now both call `validate_property_overrides` alongside `apply_intent`'s two call sites, closing the gap at all four write paths. |
| NEW-4 | The comment-reference checker (`check-comment-refs.mjs`) reports clean both before and after a test name narrating an external incident by allusion ("the reported panic"), on the same instrument fingerprint — a detection gap, not instrument drift. History narration by allusion carries no fixed lexical marker the pattern can anchor on. | 7 — pairs with the existing item teaching the same checker to see a skill-name repo pointer; recorded in `docs/TODO.md`. Whoever builds it must positive-control against a known-violating name before trusting a green run, because a false positive is visible while a false negative is not, and that asymmetry is exactly what invites re-narrowing a widened detector. |
| NEW-5 | The repo-wide `property_overrides` key survey (verifying every constructed key already falls inside the classifier's whitelist) greps the literal field name, so a key built through a helper function is invisible to the method. The one such helper in the repo was hand-audited and found compliant, so OB2's closing conclusion holds — but the method itself has a blind spot a future survey of this shape would repeat. | Not phase-scoped (method note, no code defect); recorded in `docs/TODO.md` for the next survey of this shape. |

---

## 5. Phase design

Each phase is one branch, SDD-executed, per-task two-reviewer gate, whole-branch review before
merge, merged with `--no-ff` to local `main`. Pushing waits for the full sub-project.

### Phase 1 — Server: data, permissions, wire

**OB2 is the phase's spine and its shape is already decided by the user:** redaction operates on
content bands, never on the envelope. Three parts, in order.

1. **One shared classifier** in the permission module — the four redactable content bands plus a
   function mapping a pointer to either a whole-band target (nulled in place) or a within-band
   target (pointer strip, now provably landing inside untyped or optional data). Ingress and egress
   currently duplicate the judgement of what a pointer means, and this panic **is** what that fork
   looks like when it drifts. The change-delta path reads the same symbol, so it cannot diverge
   from whole-document egress either.
2. **Ingress rejects an unclassifiable pointer** at both existing call sites, so an envelope-naming
   override becomes a bad-path error rather than a stored landmine.
3. **The egress filter returns a result** and both panicking assertions are deleted. Callers fail
   **closed**: broadcast drops delivery to that recipient, and the read routes error rather than
   shipping a half-redacted document. The whitelist alone closes the reachable bug; the result type
   covers what a whitelist structurally cannot — a band added to the document type without
   updating the classifier.

No migration and no compatibility shim: no worlds or users exist, and every override key
constructed anywhere in the repo is already inside the whitelist.

Tests: per-pointer ingress rejection for each envelope field; acceptance for the four bands and
their nested forms; a regression test that the exact nested-permissions input errors instead of
panicking; and **a mutation check that removing a band from the shared list fails the suite** — a
parity test that passes because both paths are wrong the same way proves nothing.

TD26, TD27 and TD31 tighten the client wire boundary against its Rust source. TD26 and TD31 change
runtime accept/reject, which is why they were deferred out of documentation work and belong here
with real tests. PW19 is confirmed here — its buddy-check ran with this phase's plan — and its
promotion to `OPEN_BUGS.md` happens in this phase, but its ruled fix is scoped to Phase 1b (§3).
NEW-1's adversarial re-triage overturned a second entry sharing PW19's root cause — an `Update` to
a since-deleted document redacted against a reused id's new document rather than dropped (NEW-2) —
and it is promoted to `OPEN_BUGS.md` alongside PW19, with its fix scoped to Phase 1b as well.

### Phase 2 — Server: scene geometry, movement, vision

**PW1 and PW2 are one root cause with two symptoms**, and are fixed as one change: a
step-and-extent-to-world-distance conversion is *assumed* equal to the cell size instead of
*derived* from the grid shape. That assumption holds on square grids and is wrong on hex, where
adjacent centers sit a factor of about 1.73 apart and a bounds rectangle spans neither `w × size`
nor `h × size`.

The fix resolves the unit question first, because it decides which consumer is the defect: bounds
are authored by a GM in **grid units** and documented as such, so the navmesh builder's reading is
correct and the other consumer — which feeds the same value straight in alongside raw wall
coordinates as pixels — is wrong. Both then read **one shared symbol** that converts bounds to a
world-unit extent using the grid shape, and a second shared symbol supplies the per-step world
distance. Neither call site keeps its own conversion. Hex + continuous has thin coverage, which is
why PW3's non-GM end-to-end test lands in the same phase rather than later.

PW4 becomes buildable here for the first time: edge-projected environment light was specified and
implemented as flat ambient purely because the scene model had no boundary to project from. Scene
bounds now exist, so the constraint is gone and the specified behavior is built, with occlusion
already implemented.

TD17, TD18, TD19, TD48 and PW5 are contained changes in the same crate. PW31 is re-verified
against the shipped corner-drift fix **before** any new work: that fix may already subsume it, and
patching a symptom whose cause was fixed elsewhere is how a repeat failure gets hidden.

### Phase 3 — Server: ops, performance, asset staleness

**OB4's fix is a derivation, not a counter.** The stale-image bug exists because the cache-bust
value is a client-local counter incremented only by a frame that an ordinary reconnect is enough to
miss, and a missed frame leaves the serve URL byte-identical — so no request is issued, so the
entity tag is never revalidated. The long-term shape is to make the bust derive from the asset's
**authoritative version**: carry the version in the change frame and have the resolver fall back to
the version last seen in a document or asset listing, so a resync repairs it. Routing the frame
through the sequenced path would also fix it but costs a world sequence number per byte swap, which
that route is deliberately exempt from — so it is rejected.

TD9 and TD10 are performance fixes on the connect path and the single-writer pool; TD10 gives the
session store its own connection while leaving the write path's deliberate serialization untouched.
TD4a and TD5 are the server half of the ui-state work, kept with their siblings in Phase 4 by
ledger id so neither half can be forgotten. TD49 is a shortcode scanner refinement.

### Phase 4 — Client: shell, session, boot, ui-state

The largest cluster of contained defects. Three sub-groups:

**Session persistence** (TD4b, TD6, TD7, TD8) — an in-flight ordering guard replacing a
fire-and-forget leading edge, a logout reset that makes the write guard structural rather than
marker-based, exhaustive patch construction that turns a widened type into a compile error, and
client pruning that makes an over-cap blob recoverable.

**Boot and transport robustness** (TD12, TD13, TD14, TD15) — TD14 is the sharpest: an activation
rethrow escapes to an outer handler and skips member fetch, topology reconciliation, scene
re-subscription and the first-scene seed on **every** subsequent connection, not just the failing
one. Activation failure should degrade surfaces, not silently skip everything after it.

**Parity and duplication** (TD3, TD16, TD20, TD29, PW16, PW17) — TD20 is a real merge, not a swap:
the two implementations disagree on three axes and neither is a superset, so adopting either as-is
silently drops a property. The unified implementation carries encoding, the server's error text,
**and** the request timeout. PW17 is a trace-then-decide task: determine whether entering a world
is reachable without leaving one first; if it is not, the warning is a false positive and the
capture is documented as deliberate.

### Phase 5 — Client: modules, UI, render

The broadest phase; three of the five open bugs plus two new authoring features.

OB3's fix makes the two call sites agree on a coordinate frame — the fallback was written assuming
both points share one, and mixing a snapped anchor with a raw pointer defeats its own stated
purpose — then adds the extent guard its three sibling tools already carry. OB5 is a one-argument
correction plus the test that seeds the actually-broken value; the existing test seeds the one
value for which the bug is a no-op, and client tests mock dispatch, so they prove what is sent and
never what the server accepts. OB1 narrates the open op **when it changes placement**,
distinguishing that from a focus bump.

TD37 and TD38 are the two authoring features. TD38's render half already works, so the work is a
picker plus one concurrency-correct dispatch using the raw stored value as the pre-image —
the same convention whose absence caused OB5. TD37 adds the control and the wrap-aware
shortest-signed-delta lerp that the existing animator's raw-scalar tween needs.

PW13 and PW14 were originally scoped to this phase but have moved to the Nightfox repo's own
trackers, per §2.6: both are defects in Nightfox's own sheet component, not in Shadowcat's engine
or toolchain. This phase does not span two repositories.

### Phase 6 — Module toolchain

TD50/PW10 build the i18n registration seam: module-supplied catalog fragments merged per locale
with defined collision rules, after which the authoring guide and worked example stop instructing
authors around the gap. PW8 addresses the same class one layer down — registration cannot reach
session context, so every stateful module repeats a construct-at-mount dance; the resolution is a
session-scoped activation phase that carries it. TD39/TD40 build runtime module install/uninstall
and the two consequences that were parked behind it. PW11 promotes the orphaned document-type
constant to the engine barrel beside its sibling.

### Phase 7 — Tooling, gates, test infrastructure

**TD1 is the largest single item and is an audit, not a transcription.** Twenty-six sites, but two
dominant causes, so it is nearer two judgements than twenty-six: ten argument-count suppressions
usually removable by grouping arguments into the struct they already implicitly form, one
large-error suppression usually removable by boxing the variant, and fifteen client-side type
suppressions. Entries reading "constructor, threshold is arbitrary" would be a rubber stamp
reporting green forever — the same defect as an empty comment satisfying a documentation gate,
authored by the very task meant to close the hole. **A site with no honest, site-specific reason is
a fix or a proposal, never an entry.** The checker fails on four conditions including a
manifest-level lowering, which is the one surviving route around the gate, and is keyed by file and
item symbol rather than line number.

TD28 and TD30 close two blind spots in existing checkers. TD30 explicitly must **not** be closed by
widening the predicate to whole HTML files: a guide page with a fenced example would then fail the
build spuriously, and the obvious repair for that is narrowing the detector — which is what hides
real misses. TD44/PW9 and PW12 build the two missing harnesses. PW20 first establishes the actual
pattern-matching semantics against a harmless command shaped the same way, and only then widens.

### Phase 8 — Closeout

Documentation sync across all tracking files to empirically verified reality; the reviewed
skill-update gate over every `shadowcat-codebase-*` skill the campaign touched, with a spec
reviewer confirming each skill diff; the plugin version bump and marketplace refresh in each
consuming repo, without which a directory-sourced plugin serves its cached snapshot and a stale
copy is indistinguishable from a current one; a graph update; then merge, push, and watch CI to
green. **This phase cannot pass while any parked ledger item's unblocking phase has already
merged** (§2.4).

---

## 6. Design forks, resolved

Resolved here so no phase re-litigates one mid-execution. Each answers "what is the best long-term
shape in keeping with our plans and goals?".

| Fork | Resolution |
|---|---|
| **PW1** — which consumer of the scene bounds is the defect? | Bounds are grid units, as authored and documented. The consumer treating them as pixels is the defect. Both then read one shared conversion symbol so the units cannot fork again. |
| **TD22** — exclude an instance wholesale, or filter after computing its plan? | Filter **after** computing the plan, requiring the extra capability only when that instance's plan actually emits such a change. Wholesale exclusion is over-strict and drops instances whose plan touches nothing embedded; the precise form is cheap here because the plan is already computed immediately after. |
| **TD23** — wire the focus seam or delete it? | **Wire it.** The user-visible consequence is real: opening a sheet whose panel is open but scrolled out of view activates it in the tree while nothing raises it in the DOM. The stage-well guard that exists on only one adapter moves to the shared caller rather than being duplicated into both — duplicating it is the fork this codebase produces most. |
| **TD45** — give the fallback engine a panel menu, or leave the gap? | Give it the menu, from the same component. A fallback engine a panel cannot leave a zone from is a trap, and "production never reaches it" is exactly the reasoning that made OB1 a live accessibility bug. |
| **TD21** — is the unlabeled negative form intended? | Emit the labeled negative form. It is arithmetically identical and restores the breakdown chip. The notation-level choice was deliberate at that layer; what was never established is whether its downstream effect was considered — and it was not. Decided at the breakdown layer, with a server-side test. |
| **TD18** — re-wire or delete the caller-less predicate? | Delete it. Its stated justification is that it is one home for wall-crossing semantics, but the production path now reads a different symbol directly — so it is a **second** home for that decision with no callers, which is worse than none. |
| **OB4** — version-derived bust, or route through the sequenced path? | Version-derived. The sequenced route costs a world sequence number per byte swap, which the replace path is deliberately exempt from. |
| **PW15** — which ordering comment is wrong? | Determined by reading the **server's** emission sequence, not the client. Both guards are order-independent, so playback is correct either way — but exactly one comment is wrong, and a maintainer reasoning from it could remove the guard that is in fact load-bearing. |
| **TD30** — how to see an inline style block? | Parse out the style regions and test those. Applying the stylesheet predicate to whole HTML files fails documentation pages carrying fenced examples, and the obvious repair for that is narrowing the detector. |

---

## 7. Requires the user's ruling

One item where "what is the best long-term shape?" genuinely does not answer the question. It is
surfaced rather than decided, per §2.3.

**PW18 — the undiagnosed whole-suite e2e failure.** Fifteen of sixteen tests failed on an
unmodified commit and then passed on re-run, which establishes non-determinism and rules out a code
regression at that revision. Three candidate causes were tested and eliminated. The cause remains
unknown and was deliberately not guessed at; trace retention is now in place so the next occurrence
is diagnosable. This cannot be *fixed* without a reproduction. I propose a **bounded
investigation** of the leading untested candidate — contention against the single writer
connection, under a constrained-CPU repro — and, if that does not reproduce it, the item stays open
with its evidence capture rather than being closed on a green re-run. **Do not treat the green
re-run as evidence the cause is gone** is the entry's own standing instruction.

Also surfaced, not decided: TD33 is a per-machine action inside another program's UI and cannot be
closed by any agent (§4.3). TD32 moved to the Nightfox repo's own backlog (§2.6) and is no longer
this campaign's item to surface.

---

## 8. Execution model

**Dispatch.** Implementation goes to `shadowcat-coder`; every review checkpoint dispatches the
`shadowcat-spec-reviewer` + `shadowcat-code-reviewer` pair. Every dispatch specifies `effort`
explicitly — an unspecified effort silently inherits the session's. When a base agent reports
blocked, or a reviewer's findings read as shallow, the `-opus` twin is dispatched before the human
is. Reviewers have no shell by directive, so diffs are pre-generated to a file and gate outputs are
relayed to them.

**Reporting channel.** Stated in every first prompt: return the report as the Agent tool's result,
or send it via SendMessage, or write it to a named document. Campaign dispatches that must report
back are launched **without** a `name`, because naming an agent backgrounds it and its final text
reaches nobody.

**Verification.** Client: build, `pnpm -r test`, `pnpm -r typecheck`, lint. Server: `cargo test`,
`cargo fmt`, `cargo clippy`. A shared wire-schema change runs the full repo test gate, not a
filtered one. The client build precedes any server build — the embed validates its input at compile
time. Evidence before assertions: no phase reports green without the command output.

**Branch and merge.** One branch per phase, merged `--no-ff` to local `main` after its whole-branch
review. History is never rewritten. Only one agent edits a tree at a time; an agent is stood down
explicitly and its acknowledgement received before any replacement is dispatched onto its scope.

**Doc discipline.** Bugs never go in the to-do file; deferrals never go in the bug file; mid-run
anomalies go to the findings file. Comments cite symbols, never file names or line numbers, and
name nothing whose identity a process assigns.

---

## 9. Sub-projects 2 through 8

Built after Sub-project 1 merges, each with its own brainstorm → spec → plan → execute cycle, in
this order — dependency-light first, so the two carrying real design risk land last:

1. **Per-world export/import** — a world-scoped row subset preserving cross-key referential
   integrity and shared asset references.
2. **Dice-notation grammar growth** — math functions plus crit-event and tier-ladder syntax.
3. **In-body document-link chat segment** — needs a server producer and an authoring affordance;
   actor-name navigation already ships, a free-form link does not.
4. **Recalc-from-chat** — persisting roll provenance, which carries a persistence and secrecy fork.
5. **Per-channel and per-message dice-settings overrides** — a channel model exists in chat, so
   this is an override layer rather than a new subsystem.
6. **Speak-as-token-instance** — the wire variant is rejected at ingest with no first-party
   producer; the composer UX and lifting the rejection ship together, never separately.
7. **Link-preview extensions** — a fetch-cache-as-asset image pipeline, async post-publish
   enrichment, a shared preview cache, and provider embeds. The embed surface carries request
   forgery and privacy exposure and **must be threat-modelled** as part of its own design pass.

---

## 10. Definition of done

Sub-project 1 is complete when:

- Every ledger id in §4.1, §4.2 and §4.4 marked for a phase has a per-item disposition line
  recording what was done, with evidence.
- `OPEN_BUGS.md` contains no entry that is not validly blocked under §2.1.
- `TODO.md` contains only the seven validly-blocked items of §4.3, each naming its blocking phase,
  plus the one user-action item and the seven follow-on sub-projects.
- `POST_WORK_FINDINGS.md` contains no untriaged entry.
- NEW-1's adversarial pass over the provisionally-accepted findings batch (§4.4) has completed,
  and every finding it overturned has been folded into its owning phase.
- No parked new item (§2.4) has an unblocking phase that has already merged.
- Every affected `shadowcat-codebase-*` skill is updated and its diff reviewer-confirmed; the
  plugin version is bumped and refreshed in each consuming repo.
- The full gate is green on all three operating systems and CI is green after push.

The campaign as a whole is complete when Sub-projects 2 through 8 have each run their own cycle to
the same standard.
