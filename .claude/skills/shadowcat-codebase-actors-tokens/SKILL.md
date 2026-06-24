---
name: shadowcat-codebase-actors-tokens
description: "Use when touching Shadowcat actors, tokens (linked vs instanced), token visual resolution, the factions/conditions registries, name privacy, or the actors/factions/conditions UI modules. Covers src/client/core/{actor.ts,scene-docs.ts} + src/modules/{actors,factions,conditions}. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Actors & Tokens

Orientation for the actor document model, token placement (linked/instanced), the factions
registry, and name privacy.

## Purpose

An `Actor` is a world-scoped document. A token on a scene either **links** to a shared actor
(reads it live + applies an override whitelist) or **instances** it (embeds an independent copy
with provenance). A single read-through resolves either to an `EffectiveActor` that the render
layer decorates. Factions and conditions are world-scoped config-documents; name privacy hides a
token/actor name from non-owners via the `OwnerOrGm` visibility tier. Conditions are markers-only
(no mechanical effects): icon badges overlaid on the token, toggled by the GM or the token owner.

## Key files & seams

- `src/client/core/src/scene-docs.ts` — builders + types (all re-exported from `core/index.ts`):
  - `buildActorDoc(worldId, system, id?)`, `ActorSystem`, `ActorVisual`.
  - `buildTokenFromActor(worldId, sceneId, actor, "link"|"instance", pos, size, id?)` — link mode
    sets `token.system.actor_id` + `overrides`; instance mode embeds an independent (deep-cloned)
    copy with `source` provenance.
  - `setNameHidden(doc, hidden)` — sets/clears the `OwnerOrGm` override on `/system/name`.
  - `FactionStance = "friendly"|"neutral"|"hostile"`, `Faction { name, color, stance }`,
    `FactionRegistrySystem`, `buildFactionRegistryDoc(worldId, factions, id?)` (param
    `factions: Record<string, Faction>`) — a
    world-scoped, **parentless config-document** with an id-keyed faction map.
  - `Condition { name, icon }`, `ConditionRegistrySystem`, `buildConditionRegistryDoc(worldId,
    conditions, id?)` (param `conditions: Record<string, Condition>`) — same parentless
    config-document shape as factions; `icon` is an emoji glyph rendered as a token badge.
- `src/client/core/src/actor.ts` — `resolveTokenActor(token, store) -> EffectiveActor | null`
  (the one read-through), `EffectiveActor`, `actorDisplayName(a, fallback)` (safe name with a
  redaction-aware fallback), `TokenOverrides` projection. Conditions: `resolveConditions(token,
  store)` (effective condition ids → `{id,name,icon}` via the registry, fail-closed) +
  `conditionTarget(token, store) -> {doc, path, conditions}` (the write site: linked →
  `actor` doc `/system/conditions`; instanced → token `/embedded/actor/0/system/conditions`).
- `src/modules/actors/{ActorsPanel.svelte,index.ts}` — create/list/pick actors; hide-name control;
  faction assignment.
- `src/modules/factions/{FactionsPanel.svelte,index.ts}` — GM editor + idempotent seed of the
  faction registry; faction-colored token border + select-by-faction.
- `src/modules/conditions/{ConditionsPanel.svelte,index.ts}` — GM editor + idempotent emoji seed
  of the condition registry + a token-selection-driven toggle palette; render via
  `TokenNodeSpec.badges` (upright glyph chips). Toggle gated by `AppContext.canEdit(doc, path)`
  (GM or token owner).

## Hard invariants

- **Instanced token's embedded actor copy needs `structuredClone`, not `{...}`** — a shallow copy
  aliases nested `system`/`permissions`/`embedded` with the source until the wire round-trip
  [[embedded-copy-needs-deep-clone]].
- **Registries are config-documents** (world-scoped, parentless, runtime-editable), not hardcoded.
  Keyed by id as a **map**, so adding an entry is a single-key Update (factions, conditions).
- **Engine owns the mechanism, a replaceable first-party module owns the content** — `module-factions`
  / `module-conditions` seed default content (idempotent GM seed); a game-system module replaces
  them wholesale. The registry/resolution/render seams stay engine-side.
- **Condition toggling is capability-gated client-side via `AppContext.canEdit(doc, path)`** — an
  advisory mirror of the server's Update-path check (GM bypasses; a non-GM needs the doc-role
  write cap). The server stays authoritative; the gate only shows/hides the control.
- **Name privacy rides the existing redaction layer** — `setNameHidden` flips `/system/name` to
  `OwnerOrGm`; the owner still sees it, others get the `actorDisplayName` fallback. Enforcement is
  server-side and fail-closed (see `shadowcat-codebase-documents-permissions`).

## Gotchas

- **Linked vs instanced provenance diverges**: a linked token reflects later actor edits; an
  instanced copy is frozen at placement. Instanced re-sync against the source is deferred
  [[document-inheritance-merge-model]].
- **Tokens are Container sprites behind a `TokenVisual` source abstraction** (static images ship
  first; multi-face/animated/procedural later) — don't bind rendering to raw image URLs
  [[token-architecture-forward-looking]].

## Pointers

- Rationale: the M10 tokens design spec under `docs/superpowers/specs/` (`*-m10-tokens-design.md`);
  data-model context in `docs/design/M2-data-foundation.md`.
- Relationships:
  `graphify query "actor token linked instanced resolveTokenActor EffectiveActor faction"`.
- Forward-looking visual pipeline: [[token-architecture-forward-looking]].
