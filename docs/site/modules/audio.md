# audio

## Purpose

The Audio panel: per-device channel gain/mute sliders, the GM-only "now playing" transport
(pause/resume/stop/next/prev/seek, plus stop-all), and the playlists list with live search,
per-row Play, Create/Delete, and open-sheet.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `audio:panel` | `shadowcat.panel` | `AudioPanel` | order 4, launcher-closed, icon 🔊 |

## Contracts & seams

- **Requires** `shadowcat.panel` (`PANEL_CONTRACT`); depends on `core-ui`.
- Transport controls render only for `ctx.role === "gm"`; a player sees the now-playing list
  read-only. `ctx.audio.transport` is fire-and-forget — a refusal surfaces as a
  shell-wide toast via `onAudioError`, never a rejected promise.
- Delete is gated by `ctx.canDelete(doc)`, Create by `ctx.canCreate(PLAYLIST_DOC_TYPE)` — never
  `doc.owner === ctx.selfId`.
- Search sends `ctx.searchDocuments(q, { limit: 20, docTypes: [PLAYLIST_DOC_TYPE] })` — the one
  server-side filter, no client-side re-filter.
- Channel sliders/mutes call `ctx.audio.setChannel` directly — device-local state, never sent
  to the server.
- The GM "listen as" picker calls `ctx.audio.listenAs(tokenId | null)` — a fire-and-forget
  spatial-audio preview seam, independent of any token this connection owns.

## Pointers

- Source: `src/modules/audio/`
- API: [`@shadowcat/module-audio`](/api/ts/modules/_shadowcat_module-audio.html)
