# assets

## Purpose

The asset panel: upload, browse, replace, and delete uploaded files (scene
backgrounds, token art). Uploads are size- and rate-capped server-side (GM caps
default to 2× player caps). Images are converted to WebP on upload, with the
original retained beside the canonical (`retain_originals`) and thumb/preview
derivatives generated — see the asset pipeline in `docs/design/ARCHITECTURE.md` §4 of the
repository. Non-image
files are stored pass-through under their declared type.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `assets:panel` | `shadowcat.panel` | `Assets` | order 1, icon 🖼️, labelKey `assets.tab`, launcher-closed |

## Components

- `Assets.svelte` — upload + grid + replace/delete.

## Contracts & seams

- **Requires** `shadowcat.panel`; depends on `core-ui ^0.1.0`.
- Uses `ctx.assets` (the `AssetResolver`, cache-busting on replace, `?variant=`
  derivative URLs) and `ctx.onAssetChanged` (out-of-band created / replaced /
  moved / deleted notices) — the same seams every asset-displaying module uses.

## Pointers

- Source: `src/modules/assets/`
- API: [`@shadowcat/module-assets`](/api/ts/modules/_shadowcat_module-assets.html)
