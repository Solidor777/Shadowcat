# Codebase Skills & Agents Implementation Plan

> **For agentic workers:** This plan is executed via the project's `mainline-plan-execution`
> directive (long-context frontier model): implement task-by-task in this session with an
> inline spec-compliance check per task and a buddy-check of the full branch before merge.
> Steps use checkbox (`- [ ]`) syntax for tracking. Source spec:
> `docs/superpowers/specs/2026-06-24-shadowcat-codebase-skills-design.md`.

**Goal:** Build a project-scoped codebase-knowledge layer — 7 orientation+index skills, a
scoped activation hook, three project-specialized subagents — and the CLAUDE.md directives
that keep them current and dispatch them.

**Architecture:** Skills are concise per-subsystem briefs (`.claude/skills/<name>/SKILL.md`)
that point INTO graphify/design-docs/memory rather than duplicating them. A `PreToolUse`
`Edit|Write` hook (`.claude/hooks/codebase-skill-reminder.py`, deduped per session) reminds the
main-thread agent which skill is relevant. Three agents (`.claude/agents/*.md`) carry skills
across the subagent boundary via the `Skill` tool. CLAUDE.md binds a reviewed self-update gate
and an agent-dispatch directive.

**Tech Stack:** Markdown (skills/agents/docs), Python 3 (hook, mirrors existing graphify hooks),
JSON (`.claude/settings.json`). No `src/` runtime code changes.

## Global Constraints

- **No `src/` changes.** This is tooling only. Touch only `.claude/`, `docs/`, and `CLAUDE.md`.
- **Skills are orientation+index, never duplication.** Every skill body uses the fixed shape
  (Purpose / Key files & seams / Hard invariants / Gotchas / Pointers) and routes to graphify,
  `docs/design/*`, and memory slugs instead of restating them. (Spec §3.1)
- **Cross-platform.** The hook must run on Windows, macOS, Linux: use Python `tempfile`/`os`,
  normalize backslashes (`.replace(chr(92),'/')`), never shell builtins. (Spec §4.1)
- **Hook fails open.** It must never block a tool call and must emit nothing on any parse error.
- **Reviewers are read-only.** `shadowcat-spec-reviewer` and `shadowcat-code-reviewer` get no
  `Edit`/`Write`/`MultiEdit`. (Spec §5)
- **Each agent invokes the codebase skill first.** Granted the `Skill` tool + a hard opening
  rule; documented `Read`-the-`SKILL.md` fallback if `Skill` is unavailable to subagents. (§5.2)
- **Naming:** skill family is `shadowcat-codebase-*`; agents are `shadowcat-coder`,
  `shadowcat-spec-reviewer`, `shadowcat-code-reviewer`. (Spec §10 #9)
- **Branch:** `codebase-skills-agents` (already created; spec committed at `8ff1ea7`).

---

## Skill-authoring task shape (applies to Tasks 1–7)

Authoring a knowledge skill is a research-and-write task, not a code TDD cycle. Each skill task
therefore uses this adapted cycle (no failing-unit-test step; the "test" is structural
validation):

1. **Research** — read the listed key files; run the listed `graphify` queries; skim cited
   design docs / memory slugs. Capture only what a fresh agent needs to get oriented.
2. **Write** `SKILL.md` — valid frontmatter (`name`, `description`) + the five fixed sections.
   The `description` leads with trigger conditions + subsystem nouns (match the graphify skill's
   quality bar). The **Pointers** section must include the listed graphify queries and resolve
   to the listed real design-doc paths / memory slugs.
3. **Validate** — run the structural check (frontmatter present, all five `## ` sections
   present, every pointer path exists). Shown once here; reused per task:

```bash
S=.claude/skills/<name>/SKILL.md
grep -q '^name:' "$S" && grep -q '^description:' "$S" && echo "frontmatter OK"
for h in Purpose "Key files" "Hard invariants" Gotchas Pointers; do
  grep -qi "## .*$h" "$S" && echo "section OK: $h" || echo "MISSING SECTION: $h"
done
# Every docs/ or memory path referenced must exist:
grep -oE 'docs/design/[A-Za-z0-9_.-]+\.md' "$S" | sort -u | while read p; do test -f "$p" && echo "ptr OK: $p" || echo "BROKEN PTR: $p"; done
```

4. **Commit** the single skill.

> **Invariant content rule:** "Hard invariants" must cite its source memory slug or design-doc
> section in brackets, e.g. `[[search-index-must-be-visibility-partitioned]]`. An uncited
> invariant is incomplete (mirrors the project's comment-citation rule).

---

## Task 1: `shadowcat-codebase-core` skill

**Files:**
- Create: `.claude/skills/shadowcat-codebase-core/SKILL.md`

**Interfaces:**
- Produces: the always-relevant base skill every agent invokes. No path-hook glob (description-
  activated). Later tasks (8 hook, 9–11 agents, CLAUDE.md) reference its name verbatim.

- [ ] **Step 1: Research**

Read: `docs/design/ARCHITECTURE.md`, `CLAUDE.md` (reference-docs table + cross-platform +
commenting rules), `package.json` / workspace config (top level), `Cargo.toml`.
Run: `graphify query "overall architecture: client modules, server, types, build"` and
`graphify explain "contribution surface module"`.

- [ ] **Step 2: Write `SKILL.md`**

Frontmatter `description` must trigger on: "how does Shadowcat build/test", "project
conventions", "architecture overview", "which knowledge layer", "module/contribution model".
Body (five sections) must contain, at minimum:
- **Purpose:** Shadowcat = modular open-source VTT; Rust server + Svelte 5 (Runes) client + SCSS;
  source strictly under `src/`, build output `dist/`.
- **Key files & seams:** `src/{client,server,modules,types}` layout; client = `core`/`render`/
  `shell`/`ui-kit`; modules = contribution packages; `src/types/generated` = ts-rs output.
- **Hard invariants:** cross-platform-from-day-one (CI matrix; portable paths) [CLAUDE.md
  Cross-Platform]; `dist/` must be built before any `cargo` build of the server
  [[embed-dist-compile-ordering]]; capability/permission model [[capability-permissions]].
- **Gotchas:** `CLAUDE.md` is git-ignored — durable rules live in ARCHITECTURE.md §2
  [[claude-md-is-git-ignored]]; ts-rs types are generated, edit the Rust source not the `.ts`.
- **Pointers:** the **knowledge-layer map** (skills = orientation; graphify = relationships;
  `docs/design/` = rationale; memory = lessons) + exact build/test commands per workspace
  (`pnpm --filter @shadowcat/ui build`, `cargo test`, `cargo fmt`/`clippy`) + `docs/design/ARCHITECTURE.md`.

- [ ] **Step 3: Validate** (run the structural check above with `<name>`=`shadowcat-codebase-core`)

- [ ] **Step 4: Commit**

```bash
git add .claude/skills/shadowcat-codebase-core/SKILL.md
git commit -m "feat(skills): shadowcat-codebase-core orientation skill"
```

---

## Task 2: `shadowcat-codebase-documents-permissions` skill

**Files:**
- Create: `.claude/skills/shadowcat-codebase-documents-permissions/SKILL.md`

**Interfaces:**
- Produces: subsystem skill; path-hook globs `src/server/src/data/**`, `src/client/core/src/wire.ts`.

- [ ] **Step 1: Research**

Read: `src/server/src/data/{document,permission,search,validation,repository}.rs`,
`src/client/core/src/wire.ts`, `docs/design/M2-data-foundation.md`.
Run: `graphify query "document permissions redaction filter_properties can_see"` and
`graphify path "permission.rs" "search.rs"`.

- [ ] **Step 2: Write `SKILL.md`**

`description` triggers on: documents, permissions, redaction, visibility, `OwnerOrGm`, wire
types, Zod schema, access control. Body must contain:
- **Purpose:** the document data model + capability/permission redaction layer (server source of
  truth) and its client wire/Zod mirror.
- **Key files & seams:** `document.rs` (model), `permission.rs` (`resolve_access`, `Access`,
  `is_owner`/`can_see`, `filter_properties`/`redact_change`, `OwnerOrGm` tier), `search.rs`
  (visibility-partitioned index), `repository.rs`, `validation.rs`; client `wire.ts` Zod mirror;
  ts-rs `Visibility` type.
- **Hard invariants:** redaction is **fail-closed** and owner-aware; the search index is
  **visibility-partitioned** — never redact only the returned doc
  [[search-index-must-be-visibility-partitioned]]; path-prefix authz covers ancestor +
  whole-doc Create [[path-prefix-authz-covers-ancestor-and-create]]; check-then-act across two
  queries needs one transaction [[two-query-guard-needs-tx]]; `INSERT…ON CONFLICT(id)` on a
  mutated id duplicates rather than moves [[upsert-on-conflict-duplicates-not-moves]].
- **Gotchas:** wire types are generated via ts-rs — change the Rust enum, regenerate, then mirror
  in Zod (drift guard); embedded copies need deep clone [[embedded-copy-needs-deep-clone]].
- **Pointers:** `docs/design/M2-data-foundation.md`, the graphify queries above,
  [[document-inheritance-merge-model]].

- [ ] **Step 3: Validate** (structural check, `<name>`=`shadowcat-codebase-documents-permissions`)

- [ ] **Step 4: Commit**

```bash
git add .claude/skills/shadowcat-codebase-documents-permissions/SKILL.md
git commit -m "feat(skills): documents-permissions subsystem skill"
```

---

## Task 3: `shadowcat-codebase-actors-tokens` skill

**Files:**
- Create: `.claude/skills/shadowcat-codebase-actors-tokens/SKILL.md`

**Interfaces:**
- Produces: subsystem skill; globs `src/modules/actors/**`, `src/modules/factions/**`,
  `src/client/core/src/actor.ts`.

- [ ] **Step 1: Research**

Read: `src/modules/actors/src/*`, `src/modules/factions/src/*`, `src/client/core/src/actor.ts`,
`src/client/core/src/scene-docs.ts`.
Run: `graphify query "actor token linked instanced resolveTokenActor EffectiveActor faction"`.

- [ ] **Step 2: Write `SKILL.md`**

`description` triggers on: actors, tokens, linked vs instanced, factions, token visual, name
privacy. Body must contain:
- **Purpose:** the Actor document + token model (place/resolve), factions registry, name privacy.
- **Key files & seams:** `actor.ts` (`resolveTokenActor`→`EffectiveActor`), token linkage
  (`token.system.actor_id` + `overrides` vs `embedded.actor[0]` + `source`),
  `buildTokenFromActor`, `module-actors` (create/list/pick), `module-factions` (editor + seed),
  faction registry config-doc `{name,color,stance}`.
- **Hard invariants:** instanced token's embedded actor copy needs `structuredClone`, not `{...}`
  [[embedded-copy-needs-deep-clone]]; registries are config-documents; name privacy rides the
  `OwnerOrGm` tier + fail-closed redaction (see `shadowcat-codebase-documents-permissions`).
- **Gotchas:** linked vs instanced provenance divergence; instanced re-sync is deferred
  [[document-inheritance-merge-model]].
- **Pointers:** [[token-architecture-forward-looking]], the M10 tokens design spec, the graphify
  query above.

- [ ] **Step 3: Validate** (structural check)

- [ ] **Step 4: Commit**

```bash
git add .claude/skills/shadowcat-codebase-actors-tokens/SKILL.md
git commit -m "feat(skills): actors-tokens subsystem skill"
```

---

## Task 4: `shadowcat-codebase-scene-rendering` skill

**Files:**
- Create: `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`

**Interfaces:**
- Produces: subsystem skill; globs `src/server/src/scene/**`, `src/modules/stage/**`,
  `src/modules/scene-tools/**`, `src/client/render/**`.

- [ ] **Step 1: Research**

Read: `src/server/src/scene/{mod,vision,explored}.rs`, `src/modules/stage/src/*`,
`src/modules/scene-tools/src/*`, `src/client/render/src/*` (top-level).
Run: `graphify query "scene ECS derived read-model vision fog stage pixi render tokens"`.

- [ ] **Step 2: Write `SKILL.md`**

`description` triggers on: scene, ECS, rendering, stage, Pixi, vision, fog, scene-tools, canvas.
Body must contain:
- **Purpose:** server scene ECS (derived read-model) + client stage (Pixi backend) + scene-tools
  interactive loop + vision/fog.
- **Key files & seams:** server `scene/mod.rs` (SceneEcs/derived frames), `vision.rs`
  (raycasting), `explored.rs` (fog); client `stage` (render-from-optimistic), `scene-tools`
  (place/select/move/draw/template/measure/ping); `SceneDerived` dispatch + egress.
- **Hard invariants:** canvas renders the **optimistic** view (`AppContext.documents`), not the
  authoritative store [[render-from-optimistic-view]]; fog is the secrecy gate — **fail closed**,
  hide-everything on missing/garbled signal; container-local coords must be tagged + filtered to
  the active container [[fog-is-the-secrecy-gate-fail-closed]]; bound recursive walks over
  self-FK tables with a visited-set [[m8a-execution-state]].
- **Gotchas:** scene auto-create on GM entry [[scene-lifecycle-gap]]; movement-blocking is
  server-authoritative [[m9-progress]].
- **Pointers:** [[m8-brainstorm]], [[m8d-2-scene-tools]], the graphify query above.

- [ ] **Step 3: Validate** (structural check)

- [ ] **Step 4: Commit**

```bash
git add .claude/skills/shadowcat-codebase-scene-rendering/SKILL.md
git commit -m "feat(skills): scene-rendering subsystem skill"
```

---

## Task 5: `shadowcat-codebase-realtime-sync` skill

**Files:**
- Create: `.claude/skills/shadowcat-codebase-realtime-sync/SKILL.md`

**Interfaces:**
- Produces: subsystem skill; globs `src/server/src/ws/**`, `src/server/src/http/**`,
  `src/server/src/auth/**`.

- [ ] **Step 1: Research**

Read: `src/server/src/ws/{room,conn,protocol,time}.rs`, `src/server/src/http/{routes,mod}.rs`,
`src/server/src/auth/{session,role,password}.rs`, `src/client/core/src/*` (store/optimistic).
Run: `graphify query "websocket room broadcast egress optimistic rollback store session auth"`.

- [ ] **Step 2: Write `SKILL.md`**

`description` triggers on: websocket, realtime, broadcast, room, sync, optimistic, rollback,
store, session, auth, login. Body must contain:
- **Purpose:** realtime transport (ws/http/auth) + client store with optimistic/rollback.
- **Key files & seams:** `ws/room.rs` (broadcast), `ws/conn.rs`, `ws/protocol.rs` (frames),
  `http/routes.rs`, `auth/session.rs`; client store (Zod-validated, optimistic apply +
  rollback base).
- **Hard invariants:** socket-buffer backpressure is non-portable — test the generic egress sink
  with a credit-gated Sink, not real-socket TCP backpressure
  [[socket-buffer-backpressure-nonportable]]; debounce on the leading edge, arm only when idle
  [[debounce-leading-edge-not-trailing-rearm]]; two-query guards need one transaction
  [[two-query-guard-needs-tx]].
- **Gotchas:** the client renders the optimistic view; `appliedSeq` watermark identity
  [[render-from-optimistic-view]]; live search rides the broadcast [[m6c-2-live-search]].
- **Pointers:** [[m6a-client-core]], [[m6c-1-search]], the graphify query above.

- [ ] **Step 3: Validate** (structural check)

- [ ] **Step 4: Commit**

```bash
git add .claude/skills/shadowcat-codebase-realtime-sync/SKILL.md
git commit -m "feat(skills): realtime-sync subsystem skill"
```

---

## Task 6: `shadowcat-codebase-client-shell` skill

**Files:**
- Create: `.claude/skills/shadowcat-codebase-client-shell/SKILL.md`

**Interfaces:**
- Produces: subsystem skill; globs `src/modules/entry/**`, `src/modules/core-ui/**`,
  `src/modules/topbar/**`, `src/modules/statusbar/**`, `src/modules/settings/**`,
  `src/client/shell/**`, `src/client/ui-kit/**`.

- [ ] **Step 1: Research**

Read: `src/modules/{entry,core-ui,topbar,statusbar,settings}/src/*`, `src/client/shell/src/*`,
`src/client/ui-kit/src/*` (locales, surface adapter).
Run: `graphify query "contribution registry surface appContext shell router i18n locale panel"`.

- [ ] **Step 2: Write `SKILL.md`**

`description` triggers on: UI shell, modules, contribution, Surface, panels, router, i18n,
locale, entry views, app bootstrap. Body must contain:
- **Purpose:** the SPA shell + UI-as-modules contribution architecture + i18n.
- **Key files & seams:** `ContributionRegistry`, manifest `provides`/`requires` + contract
  resolution, `<Surface>` adapter + `AppContext`, hash router + entry views (plain-routed, not
  contributions), neutral I18n core + Svelte `t`/`locale` adapter, region surfaces + panels.
- **Hard invariants:** a value put into `setContext` must be a stable in-place-mutated ref
  (e.g. `SvelteMap`), not a reassigned `$state` [[svelte-context-stable-ref]]; contribute/activate
  before any `await` that gates host mount [[refactor-async-contribution-paint-timing]]; entry
  views are plain-routed, surfaces are in-world only.
- **Gotchas:** i18n MUST stay framework-neutral (hand-rolled core, not svelte-i18n); UI packaging
  target = swappable entry package + per-element packages + thin shell [[ui-packaging-target]].
- **Pointers:** [[m7-brainstorm]], [[m6b-modules-capabilities]], the graphify query above.

- [ ] **Step 3: Validate** (structural check)

- [ ] **Step 4: Commit**

```bash
git add .claude/skills/shadowcat-codebase-client-shell/SKILL.md
git commit -m "feat(skills): client-shell subsystem skill"
```

---

## Task 7: `shadowcat-codebase-assets` skill

**Files:**
- Create: `.claude/skills/shadowcat-codebase-assets/SKILL.md`

**Interfaces:**
- Produces: subsystem skill; globs `src/modules/assets/**`, `src/server/src/data/asset.rs`,
  `src/server/src/http/assets.rs`.

- [ ] **Step 1: Research**

Read: `src/modules/assets/src/*`, `src/server/src/data/asset.rs`, `src/server/src/http/assets.rs`.
Run: `graphify query "asset upload store ETag version AssetChanged streaming limit"`.

- [ ] **Step 2: Write `SKILL.md`**

`description` triggers on: assets, upload, image, file store, ETag, asset version. Body must
contain:
- **Purpose:** asset upload/store + serving (server) and the client asset panel.
- **Key files & seams:** `data/asset.rs` (metadata row), `http/assets.rs` (serve, ETag),
  `module-assets` panel; stable-UUID replace/delete; out-of-band `AssetChanged`.
- **Hard invariants:** commit the source-of-truth/cache-key row **before** swapping the file —
  the inverse strands new bytes under a stale ETag/version (silent 304)
  [[commit-db-row-before-swapping-file]]; ETag == version; tiered configurable upload limits
  (GM 2× regular); stream uploads to disk.
- **Gotchas:** byte-swap replace keeps the stable UUID so links survive.
- **Pointers:** [[m8b-assets]], the graphify query above.

- [ ] **Step 3: Validate** (structural check)

- [ ] **Step 4: Commit**

```bash
git add .claude/skills/shadowcat-codebase-assets/SKILL.md
git commit -m "feat(skills): assets subsystem skill"
```

---

## Task 8: Scoped `Edit|Write` activation hook

**Files:**
- Create: `.claude/hooks/codebase-skill-reminder.py`
- Modify: `.claude/settings.json` (add a `PreToolUse` matcher for `Edit|Write|MultiEdit`)
- Test: `.claude/hooks/test-codebase-skill-reminder.sh` (committed self-test)

**Interfaces:**
- Consumes: the 6 domain skill names + their globs from Tasks 2–7.
- Produces: per-session-deduped subsystem reminders to the main-thread agent. No interface other
  tasks consume.

- [ ] **Step 1: Write the failing test**

Create `.claude/hooks/test-codebase-skill-reminder.sh`:

```bash
#!/usr/bin/env bash
# Self-test for the codebase-skill reminder hook. Exits non-zero on any failure.
set -u
H="python3 .claude/hooks/codebase-skill-reminder.py"
SID="testsession-$$"
mk() { printf '{"session_id":"%s","tool_name":"Edit","tool_input":{"file_path":"%s"}}' "$SID" "$1"; }

# 1) First edit under data/ => emits documents-permissions reminder
out1=$(mk "src/server/src/data/permission.rs" | $H)
echo "$out1" | grep -q "shadowcat-codebase-documents-permissions" || { echo "FAIL: no reminder on first data edit"; exit 1; }

# 2) Second edit same subsystem same session => silent (deduped)
out2=$(mk "src/server/src/data/document.rs" | $H)
[ -z "$out2" ] || { echo "FAIL: not deduped on second data edit: $out2"; exit 1; }

# 3) Unmapped path => silent
out3=$(mk "README.md" | $H)
[ -z "$out3" ] || { echo "FAIL: emitted for unmapped path"; exit 1; }

# 4) Read tool is never matched by settings, but script must also be silent if invoked
out4=$(printf '{"session_id":"%s","tool_name":"Read","tool_input":{"file_path":"src/server/src/data/permission.rs"}}' "${SID}b" | $H)
# (script keys only on file_path+session, so a fresh session WOULD emit; this asserts the script
#  itself doesn't crash and produces valid-or-empty output)
echo "$out4" | python3 -c "import sys,json; s=sys.stdin.read().strip(); json.loads(s) if s else None" || { echo "FAIL: non-JSON output"; exit 1; }

# 5) Malformed input => silent, exit 0 (fail open)
out5=$(echo 'not json' | $H) ; rc=$?
{ [ -z "$out5" ] && [ $rc -eq 0 ]; } || { echo "FAIL: not fail-open on garbage"; exit 1; }

echo "ALL HOOK TESTS PASS"
```

- [ ] **Step 2: Run it to verify it fails**

Run: `bash .claude/hooks/test-codebase-skill-reminder.sh`
Expected: FAIL (script does not exist yet) — e.g. `can't open file ... codebase-skill-reminder.py`.

- [ ] **Step 3: Write the hook script**

Create `.claude/hooks/codebase-skill-reminder.py`:

```python
#!/usr/bin/env python3
"""PreToolUse(Edit|Write|MultiEdit): remind the agent which shadowcat-codebase-* skill
covers the file being edited. Deduped once per (session, subsystem). Fails open."""
import sys, json, os, tempfile, re

# Ordered most-specific -> general. First match wins. Globs expressed as path substrings/regex.
SUBSYSTEMS = [
    ("documents-permissions", [r"src/server/src/data/", r"src/client/core/src/wire\.ts"]),
    ("assets",               [r"src/modules/assets/", r"src/server/src/data/asset\.rs", r"src/server/src/http/assets\.rs"]),
    ("actors-tokens",        [r"src/modules/actors/", r"src/modules/factions/", r"src/client/core/src/actor\.ts"]),
    ("scene-rendering",      [r"src/server/src/scene/", r"src/modules/stage/", r"src/modules/scene-tools/", r"src/client/render/"]),
    ("realtime-sync",        [r"src/server/src/ws/", r"src/server/src/http/", r"src/server/src/auth/"]),
    ("client-shell",         [r"src/modules/entry/", r"src/modules/core-ui/", r"src/modules/topbar/", r"src/modules/statusbar/", r"src/modules/settings/", r"src/client/shell/", r"src/client/ui-kit/"]),
]

def main():
    try:
        raw = sys.stdin.read()
        d = json.loads(raw)
    except Exception:
        return  # fail open
    t = d.get("tool_input", d) or {}
    path = str(t.get("file_path") or "").replace(chr(92), "/").lower()
    session = str(d.get("session_id") or "nosession")
    if not path:
        return
    sub = None
    for name, pats in SUBSYSTEMS:
        if any(re.search(p.lower(), path) for p in pats):
            sub = name
            break
    if sub is None:
        return
    # Per-(session, subsystem) dedup via marker file.
    try:
        mdir = os.path.join(tempfile.gettempdir(), "shadowcat-skill-markers")
        os.makedirs(mdir, exist_ok=True)
        marker = os.path.join(mdir, "%s-%s.seen" % (re.sub(r"[^A-Za-z0-9_.-]", "_", session), sub))
        if os.path.exists(marker):
            return
        with open(marker, "w") as f:
            f.write("1")
    except Exception:
        pass  # if dedup bookkeeping fails, still emit (better a repeat than silence)
    skill = "shadowcat-codebase-%s" % sub
    msg = ("You are editing the %s subsystem. Consider invoking the `%s` skill "
           "(plus `shadowcat-codebase-core`) for invariants and pointers before changing it."
           % (sub, skill))
    print(json.dumps({"hookSpecificOutput": {
        "hookEventName": "PreToolUse", "additionalContext": msg}}))

if __name__ == "__main__":
    main()
```

- [ ] **Step 4: Wire it into `.claude/settings.json`**

Add this object to the existing `hooks.PreToolUse` array (alongside the two graphify matchers):

```json
{
  "matcher": "Edit|Write|MultiEdit",
  "hooks": [
    { "type": "command", "command": "python3 .claude/hooks/codebase-skill-reminder.py" }
  ]
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `bash .claude/hooks/test-codebase-skill-reminder.sh`
Expected: `ALL HOOK TESTS PASS`.

- [ ] **Step 6: Commit**

```bash
git add .claude/hooks/codebase-skill-reminder.py .claude/hooks/test-codebase-skill-reminder.sh .claude/settings.json
git commit -m "feat(hooks): scoped Edit/Write codebase-skill reminder (per-session deduped)"
```

---

## Task 9: `shadowcat-coder` agent

**Files:**
- Create: `.claude/agents/shadowcat-coder.md`

**Interfaces:**
- Produces: an implementation subagent dispatched by `subagent-driven-development` /
  `dispatching-parallel-agents`. Returns a structured implementation report (its final message).

- [ ] **Step 1: Write the agent file**

Create `.claude/agents/shadowcat-coder.md`:

```markdown
---
name: shadowcat-coder
description: Implement a scoped Shadowcat feature or plan task. Dispatch as the implementation subagent when delegating coding work. Invokes the relevant shadowcat-codebase-* skill first, follows TDD and the project CLAUDE.md, returns a structured implementation report.
tools: Read, Write, Edit, Bash, Glob, Grep, Skill
---

You implement a single scoped task in the Shadowcat codebase.

HARD FIRST STEP — codebase context (subagents do not auto-activate skills):
1. Invoke `shadowcat-codebase-core` via the Skill tool.
2. Invoke the subsystem skill(s) for the files in scope (e.g. `shadowcat-codebase-documents-permissions`
   for `src/server/src/data/**`). If you are unsure which, invoke core and pick from its map.
   FALLBACK: if the Skill tool is unavailable to you, `Read` the file(s) directly at
   `.claude/skills/<name>/SKILL.md`. Never proceed without this context.

Then implement:
- Follow Test-Driven Development: write the failing test, see it fail, minimal implementation,
  see it pass. Commit in small logical units.
- Obey the project `CLAUDE.md`: cross-platform code, portable paths, no debug code in release,
  citation-style comments, immutable git history.
- Honor every invariant the codebase skill listed for the subsystem you touched.
- If you change a seam/invariant/gotcha, note it so the dispatcher can update the codebase skill.

RETURN (your final message IS the structured report, not a human chat):
- Summary (1-2 lines)
- Files changed (path — what)
- Tests added + result (command + pass/fail)
- Lint/format/typecheck status
- Deviations from the task spec (or "none")
- Residual risks / skill-update notes (or "none")
```

- [ ] **Step 2: Validate frontmatter + tool set**

Run:
```bash
grep -q '^name: shadowcat-coder' .claude/agents/shadowcat-coder.md && \
grep -q 'tools:.*Skill' .claude/agents/shadowcat-coder.md && \
grep -q 'tools:.*Edit' .claude/agents/shadowcat-coder.md && echo "coder frontmatter OK"
```
Expected: `coder frontmatter OK`.

- [ ] **Step 3: Commit**

```bash
git add .claude/agents/shadowcat-coder.md
git commit -m "feat(agents): shadowcat-coder implementation subagent"
```

---

## Task 10: `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` agents

**Files:**
- Create: `.claude/agents/shadowcat-spec-reviewer.md`
- Create: `.claude/agents/shadowcat-code-reviewer.md`

**Interfaces:**
- Consumes: same Skill-first contract as Task 9.
- Produces: two read-only review subagents; the spec-reviewer additionally reviews codebase-skill
  diffs for the self-update gate (Task 11 / spec §6). Both return findings-only.

- [ ] **Step 1: Write `shadowcat-spec-reviewer.md`**

```markdown
---
name: shadowcat-spec-reviewer
description: Read-only review of whether an implementation matches its spec/plan — completeness, nothing skipped or downgraded, intent honored. Also verifies codebase-skill update diffs accurately capture implemented changes. Dispatch at review checkpoints (buddy-check, mainline-plan-execution final review). Returns findings only; never edits.
tools: Read, Grep, Glob, Bash, Skill
---

You verify that completed work matches its spec/plan. You are READ-ONLY: you have no Edit/Write.

HARD FIRST STEP: invoke `shadowcat-codebase-core` + the relevant subsystem skill(s) via the
Skill tool (FALLBACK: `Read` `.claude/skills/<name>/SKILL.md`). Use them as the bar for
subsystem invariants.

Check, against the spec/plan you were given:
- Completeness: every required task/requirement implemented; nothing silently skipped,
  downgraded, or re-scoped (project CLAUDE.md forbids unilateral re-scoping).
- Intent: behavior matches what the spec asked for, not just what compiles.
- Invariants: no listed subsystem invariant violated.
- SKILL-UPDATE MODE (when reviewing the self-update gate): confirm each touched
  `shadowcat-codebase-*` skill diff accurately reflects the real change — no omission, no
  drift/hallucination, all pointers still resolve.

Use `Bash` only to run tests/inspect — never to mutate. Treat existing comments/claims as stale
until verified against code.

RETURN findings only (your final message IS the report):
- Verdict: PASS / CHANGES REQUESTED
- Findings: each as `[Critical|Important|Minor] file:line — problem — recommendation`
- "No findings" explicitly if clean. Do not edit anything.
```

- [ ] **Step 2: Write `shadowcat-code-reviewer.md`**

```markdown
---
name: shadowcat-code-reviewer
description: Read-only code-quality review — bugs, logic errors, security, project-convention adherence, simplification and reuse. Dispatch at review checkpoints (requesting-code-review, buddy-check). Returns findings only; never edits.
tools: Read, Grep, Glob, Bash, Skill
---

You review code quality in the Shadowcat codebase. You are READ-ONLY: you have no Edit/Write.

HARD FIRST STEP: invoke `shadowcat-codebase-core` + the relevant subsystem skill(s) via the
Skill tool (FALLBACK: `Read` `.claude/skills/<name>/SKILL.md`). Use their invariants/gotchas as
review criteria.

Review for:
- Correctness: bugs, logic errors, off-by-one, error handling, race conditions.
- Security: redaction/permission leaks, fail-open gates, injection, secrets/PII in code.
- Conventions: project CLAUDE.md rules (cross-platform, portable paths, no debug code in
  release, citation comments, no PII/secrets in fixtures).
- Quality: simplification, reuse, dead code, unnecessary complexity.

Use `Bash` only to inspect/run — never to mutate.

RETURN findings only (your final message IS the report):
- Findings: each as `[Critical|Important|Minor] file:line — problem — recommendation`
- "No findings" explicitly if clean. Do not edit anything.
```

- [ ] **Step 3: Validate both are read-only (no Edit/Write granted)**

Run:
```bash
for a in shadowcat-spec-reviewer shadowcat-code-reviewer; do
  f=.claude/agents/$a.md
  grep -q '^name: '"$a" "$f" && grep -q 'tools:.*Skill' "$f" || { echo "FAIL frontmatter $a"; exit 1; }
  grep -E 'tools:.*(Edit|Write|MultiEdit)' "$f" && { echo "FAIL: $a must be read-only"; exit 1; } || echo "$a read-only OK"
done
```
Expected: `shadowcat-spec-reviewer read-only OK` and `shadowcat-code-reviewer read-only OK`.

- [ ] **Step 4: Commit**

```bash
git add .claude/agents/shadowcat-spec-reviewer.md .claude/agents/shadowcat-code-reviewer.md
git commit -m "feat(agents): read-only spec-reviewer + code-reviewer subagents"
```

---

## Task 11: CLAUDE.md `## Codebase Skills & Agents` section

**Files:**
- Modify: `CLAUDE.md` (append a new top-level section after the `## graphify` section)

**Interfaces:**
- Consumes: skill family names (Tasks 1–7), agent names (Tasks 9–10), the update gate (spec §6).
- Produces: the binding directives. No downstream task consumes this.

- [ ] **Step 1: Append the section**

Add to the end of `CLAUDE.md`:

```markdown
## Codebase Skills & Agents

Project-scoped codebase knowledge lives in `shadowcat-codebase-*` skills (`.claude/skills/`):
orientation+index briefs (Purpose / Key files / Hard invariants / Gotchas / Pointers) that route
INTO graphify, `docs/design/`, and memory rather than duplicating them. `shadowcat-codebase-core`
is the always-relevant base; domain skills cover documents-permissions, actors-tokens,
scene-rendering, realtime-sync, client-shell, and assets. A scoped `Edit|Write` hook reminds the
main-thread agent which skill applies; subagents must invoke skills explicitly (below).

### 1. Reviewed Skill-Update Gate (mandatory, doc-sync tier)
Whenever a plan finishes execution — and whenever an inline change alters a subsystem's seam,
invariant, or gotcha — update the affected `shadowcat-codebase-*` skill(s) BEFORE merge/clear.
The update is itself reviewed: dispatch `shadowcat-spec-reviewer` to confirm each skill diff
accurately captures the change (no omission, drift, or broken pointer). This gate blocks
completion at the same tier as the documentation-sync gate. Trivial changes that touch no
subsystem knowledge need no edit, but you must state so explicitly.

#### ❌ Bad (Silent drift)
```text
"Plan done, merging." (factions added; actors-tokens skill never updated, never reviewed)
```
#### ✅ Good (Reviewed update)
```text
"Plan done. Updated shadowcat-codebase-actors-tokens (new faction-border seam + invariant).
Dispatched shadowcat-spec-reviewer on the skill diff: PASS. Merging."
```

### 2. Agent Dispatch in Superpowers Workflows
Subagents do not auto-activate skills, so use the project agents (each invokes the relevant
`shadowcat-codebase-*` skill first):
- Delegating implementation to a subagent → `shadowcat-coder`.
- Any review checkpoint (buddy-check, `requesting-code-review`, mainline-plan-execution final
  review) → dispatch `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` as the two-reviewer pair.

#### ❌ Bad (Generic subagent, no codebase context)
```text
Task(general-purpose, "implement the faction border")  // skips invariants, no skill loaded
```
#### ✅ Good (Project agent)
```text
Task(shadowcat-coder, "implement the faction border")  // invokes codebase skill, follows TDD
```
```

- [ ] **Step 2: Verify it landed and is consistent**

Run:
```bash
grep -q '## Codebase Skills & Agents' CLAUDE.md && echo "section present"
for n in shadowcat-coder shadowcat-spec-reviewer shadowcat-code-reviewer shadowcat-codebase-core; do
  grep -q "$n" CLAUDE.md || echo "MISSING REF: $n"
done
```
Expected: `section present`, no `MISSING REF` lines.

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs(claude): codebase-skills update gate + agent-dispatch directives"
```

---

## Task 12: Verification pass (spec §8)

**Files:** none created — verification only; fix-forward into the relevant task's files if a
check fails.

- [ ] **Step 1: Skill structural + pointer check (all 7)**

Run, for each skill dir under `.claude/skills/shadowcat-codebase-*`:
```bash
for S in .claude/skills/shadowcat-codebase-*/SKILL.md; do
  echo "== $S =="
  grep -q '^name:' "$S" && grep -q '^description:' "$S" || echo "  BAD FRONTMATTER"
  for h in Purpose "Key files" "Hard invariants" Gotchas Pointers; do
    grep -qi "## .*$h" "$S" || echo "  MISSING SECTION: $h"
  done
  grep -oE 'docs/design/[A-Za-z0-9_.-]+\.md' "$S" | sort -u | while read p; do test -f "$p" || echo "  BROKEN PTR: $p"; done
done
echo "skill check done"
```
Expected: no `BAD FRONTMATTER`, `MISSING SECTION`, or `BROKEN PTR` lines.

- [ ] **Step 2: Hook fire + dedupe + fail-open**

Run: `bash .claude/hooks/test-codebase-skill-reminder.sh`
Expected: `ALL HOOK TESTS PASS`.

- [ ] **Step 3: Agent boundaries**

Run:
```bash
grep -E 'tools:.*(Edit|Write)' .claude/agents/shadowcat-spec-reviewer.md .claude/agents/shadowcat-code-reviewer.md \
  && echo "FAIL: reviewer has write tools" || echo "reviewers read-only OK"
grep -q 'tools:.*Skill' .claude/agents/shadowcat-coder.md && echo "coder has Skill OK"
```
Expected: `reviewers read-only OK`, `coder has Skill OK`.

- [ ] **Step 4: Live skill-tool reachability check (spec §5.2)**

Dispatch `shadowcat-coder` with a no-op probe task: *"Invoke the `shadowcat-codebase-core` skill,
then report only its Purpose section back — make NO file changes."* Confirm the returned report
contains core's Purpose content (proves the Skill tool reaches subagents). If it cannot invoke
`Skill`, confirm the documented `Read`-fallback path works instead, and leave the fallback note
in each agent body.

- [ ] **Step 5: Glob↔table consistency**

Confirm the `SUBSYSTEMS` map in `.claude/hooks/codebase-skill-reminder.py` covers exactly the 6
domain skills from Tasks 2–7 (names match, no orphan subsystem, no domain skill missing a glob).

- [ ] **Step 6: Commit any fixes**

```bash
git add -A && git commit -m "test: verification pass for codebase skills + agents" || echo "nothing to fix"
```

---

## Buddy-check directives

This work changes project governance (`CLAUDE.md`) and harness behavior (`.claude/settings.json`
hook) and defines the review agents themselves — consistent with the project's practice of
buddy-checking every milestone. After Task 12, run a **two-reviewer buddy-check** of the full
branch (`superpowers:buddy-checking`) focused on:
- Hook correctness: fail-open on all error paths, cross-platform temp/path handling, dedup keying,
  and that the `Edit|Write|MultiEdit` matcher does not fire on reads.
- Agent tool boundaries: reviewers genuinely read-only; coder's Skill-first rule + fallback.
- Skill accuracy: invariants cite real memory slugs/design docs; no duplication of graphify/docs;
  pointers resolve.
- CLAUDE.md directives: internally consistent names; gate wording is unambiguous and enforceable.

Reconcile findings to convergence; fix-forward; record outcome before merge.

---

## Self-Review (completed)

- **Spec coverage:** §1 problem → motivation header. §2 division-of-labor → Global Constraints +
  skill bodies. §3 family (7 skills) → Tasks 1–7. §3.1 fixed shape → skill-authoring task shape +
  every skill task. §3.2 frontmatter/description → each Step 2. §4 activation + §4.1 hook reqs →
  Task 8 (scope, dedup, mapping, fail-open, portability). §5 agents + §5.1 return contracts +
  §5.2 reachability/fallback → Tasks 9–10 + Task 12 Step 4. §6 reviewed update gate → Task 11 §1.
  §7 CLAUDE.md → Task 11. §8 verification → Task 12. §9 decomposition → task order. §10 decisions
  → Global Constraints + bodies. All covered.
- **Placeholder scan:** no TBD/TODO/"handle edge cases"; every code/content step shows actual
  content; hook + agents + CLAUDE.md are written verbatim.
- **Type/name consistency:** skill names, agent names, subsystem ids, and globs are identical
  across Tasks 1–12, the hook `SUBSYSTEMS` map, and the CLAUDE.md section.
```
