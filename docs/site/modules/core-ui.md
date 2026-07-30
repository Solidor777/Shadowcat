# core-ui

## Purpose

The first-party layout module: owns the responsive region grid (`Layout`) and
declares the region surfaces every other UI element renders into. Replacing
this one module swaps the whole layout.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `core-ui:root` | `shadowcat.surface:root` | `Layout` | — |

## Components

- `Layout.svelte` — the responsive region grid; hosts the topbar / stage /
  statusbar / toolrail / panel-host surfaces.

## Contracts & seams

- **Provides** (surface declarations): `shadowcat.surface:root` (singleton),
  `:topbar` (singleton), `:stage` (singleton), `:statusbar` (singleton),
  `:toolrail` (multi), `:panel-host` (singleton).
- Region *content* comes from the per-element modules (topbar, statusbar,
  stage, scene-tools, panels) contributing into those surfaces.

## Pointers

- Source: `src/modules/core-ui/`
- API: [`@shadowcat/module-core-ui`](/api/ts/modules/_shadowcat_module-core-ui.html)
