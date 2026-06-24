# Shadowcat Codebase Skills & Agents — Design Spec

> **Date:** 2026-06-24
> **Status:** Approved design, ready for implementation planning (`writing-plans`).
> **Scope:** Project tooling — a family of project-scoped codebase-knowledge skills, a
> scoped activation hook, three project-specialized subagents, and the CLAUDE.md directives
> that bind them. NOT a product/engine feature. Does not touch `src/` runtime code.

## 1. Problem & Goal

Fresh agents (and fresh sessions) repeatedly re-derive how each Shadowcat subsystem works.
The repo already holds knowledge in three places, but none is an agent-facing, fast,
per-subsystem orientation layer:

- **graphify** (`graphify-out/`) — auto-extracted relationship graph (who-calls-what, file
  relationships). Great for "what connects to what," not for "what are the invariants here."
- **`docs/design/` + `docs/design/ARCHITECTURE.md`** — human-authored rationale and design
  decisions. Authoritative but long-form; not optimized for "get oriented in 30 seconds."
- **memory folder** (`~/.claude/projects/C--Dev-Shadowcat/memory/`) — cross-session lessons
  and resume state. Stylistic/continuity, not subsystem reference.

**Goal:** add a fourth, deliberately distinct layer — concise per-subsystem **orientation +
index** skills — plus the machinery to keep them current and to carry them across the
subagent boundary (subagents do not auto-activate skills).

### Non-goals
- Not duplicating graphify relationships or design-doc rationale — skills **point into** them.
- Not a replacement for superpowers skills — the agents **compose with** them.
- No changes to product/runtime code under `src/`.

## 2. Division of Labor (what makes each layer distinct)

| Layer | Answers | Form |
|---|---|---|
| **Codebase skills** (new) | "What is this subsystem, where are its seams, what must I not break, where do I look next?" | Short fixed-shape brief, agent-facing |
| graphify | "What calls/depends on what?" | Auto graph, queried |
| design docs | "Why was it built this way?" | Long-form rationale |
| memory | "What cross-session lessons/resume state apply?" | One-lesson-per-file |

A codebase skill that restates graphify edges or design-doc prose is a defect — it routes to
them instead.

## 3. The Skill Family

Project-scoped skills at `.claude/skills/<name>/SKILL.md`, checked into the repo. **Core + 6
coarse-subsystem skills**, all **fully authored at creation** (researched from graphify +
source + design docs). Coarse subsystems are cut by cohesion, not 1:1 with directories.

| Skill name | Covers | Path-hook globs |
|---|---|---|
| `shadowcat-codebase-core` | Architecture invariants, build/test commands, code-style & cross-platform rules, the doc↔memory↔graphify map, module/contribution model overview | *(none — description-activated; always-relevant base)* |
| `shadowcat-codebase-documents-permissions` | `src/server/src/data/` (document/permission/search), M5 redaction model, owner-aware `can_see`, `OwnerOrGm` tier, wire types + client Zod mirror | `src/server/src/data/**`, `src/client/core/src/wire.ts` |
| `shadowcat-codebase-actors-tokens` | Actor doc model, linked vs instanced tokens, `resolveTokenActor`/`EffectiveActor`, factions registry, name privacy | `src/modules/actors/**`, `src/modules/factions/**`, `src/client/core/src/actor.ts` |
| `shadowcat-codebase-scene-rendering` | Server scene ECS (derived read-model), client stage (Pixi backend), scene-tools, render-from-optimistic | `src/server/src/scene/**`, `src/modules/stage/**`, `src/modules/scene-tools/**` |
| `shadowcat-codebase-realtime-sync` | ws/http/auth, broadcast egress, client store + optimistic/rollback, search frames | `src/server/src/ws/**`, `src/server/src/http/**`, `src/server/src/auth/**` |
| `shadowcat-codebase-client-shell` | entry/core-ui/topbar/statusbar/settings, contribution + Surface architecture, hash router, i18n core | `src/modules/entry/**`, `src/modules/core-ui/**`, `src/modules/topbar/**`, `src/modules/statusbar/**`, `src/modules/settings/**` |
| `shadowcat-codebase-assets` | Asset module + server asset store, ETag=version, tiered upload limits, streaming-to-disk, out-of-band `AssetChanged` | `src/modules/assets/**`, server asset code |

New domains (effects, pathfinding, …) get their own skill when those milestones open; they
are **not** stubbed pre-emptively.

### 3.1 Skill body shape (fixed)

Every domain skill body follows the same sections, kept short:

1. **Purpose** — one paragraph: what the subsystem is and its responsibility boundary.
2. **Key files & seams** — the handful of files that matter and the interfaces between them.
3. **Hard invariants** — what must not break (e.g., "redaction is fail-closed,"
   "search index is visibility-partitioned"). Cite the memory slug / design doc where one exists.
4. **Gotchas** — non-obvious traps a fresh agent would hit.
5. **Pointers** — `graphify query "…"` / `graphify path "A" "B"` examples, relevant
   `docs/design/*.md`, and memory slugs. This section replaces re-derivation.

`shadowcat-codebase-core` additionally documents: build/test commands per workspace, the
cross-platform invariants summary, and the "which knowledge layer for which question" map.

### 3.2 Frontmatter & activation description

Standard `SKILL.md` frontmatter: `name`, `description`. The `description` is written for
strong task-level auto-match (the graphify skill's description is the reference quality bar):
lead with trigger conditions and subsystem nouns so the main-thread agent auto-activates it
when a task concerns that subsystem.

## 4. Activation (main-thread agent only)

Two complementary mechanisms; both only help the **main-thread** agent (subagents handled in §5):

1. **Description-match** — built-in; handles task-level relevance.
2. **Scoped path hook** — a new `PreToolUse` matcher on `Edit|Write` in `.claude/settings.json`.
   When the edited `file_path` falls under a subsystem's globs, it emits a one-line
   `additionalContext` reminder: *"You're editing <subsystem>; consider the
   `shadowcat-codebase-<x>` skill."*

### 4.1 Path-hook requirements

- **Tool scope:** `Edit|Write` (and `MultiEdit` if present) — **not** `Read`/`Glob`, to avoid
  noise on routine reads.
- **Per-session dedup:** fire **once per (session, subsystem)**. Implemented with a marker file
  keyed on the hook's `session_id` + subsystem id (e.g. under the OS temp dir). If the marker
  exists, stay silent; else create it and emit. This prevents repeat reminders on every edit.
- **Mapping:** a single source-of-truth map from glob → subsystem id, kept inside the hook
  script. Must stay in sync with §3's table (verification step covers this).
- **Shape:** same Python + `{"hookSpecificOutput":{"hookEventName":"PreToolUse",
  "additionalContext":"…"}}` pattern already used by the two graphify hooks. Must fail open
  (never block the tool call) and emit nothing on parse failure.
- **Portability:** temp-dir resolution and path normalization must work on Windows, macOS,
  Linux (the project's cross-platform invariant). Use Python `tempfile`/`os` rather than shell
  builtins; normalize backslashes like the existing graphify Read hook does.

## 5. The Three Agents

Agent definitions at `.claude/agents/<name>.md` (first agents in the repo), checked in.
Frontmatter: `name`, `description`, `tools`, optional `model` (omit → inherit). Body = system
prompt.

**The carrying mechanism:** subagents do not auto-activate skills, so each agent is granted the
**`Skill` tool** and its system prompt opens with a hard rule: *"Before any analysis or
edits, invoke the relevant `shadowcat-codebase-*` skill(s) for the files in scope (always
`shadowcat-codebase-core`, plus the subsystem skill)."* They **compose with** superpowers
skills rather than replacing them.

| Agent | `tools` | Responsibility | Composes with |
|---|---|---|---|
| `shadowcat-coder` | Read, Write, Edit, Bash, Glob, Grep, Skill (full implementation set) | Implement a scoped feature/task: invoke codebase skill(s) → follow `test-driven-development` + project CLAUDE.md → return a structured implementation report (files changed, tests added, commands run, open risks) | `subagent-driven-development`, `dispatching-parallel-agents`, `test-driven-development` |
| `shadowcat-spec-reviewer` | Read, Grep, Glob, Bash (tests only), Skill — **read-only, no Edit/Write** | Spec/plan compliance: completeness, nothing skipped/downgraded, matches intent. **Also** verifies codebase-skill diffs accurately capture implemented changes (§6). Returns findings only | `buddy-checking`, `mainline-plan-execution` review, `requesting-code-review` |
| `shadowcat-code-reviewer` | Read, Grep, Glob, Bash (tests only), Skill — **read-only** | Code quality: bugs, logic errors, security, project-convention adherence, simplification/reuse. Returns findings only | `requesting-code-review`, `buddy-checking` |

### 5.1 Agent return contracts
- **coder:** structured report — summary, files changed, tests added + result, lint/format
  status, any deviations flagged, residual risks. Its final message *is* the return value.
- **reviewers:** findings list with severity (Critical/Important/Minor), file:line, and a
  concrete recommendation each; explicit "no findings" when clean. No edits.

### 5.2 Skill-tool reachability (must verify)
Custom agents must actually be able to call `Skill`. The implementation includes a verification
step that dispatches each agent and confirms it invokes a `shadowcat-codebase-*` skill and the
skill content reaches it. If the harness does not expose `Skill` to subagents, fall back to the
agent prompt instructing it to **Read** the specific `SKILL.md` path(s) directly (still
project-scoped, still works) — this fallback is documented in the agent body regardless, so a
skill-tool gap degrades gracefully rather than silently dropping codebase context.

## 6. Self-Update Gate (reviewed)

A hard gate at the **same tier as the existing doc-sync gate** (CLAUDE.md Documentation
Standards §1). Three beats, in order, before any merge/clear:

1. **Update** — the mainline agent updates every touched `shadowcat-codebase-*` skill so it
   reflects new/changed seams, invariants, gotchas, or pointers.
2. **Review** — `shadowcat-spec-reviewer` verifies the skill diffs accurately capture the
   implemented changes: no omissions, no drift/hallucination, pointers still valid.
3. **Land** — only once the review passes.

**Triggers:** (a) completion of any plan (mainline-plan-execution or subagent-driven), and
(b) any inline/ad-hoc change that alters a seam, invariant, or gotcha — not only formal plans.
Trivial changes that touch no subsystem knowledge need no skill edit, but the agent must
consciously confirm that ("no skill update needed: change is internal to X with no seam/invariant
impact").

## 7. CLAUDE.md Additions

A new top-level section `## Codebase Skills & Agents`, placed near the `## graphify` section,
with two binding directives written in the project's existing ❌Bad/✅Good style:

1. **Reviewed self-update gate** (§6) — mandatory final step, doc-sync tier, blocks merge/clear;
   update → spec-reviewer verifies → land.
2. **Agent dispatch** — when delegating implementation to a subagent → `shadowcat-coder`; at any
   review checkpoint (buddy-check, `requesting-code-review`, or the mainline-plan-execution final
   review) → dispatch `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` as the two-reviewer
   pair. Each dispatched agent is reminded that it must invoke the relevant codebase skill first.

The section also notes that the path hook + descriptions handle main-thread activation, so the
directives focus on the subagent and update-gate behavior the harness cannot do automatically.

## 8. Verification Plan

1. Each of the 7 skills exists, has valid frontmatter, follows the fixed body shape, and its
   pointers resolve (graphify queries run; design-doc paths and memory slugs exist).
2. Path hook: editing a file under each subsystem's globs emits exactly one reminder for that
   subsystem per session and stays silent on the second edit; editing an unmapped file and any
   `Read` emit nothing; hook never blocks a tool call; works with normalized Windows paths.
3. Each agent dispatches, invokes (or reads) the correct codebase skill, and honors its
   tool boundary (reviewers cannot Edit/Write).
4. CLAUDE.md section is present and internally consistent with the agent/skill names.
5. The glob→subsystem map in the hook matches §3's table exactly (no orphan/!missing globs).

## 9. Decomposition Hint (for writing-plans)

Suggested task order (skills before the machinery that references them):
1. Author `shadowcat-codebase-core`.
2. Author the 6 domain skills (each: research via graphify+source+design docs, then write).
3. Add the scoped `Edit|Write` path hook + glob→subsystem map to `.claude/settings.json`.
4. Author the 3 agents in `.claude/agents/`, including the Skill-tool + read/Read fallback.
5. Add the `## Codebase Skills & Agents` section to CLAUDE.md.
6. Verification pass (§8), including the live skill-tool-reachability check (§5.2).

## 10. Decision Log

| # | Decision | Choice |
|---|---|---|
| 1 | Skill role vs existing layers | Orientation + index; point into graphify/docs, never duplicate |
| 2 | Taxonomy / granularity | Coarse subsystems; core + 6 seeded; grow on demand |
| 3 | Activation (main thread) | Description-match + scoped `Edit\|Write` path hook, per-session deduped |
| 4 | Self-update discipline | Mainline hard gate at plan completion + substantial inline changes |
| 5 | Self-update review | Update step is itself reviewed by `shadowcat-spec-reviewer` before landing |
| 6 | Agent scope/boundaries | Specialized wrappers; coder RW, both reviewers read-only; compose with superpowers |
| 7 | Agent skill carrying | Granted `Skill` tool + "invoke codebase skill first"; Read-SKILL.md fallback if unavailable |
| 8 | Seeding depth | Fully author all 7 now |
| 9 | "titan" terminology | Slip for the Shadowcat codebase skills; family is `shadowcat-codebase-*` |
