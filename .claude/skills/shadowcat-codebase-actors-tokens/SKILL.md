---
name: shadowcat-codebase-actors-tokens
description: "Use when touching Shadowcat actors, tokens (linked vs instanced), token visual resolution, the factions registry, name privacy, or the actors/factions UI modules. Covers src/client/core/{actor.ts,scene-docs.ts} + src/modules/{actors,factions}. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Actors & Tokens

Orientation for the actor document model, token placement (linked/instanced), the factions
registry, and name privacy.

## Purpose

An `Actor` is a world-scoped document. A token on a scene either **links** to a shared actor
(reads it live + applies an override whitelist) or **instances** it (embeds an independent copy
with provenance). A single read-through resolves either to an `EffectiveActor` that the render
layer decorates. Factions are a world-scoped config-document; name privacy hides a token/actor
name from non-owners via the `OwnerOrGm` visibility tier.

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
- `src/client/core/src/actor.ts` — `resolveTokenActor(token, store) -> EffectiveActor | null`
  (the one read-through), `EffectiveActor`, `actorDisplayName(a, fallback)` (safe name with a
  redaction-aware fallback), `TokenOverrides` projection.
- `src/modules/actors/{ActorsPanel.svelte,index.ts}` — create/list/pick actors; hide-name control;
  faction assignment.
- `src/modules/factions/{FactionsPanel.svelte,index.ts}` — GM editor + idempotent seed of the
  faction registry; faction-colored token border + select-by-faction.

## Hard invariants

- **Instanced token's embedded actor copy needs `structuredClone`, not `{...}`** — a shallow copy
  aliases nested `system`/`permissions`/`embedded` with the source until the wire round-trip
  [[embedded-copy-needs-deep-clone]].
- **Registries are config-documents** (world-scoped, parentless, runtime-editable), not hardcoded.
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
