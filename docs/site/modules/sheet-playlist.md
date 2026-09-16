# sheet-playlist

## Purpose

The sheet for the `playlist` doc_type, registered at priority 0. Edits the playlist's name,
playback mode, mixer channel, crossfade duration, and its ordered tracks (asset, per-track
name/gain/loop) via a whole-array editor.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `sheet-playlist:sheet` | `shadowcat.sheet:playlist` | `PlaylistSheet` | sheet priority 0 |

## Components

- `PlaylistSheet.svelte` — name/mode/channel/fade fields; the tracks editor.
- `trackOps.ts` — pure, `structuredClone`-based helpers over `PlaylistEngine.tracks`; every
  mutation replaces the whole array via `setField` (`set_pointer` cannot grow arrays).

## Contracts & seams

- **Provides** `shadowcat.sheet:playlist` (multi; via `sheetContract(PLAYLIST_DOC_TYPE)`).
- The track asset picker calls `ctx.pickAsset({ kind: "audio" })`; an inline preview button
  plays the picked/current asset via `ctx.audio.playOneShot`.

## Pointers

- Source: `src/modules/sheet-playlist/`
- API: [`@shadowcat/module-sheet-playlist`](/api/ts/modules/_shadowcat_module-sheet-playlist.html)
