# vfx

## Purpose

Animated effects on the stage: per-token emitters (a `VfxEmission` on an
actor or token override, played as an `emitter:<token>` node tracking the
token's live transform) and transient one-shots any world member may fire at
a scene point (relayed room-wide as a `vfx` frame, played as a `oneshot:<id>`
node). Concurrent one-shots never stall the stage: the oldest live one-shot
is evicted past a 64-per-scene cap. Every node renders in the `vfx` core
layer (between `templates` and `lighting`, below the fog `mask`).

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `vfx:panel` | `shadowcat.panel` | `FxToolPanel` | order 3, launcher-closed, icon ✨ |
| `vfx:scene-tool` | `shadowcat.scene-tool` | — | `SceneToolMeta{id: "vfx"}` (registered by `FxToolPanel`'s own `$effect`) |

## Effect assets (no new codec)

Two formats, both already storable by the asset pipeline:

- **Animated WebP/GIF** — stored pass-through; the server derives a grid
  sheet at commit/reconvert time (never lazily): `<uuid>.sheet.webp` (frames
  tiled near-square, longest side capped at 4096 px) plus a
  `<uuid>.sheet.json` timing/geometry sidecar, recorded on `AssetMeta.sheet`
  and served as `?variant=sheet`.
- **Paired spritesheet** — a PNG/WebP atlas plus a sidecar JSON in PixiJS's
  spritesheet format, uploaded as two assets and paired by the explicit tag
  `vfx:sheet=<json-asset-id>` on the IMAGE asset (the asset browser's "Pair
  sheet" action). Pairing rules: the sidecar must be `application/json` in
  the same world, its `meta.image` must name the image asset's
  `original_name` exactly (the TexturePacker convention), and its
  `animations` map must define a `"default"` entry — the resolver never
  reads the sidecar's bytes, so `"default"` is the one fixed name.

Every effect asset should carry the constant explicit tag `vfx` — the FX
tool's picker and the asset browser's "VFX" quick filter select on it. The
tag is a browsing aid, never a gate: an untagged animated asset is still a
valid emitter.

## The FX scene tool (`SCENE_TOOL_CONTRACT`)

`SCENE_TOOL_CONTRACT` (`shadowcat.scene-tool`) is the multi-cardinality
extension point the scene-tools rail renders contributed tools from. A
contribution carries `SceneToolMeta { id, icon, labelKey, onSceneClick(x, y)
}` — nothing more; a tool needing configuration holds it elsewhere (the FX
tool keeps its last-picked asset/scale/sound in the module-scoped
`fxToolState`, edited from the FX panel; a user who never opens the panel
gets an inline asset-pick prompt on first click). `ToolController.active`
stays the closed built-in `ToolId` union; contributed tools activate through
the parallel `activeContributedId` field, mutually exclusive in both
directions. See the [creating-a-module guide](../guides/creating-a-module)
for a worked example.

## The `/fx` chat command

`/fx <asset-id-or-name> @<token name>` plays a one-shot at the named token's
center on the world's currently active scene. The asset reference is the
first whitespace-separated word — a raw asset id tried first, then a
case-insensitive exact match on `original_name`. The `@`-tail is the rest of
the line (a token name may contain spaces) and is required — `/fx` without a
target whispers back the usage notice. Token resolution is server-side and
never oracles: a token the sender cannot read produces the same "No such
token." notice as a nonexistent name. A successful `/fx` authors no chat
message; a failure authors a whispered system notice to the sender.

## Playback behavior

- **Reduced motion** (`PerformanceSettings.reducedMotion`): emitters render
  frozen on their last frame; one-shots are skipped entirely.
- **`vfx` budget off** (`PerformanceSettings.vfx`): every node is torn down
  on the next reconcile and `play` is a no-op. Per-device, never a
  server-side suppression.
- **Rate limit**: one-shots are limited per user on their own server-side
  budget (30/min), separate from ping/emote/chat; a burst starves nothing
  else.
- **Anchor rule** (all inside the `vfx` layer, which sits above `tokens`):
  `below` centers on the token's footprint base (z-order 0), `token` on its
  center (1), `above` on its top edge (2).

## Pointers

- Source: `src/modules/vfx/`
- API: [`@shadowcat/module-vfx`](/api/ts/modules/_shadowcat_module-vfx.html)
