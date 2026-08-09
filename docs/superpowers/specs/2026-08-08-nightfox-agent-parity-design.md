# Nightfox agent parity via a Shadowcat-sourced plugin

## Problem

`C:\Dev\Nightfox` is a standalone repository (own git history, own remote) that is also cloned
into a Shadowcat checkout at `src/modules/nightfox/` for development. Opened standalone, it gives
an agent a 21-line `CLAUDE.md` and nothing else: no codebase skills, no project agents, no
permission rules, no skill-routing hook, no Kimi configuration. Every engineering standard that
governs Shadowcat work is absent when the same agent works on Nightfox.

The naive fix — copy Shadowcat's `.claude/` into Nightfox — creates 15 duplicated skills and 6
duplicated agents that must be hand-synced forever, and collides on skill names during nested
development (two skills both named `shadowcat-codebase-core` in one session).

## Goal

Nightfox sessions get the same skills, agents, and standards as Shadowcat sessions, with the
shared material stored exactly once.

## Non-goals

- **Standalone-clone parity.** A single-source plugin cannot also be self-contained in a fresh
  clone with no Shadowcat present. Drift-freedom was chosen over standalone parity; a stranger
  cloning Nightfox gets a dangling plugin reference and falls back to `CLAUDE.md` alone.
- **Promoting shared standards to the global `~/.claude/CLAUDE.md`.** Rejected: not every Claude
  project is a code project, and those blocks would then govern prose and campaign-notes projects.
- **New `nightfox-*` agents.** The existing agents are skill-driven and `CLAUDE.md`-driven, so
  they adapt by context.

## Architecture

Shadowcat's repository doubles as a plugin marketplace. Its `.claude/` directory already has the
exact layout a plugin expects — `skills/`, `agents/`, `hooks/` — so it becomes the plugin source
in place, with no file moves and no change to what Shadowcat's own sessions load.

```
C:\Dev\Shadowcat\
  .claude-plugin\marketplace.json   NEW — declares the marketplace + one plugin
  .claude\                          UNCHANGED LOCATION = the plugin root
    skills\   (15 tracked + graphify) shared, single copy
    agents\   (6 agents)            shared, single copy
    hooks\    codebase-skill-reminder.py + hooks.json (NEW)
    kimi.plugin.json                NEW — Kimi shim
```

All 26 files currently tracked under `.claude/` stay tracked, stay in Shadowcat's history, and
stay subject to the reviewed skill-update gate in `CLAUDE.md`.

**A plugin source is a directory, not a git tree.** The `skills/` directory holds 15 tracked
`shadowcat-codebase-*` skills plus `graphify`, which is git-ignored. `graphify` therefore ships
to Nightfox through the plugin despite never being committed. This is accepted rather than
worked around: the skill is inert where no `graphify-out/` exists, and excluding it would mean
either moving it or adding a filtering mechanism the plugin format does not offer.

### Enablement asymmetry

The plugin is enabled in **Nightfox only**. Shadowcat already loads `.claude/skills` and
`.claude/agents` natively; enabling the plugin there as well would register every skill name and
all 6 agent names twice.

Nested development needs no special handling. When Nightfox sits at
`Shadowcat/src/modules/nightfox/`, the session's project directory is Shadowcat, so Nightfox's
settings never load, the plugin never activates, and the native skills serve. The collision path
does not exist.

## Components

### 1. Marketplace manifest

`C:\Dev\Shadowcat\.claude-plugin\marketplace.json` declares marketplace `shadowcat` owning one
plugin named `shadowcat-codebase` with `"source": "./.claude"`. Relative sources are an
established convention — the official superpowers marketplace uses `"source": "./"`.

Registered once per machine: `/plugin marketplace add C:\Dev\Shadowcat`.

### 2. Nightfox enablement

`C:\Dev\Nightfox\.claude\settings.local.json` gains
`"enabledPlugins": { "shadowcat-codebase@shadowcat": true }`, matching the existing
`enabledPlugins` idiom used for the `personal-skills` plugins.

It goes in `settings.local.json` rather than `settings.json` because it names a machine-local
absolute path; committing it to the public repo would ship a reference that resolves nowhere for
anyone else.

### 3. Agent body corrections

Each of the 6 agents carries a fallback instruction to read skills from a project-relative path
that is wrong once the skills are reached through a plugin. The fallback path changes to the
plugin root. Per the existing sync-pair convention, every body edit is mirrored to that agent's
`-opus` twin.

The instruction to obey "the project `CLAUDE.md`" is left alone — it resolves correctly per
project, which is the intent.

### 4. Skill-routing hook

`codebase-skill-reminder.py` maps edited paths to skill *names*, never to skill *paths*, so it
travels into the plugin unchanged. Two additions:

- **Nightfox-relative patterns.** The `nightfox` subsystem entry currently matches
  `src/client/formula/` and `src/modules/nightfox` — the second only fires in a nested checkout.
  It gains the standalone Nightfox module paths (the sheets directory and the roll, resolve,
  contributions, and document-model modules) so the hook fires in a standalone tree.
- **`hooks/hooks.json`.** A `PreToolUse` registration matching `Edit|Write|MultiEdit`, invoking
  the script through `${CLAUDE_PLUGIN_ROOT}`. This is the documented plugin-hook mechanism.

Shadowcat keeps its existing `settings.json` registration of the same script. The new
`hooks.json` sits inside a project `.claude/hooks/` directory, which is not itself a settings
source, so it should be inert for Shadowcat and active only for Nightfox via the plugin.

**Open risk.** That inertness is inferred, not observed. Verification during implementation: run
an edit in a Shadowcat session and confirm the reminder fires exactly once, not twice. If it
double-fires, the fallback is to drop `hooks.json` and register the block directly in Nightfox's
`settings.json` with an absolute script path — costing one duplicated block, no duplicated logic.

### 5. Nightfox `CLAUDE.md`

Grows from its Project block alone to a full standards file:

- **Kept as-is:** the existing Project block (stack, externals invariant, dev flow, cross-repo
  friction reporting).
- **Copied and adapted** from Shadowcat, with Nightfox examples substituted for Shadowcat ones:
  Collaboration & Execution Standards, Lint Suppressions, Security & IP Standards, Code
  Commenting Rules, Documentation Standards.
- **Rewritten, not copied:** the cross-platform block. Nightfox ships no server binary and has no
  OS build matrix, so the mandate reduces to browsers and mobile — responsive viewport, touch
  targets, no hover-only interaction.
- **Omitted:** Shadowcat's Reference Docs table, graphify section, and Codebase Skills section.
  All three point at Shadowcat-only artifacts.

This duplication is deliberate and was chosen over the alternatives. To keep it visible at the
point of edit, each copied block carries a sync marker in both files, reusing the
`<!-- Sync-paired with ... -->` convention already used by the agent pairs.

### 6. Nightfox permissions

`C:\Dev\Nightfox\.claude\settings.json` gains Shadowcat's `permissions` block — `trash` allowed,
`rm` and the PowerShell deletion aliases denied. Plugins cannot ship permission rules, so this is
genuinely duplicated. It is small and stable.

### 7. Kimi

Kimi Code does not read Claude's plugin manifests, and Claude `agents/` directories do not port
through the shim. Two pieces:

- **Skills:** `C:\Dev\Shadowcat\.claude\kimi.plugin.json` maps `skills` to `./skills/`, installed
  once with `/plugins install C:/Dev/Shadowcat/.claude` followed by `/reload`. Kimi plugins are
  global, so one install covers both projects. The third-party trust prompt defaults to cancel
  and must be approved.
- **Agents:** `.kimi-code/agents/` in **both** repositories, holding the 6 agents in Kimi's
  manifest format. This mirrors the split already present in `C:\Dev\Titan\.kimi-code\`.

**Effort and model remapping.** Kimi's `k3` declares support for `low`, `high`, and `max` — there
is no `medium`, and `opus` is not a Kimi model. The mapping:

| Claude agent tier | Claude setting | Kimi setting |
|---|---|---|
| coder (base) | `model: sonnet`, `effort: medium` | `k3`, `effort: low` |
| reviewers (base) | `model: sonnet`, `effort: high` | `k3`, `effort: high` |
| all `-opus` twins | `model: opus`, `effort: high` | `k3`, `effort: max` |

The base coder maps down to `low` rather than up to `high` to preserve the tier gap that makes
escalation to the twin meaningful; collapsing both onto `high` would erase the ladder.

## Verification

1. `/plugin marketplace add C:\Dev\Shadowcat` succeeds and lists `shadowcat-codebase`.
2. In a Nightfox session: all 15 `shadowcat-codebase-*` skills and 6 agents resolve, and invoking
   `shadowcat-codebase-core` returns content.
3. In a Shadowcat session: skill and agent names each appear once, not twice.
4. Editing a Nightfox sheets file fires the skill reminder naming `shadowcat-codebase-nightfox`.
5. Editing a Shadowcat file fires the reminder exactly once.
6. `git ls-files .claude/` in Shadowcat still lists all 26 files; nothing left the repository.
7. In Kimi, after install and `/reload`, the skills list includes the `shadowcat-codebase-*`
   entries and the agents resolve in both workspaces.

## Consequences

- Shared skills and agents exist once, in the repository whose code they describe, under the
  review gate that keeps them honest.
- The five copied `CLAUDE.md` blocks are the only remaining hand-synced material. Sync markers
  make the pairing visible; nothing enforces it automatically.
- A Nightfox clone on a machine without Shadowcat degrades to `CLAUDE.md` only. This is the
  accepted cost of single-sourcing.
