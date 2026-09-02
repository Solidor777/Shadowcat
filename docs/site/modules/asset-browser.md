# asset-browser

## Purpose

The GM asset browser (`@shadowcat/module-asset-browser`): folder tree, filter
bar (name / regex / tags / kind / sort), virtualized thumbnail grid, and a
preview pane with tag editing, rename, byte replacement, original download,
reconvert, and delete. Uploads run through the chunked, resumable client
(`startChunkedUpload`) with a per-file progress queue; drop files on the grid
or a folder node, or use the file input. Multi-select drives bulk move / tag /
delete. Uploads are size- and rate-capped server-side; images convert to WebP
with the original retained (`retain_originals`) and thumb/preview derivatives
generated — see the asset pipeline in `docs/design/ARCHITECTURE.md` §4 of the
repository.

## Folders

Folders are ordinary `asset_folder` documents. Creating, renaming, and deleting
go through the document store; a folder move (drag, or the accessible
"Move to…" control) rides the generic GM-only document `Move` operation, with
the server enforcing placement and cycle rules. Deleting a folder offers a
choice: keep its assets (re-parented to the deleted folder's parent) or purge
them.

## Pick mode

Other modules request an asset with `AppContext.pickAsset(opts)` — the browser
opens as a modal (any member, not just the GM), filtered per the request, and
resolves with the chosen id(s); ordered multi-pick shows pick-order badges.
The scene-tools placement picker and the actor visual editor pick through this
seam.

## Contribution

- `asset-browser:panel` — the GM-only management panel (`shadowcat.panel`).
- `asset-browser:pick-overlay` — the pick-mode modal, contributed into
  `shadowcat.surface:overlay`.
