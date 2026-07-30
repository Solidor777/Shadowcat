# scene-tools

## Purpose

The canvas tool rail: place / select / move / draw / template / measure / ping /
wall / region tools. Contributes into core-ui's toolrail surface and drives the
canvas exclusively through public seams — it never imports core-ui or render
internals (the contract-only element boundary).

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `scene-tools:toolrail` | `shadowcat.surface:toolrail` | `ToolRail` | — |

## Components

- `ToolRail.svelte` — tool selection + per-tool options.
- `AssetPicker.svelte` — background/art picker used by placement flows.

## Contracts & seams

- **Requires** `shadowcat.surface:toolrail` (multi, from core-ui); depends on
  `core-ui ^0.1.0`.
- Drives `ctx.scene` (active tool, snap, drag), `ctx.actorSelection` (what the
  place tool stamps), `ctx.tokenSelection`, `ctx.sendPing`, and the
  `pathfind`/`moveRequest` seams for gated movement.

## Pointers

- Source: `src/modules/scene-tools/`
- API: [`@shadowcat/module-scene-tools`](/api/ts/modules/_shadowcat_module-scene-tools.html)
