# Wire protocol

The Shadowcat client↔server protocol: one HTTP login, one WebSocket per world
session, JSON frames both ways. This page is the frame-level map; every payload
type links into the generated reference, where the ts-rs types carry the Rust
source's own documentation.

Authoritative source: the discriminated unions in `src/client/core/src/wire.ts`
([`ServerMsg`](/api/ts/types/_shadowcat_core.ServerMsg.html),
[`ClientMsg`](/api/ts/types/_shadowcat_core.ClientMsg.html)) — the client
validates every inbound frame against these Zod schemas.

## Connection lifecycle

1. **Login over HTTP** — `POST /api/login` sets a signed session cookie
   (rate-limited per identity and per IP).
2. **WebSocket** — the client opens `/ws` (cookie carries auth) and sends
   `hello` naming the world and its last-applied sequence number (or `null` for
   a cold start).
3. **`welcome`** — the server answers with the world id, `current_seq`, server
   time and version, the world's default capability grants, the caller's world
   role, capability requirements, UI contract declarations, and system schema
   declarations.
4. **Event stream** — from then on the socket carries sequenced events,
   request/response frames, and pushes until close or `evicted`.

## Sequencing and resync

Every mutation is a sequenced `event` (monotonic `seq` per world). The client
tracks its applied watermark (`appliedSeq`); on reconnect it sends the watermark
in `hello`, and the server replays what was missed — either as individual
events or bracketed by `resync_begin` / `resync_end` when a snapshot is cheaper.
The optimistic client keeps rendering its predicted view throughout; the
watermark is what makes rollback sound.

## Intents and events

Clients never mutate state — they send an `intent` carrying operations
([`WireOperation`](/api/ts/types/_shadowcat_core.WireOperation.html)):

- `create` / `delete` — a whole document,
- `update` — field-level changes
  ([`WireFieldChange`](/api/ts/types/_shadowcat_core.WireFieldChange.html)):
  JSON-pointer `path`, the OCC pre-image `old`, the value `new`, and an optional
  `remove: true` meaning *delete the key* (genuine absence, distinct from
  `null`),
- `move` — re-parent a top-level document: `doc_id`, the new `parent_id`
  (`null` = top level), and `old_parent_id` as the OCC pre-image. GM-only, and
  legal exactly where a `create` with that parent would be (the server enforces
  placement and folder-cycle rules); the one write path for the envelope's
  otherwise-immutable `parent_id`.

The server validates (permissions, OCC, schema), applies, and broadcasts a
[`WireCommand`](/api/ts/types/_shadowcat_core.WireCommand.html) inside an
`event` frame whose `intent_id` echoes yours — or answers `reject` with a
reason. **Broadcasts are filtered per recipient before transmission**: fields a
user may not see are stripped server-side and never cross the wire
(ARCHITECTURE invariant — redact-then-send, never send-then-hide).

## Frame catalog — server → client

Every `ServerMsg` variant:

| Frame | Purpose |
|---|---|
| `welcome` | Session bootstrap: world, `current_seq`, versions, grants, role, `role_capabilities`, contract/schema declarations |
| `event` | One sequenced, per-recipient-filtered command (ops batch), with correlating `intent_id` when it answers yours |
| `reject` | Your intent was refused; carries `intent_id` + reason — roll back the prediction |
| `resync_begin` | Replay window opens (`from_seq`..`to_seq`, with source) |
| `resync_end` | Replay window closed; `current_seq` is authoritative again |
| `time_pong` | Answer to `time_ping`: echoes `client_t0` with `server_t` for clock offset |
| `ping` | Server liveness probe; answer with `pong` |
| `error` | Protocol-level error with a machine code + message |
| `search_result` | First page for a `search` request (`hits`, `next_cursor`) |
| `search_update` | Live push of refreshed hits for a subscribed search |
| `search_error` | Search request failed |
| `scene_derived` | One update on a subscribed scene channel (`channel`, `computed_at_seq`, opaque payload) |
| `scene_error` | Scene subscription failed |
| `asset_changed` | Out-of-band notice: an asset was `created`, `replaced` (cache-bust signal), `moved` (name/folder/tags; version unchanged) or `deleted` |
| `scene_ping` | A user's transient location ping on a scene (includes your own echo) |
| `emote` | A user's transient emote glyph over a token (includes your own echo) |
| `vfx` | A relayed VFX one-shot at scene coords (includes your own echo) |
| `path_result` | Pathfinder answer: waypointed `path`, `cost`, `arrested` flag, and `budget_cells` — the mover's remaining movement budget in cells under an enforced combat, `null` when no enforced combat applies ([`PathResult`](/api/ts/interfaces/_shadowcat_core.PathResult.html)) |
| `path_error` | Pathfind request failed |
| `move_error` | Move request failed |
| `chat_error` | Chat send/edit/delete failed |
| `move_stream` | Broadcast move animation: timed position samples (empty for a glow-only recipient), per-recipient-clipped mover vision, per-recipient-admitted carried-light timeline, nullable cost ([`MoveStream`](/api/ts/interfaces/_shadowcat_core.MoveStream.html)) |
| `combat_result` | A `combat_*` intent from you was accepted; carries the committed `seq` your correlated wait resolves on |
| `combat_error` | A `combat_*` intent from you was refused; carries the player-presentable reason |
| `merge_result` | Outcome of a `merge_pull`/`merge_push`/`merge_revert` with this `request_id`: applied, or the conflict set to resolve |
| `merge_error` | A merge intent was rejected (not found, not an instance, forbidden, or stale/unknown/unresolvable resolutions carrying the fresh outcome) |
| `audio_error` | An `audio_transport` op was refused (not GM, unknown id, over cap, invalid gain) — connection-local, never broadcast |
| `evicted` | Terminal: your seat or the world is gone; the server closes the socket — do not reconnect |

## Frame catalog — client → server

Every `ClientMsg` variant:

| Frame | Purpose |
|---|---|
| `hello` | Join a world with `last_seq` watermark (`null` = full sync) |
| `intent` | Optimistic ops batch under an `intent_id` |
| `resync_request` | Ask for replay from a sequence number |
| `time_ping` | Clock-offset probe |
| `pong` | Liveness answer |
| `search` | Full-text query (`limit`, cursor, `subscribe` for live updates); `doc_types` narrows to the listed types (≤16, refused above the cap) |
| `unsubscribe` | End a live search |
| `scene_subscribe` | Open a scene-derived channel |
| `scene_unsubscribe` | Close it |
| `scene_ping` | Broadcast a location ping at scene coords |
| `emote` | Broadcast a transient emote glyph over a token you effectively own |
| `play_vfx` | Fire a one-shot VFX at scene coords; rate-limited per user, spectators refused |
| `pathfind` | Request a route (`start`, `waypoints`, footprint or `token`) |
| `move_request` | Request server-executed movement of a token along a path |
| `send_message` | Chat: post to a channel (optional actor attribution + audience). The channel must be a key of the world's channel registry; dice notation in the body may carry stat references, resolved server-side against the actor binding; a `[[asset:<uuid>\|alt]]` span renders as an image segment |
| `edit_message` | Chat: edit own message |
| `delete_message` | Chat: delete own message |
| `draw_table` | Draw one or more rows from a rollable `table` document, posted as one `MessageKind::Roll` message — see [Rollable tables](#rollable-tables) |
| `combat_start` | Activate a combat |
| `combat_pause` | Deactivate a combat, preserving its clock position |
| `combat_end` | End a combat (deletes it; children cascade) |
| `combat_advance` | End the current turn |
| `combat_rewind` | Step the clock back to the previous turn record |
| `combat_sort` | Rebuild the turn order from current initiatives |
| `combat_roll` | Roll initiative for named combatants, posting the result to a chat channel |
| `combat_resource` | Adjust one combatant's tracked resource (delta or set) |
| `merge_pull` | Merge an instance's template into it — the server computes the 3-way merge and, when conflict-free, commits it |
| `merge_push` | Merge a template into every same-world instance visible to the sender |
| `merge_revert` | Reset an instance's mergeable bands to its template's current state (never conflicts) |
| `audio_transport` | GM-only playlist/playback transport op (play/pause/seek/skip/gain) |
| `audio_listen_as` | Set (or clear) this connection's spatial-audio listening token |

Dice reference resolution: a roll's notation is a **raw template** — `1d20 +
attributes.str` — never a client-substituted string. The server rewrites each
reference at ingest (labeled constants in the roll breakdown carry the value
read) against the send's `actor_owner` host (a token instance's embedded actor
copy, else the linked actor), or, for combat rolls, against each named
combatant's formula host. A referencing roll with no binding fails with an
`unknown-ref` system notice. The same raw-template rule applies to the
`notation` of every combat-roll entry, and a combat roll's `channel` is
validated against the channel registry the same way a message's is.

Images are asset-served, never hotlinked: a `[[asset:<uuid>|alt]]` span
references an existing in-world asset directly, and a Markdown/HTML image URL
in a `markdown`/`html`-enabled message is fetched by the server (the same
SSRF-guarded pipeline link previews use) and asset-ified in the background
before the resulting `image` segment is appended to the stored message — the
client never fetches an external image URL itself. A YouTube/Vimeo link
similarly becomes an oEmbed card (thumbnail + link, structured fields only,
never the provider's own embed HTML) rather than a raw hotlink.

## Search

Every document type shares one FTS5 backend (`data::search::index_content`):
the envelope `name`, the engine-aware reader-facing projection of `engine`
(`data::engine::search_text` — a note's rendered body, a table's labels and
row text, a message's rendered content, an actor's display name; never
discriminants, ids, markup, or dice-notation internals), and every string/
number leaf of the content-agnostic `system` body. `doc_type` itself is not
an index term; `search`'s `doc_types` field narrows the ranked SQL to the
listed types instead (≤16, refused above the cap). The public/GM visibility
partition and the per-hit `cap::READ` gate are unaffected by any of this.

Assets are not documents and carry no live subscription: `GET
/api/worlds/{world}/assets` takes a `q` parameter, matched against a
trigger-maintained `assets_fts` table (name + every explicit/derived tag) via
the same sanitizer `search` uses. The asset browser already re-lists on every
`asset_changed` notice, so `q` is a plain filter under the caller's keyset
sort — never a ranked, live-updating page.

## Rollable tables

A `table` document (`TableEngine`: `draw` rule, `rows`, plain-text
`description`) is drawn from with `draw_table` (`table_id`, `channel`,
`count`, optional actor attribution/audience). `DrawRule::Weighted` rolls a
`1d<sum-of-weights>` and matches the row whose cumulative weight reaches the
total; `DrawRule::Formula` rolls its own notation and matches the row whose
inclusive range contains the total (no match leaves the draw's `row` `null`).
A matched row's `results` resolve into `content` (`text`/`doc`/`image`
entries, same segment shapes chat itself uses) and `nested` — one
`table_draw` segment per `TableEntry::Draw` entry, recursing up to a fixed
depth and per-request draw budget with cycle detection (a table naming
itself, directly or through a chain of nested draws, is refused). Every
resolved draw becomes one `table_draw` chat segment, redacted exactly like a
`roll_embed`: `spec`/`raw` are GM-only at every depth, never sent to a
player. `draw_table` follows the same channel/audience/actor-attribution
validation as `send_message`, and the same asymmetric confirm-by-broadcast
protocol — a refusal (unknown table, no READ, cycle, too many/deep draws) is
a correlated `chat_error`, never a hard `reject`.

## Notes

A `note` document (`NoteEngine`: author `source` markdown, a server-derived
`body`, sibling `sort`) is authored/edited like any other document — plain
`Create`/`Update` through the generic engine-ingress gate, no dedicated wire
frame. `body` is unconditionally overwritten on every Create/Update
post-image: the server renders `source` through the same span grammar and
sanitizer boundary chat messages use, under a policy fixed to the note
subsystem rather than the world's own chat settings, so notes stay rich even
in a plain-text-chat world. `[[roll:...]]`/bare `[[...]]` spans become
`roll_button` segments (never executed — a note save must never roll dice as
a side effect), `[[doc:...]]`/`[[token:...]]` spans become `doc_link`
segments, and `[[asset:...]]` spans become `image` segments; there is no
outbound fetch on this write path, so a Markdown image URL in a note's
source renders only as its alt text. Notes form a `parent_id` tree of notes
(a note's parent must be another note in the same world; a note is never
embedded); private to its author by default (`permissions.default: "none"`,
the author granted `Owner`) until shared.

## Template merge intents

The server computes every template 3-way merge (pull/push/revert) — the client only sends the
intent and renders whatever conflict set comes back. Each intent is a stateless two-call flow:

1. **Compute-only call** (no `resolutions`): the server merges from live documents. Conflict-free
   applies immediately and commits under the same `event` broadcast every other write uses;
   conflicted answers `merge_result` with the conflict set (per instance for `merge_push`) and
   writes nothing.
2. **Resolution call** (`resolutions`: the conflict paths whose template side to take — per
   instance for `merge_push`, keyed by instance id): the server RECOMPUTES the merge from live
   documents and rejects with `merge_error` (`stale_resolutions`/`unknown_resolution`/
   `unresolvable`, each carrying the freshly recomputed outcome) unless every submitted path is
   a current conflict whose template side the current merged shape can take — `unresolvable`
   is the ancestor/descendant case, where the instance replaced a container the template
   edited inside, so the template's leaf has nowhere to land and the user picks the instance's
   side instead — there is no server-side session between the two calls.

`merge_push` reports each same-world instance individually: `applied`, `conflicts`, or `excluded`
(visible to the pusher but not writable by them). An instance the pusher cannot see at all is
omitted from the reply entirely, matching redaction's own existence-hiding. A push commits its
instances one by one, each under its own `event`, and is not atomic across them: every submitted
resolution is validated before the first commit, so a resolutions rejection precedes any write,
but a rejection raised by a commit itself (`stale_resolutions` from an OCC pre-image that no
longer holds, `forbidden`, `internal`) leaves the instances committed before it in place with
their `event`s already broadcast. The fresh outcome a `stale_resolutions` carries is recomputed
from live documents, so an already-committed instance reads as `applied` there and the remainder
carry their current conflicts; re-sending the intent commits what remains. Every conflict set is
filtered to the paths the requester can see in both documents before it reaches the wire; a
hidden conflict resolves to the instance's own (child) value automatically.

**Visibility.** The merge runs over the requester's view of the template and of the stored
`base` snapshot (both reduced by the same per-recipient classifier that redacts documents), so a
value the requester cannot see never moves into an instance's content in either direction. The
stored `base` itself is one canonical value — the full template snapshot with its
`property_overrides` recorded (`propertyOverrides` on each embedded record), written by the server
alone — and every merge write carries the template's overrides onto the instance additively
(an `owner_or_gm` tier of a template with a different owner lands as `gm_only`), so a value a GM
pushes into a player's instance arrives hidden from that player. Each recipient receives `base`
cut by the policy it records, on top of the whole-band owner-or-GM floor: the instance owner sees
the snapshot minus what the template hid, a GM sees it whole, and nobody else receives it. A
client's "template changed" badge therefore compares its own view of the snapshot with its own
view of the template, and reads in sync for every seat after any seat's merge.

## Scene channels

Scene-derived data (vision, fog, lighting masks) does not travel as documents —
a client subscribes to a channel with `scene_subscribe` and receives
[`SceneFrame`](/api/ts/interfaces/_shadowcat_core.SceneFrame.html) payloads via
`scene_derived`, recomputed server-side as world state changes
(`computed_at_seq` orders them against the event stream). Subscriptions are
re-established by the session layer across reconnects.

On a scene with declared floors (`SceneEngine.levels`), `scene_subscribe`'s
optional `level` field selects which floor the `"vision"` channel computes
explored-fog and visibility for — level-less scenes and an absent `level` are
unaffected (the pre-levels behavior). A viewed-level change re-subscribes
`"vision"` (never `"footprints"` or any other channel) because the SERVER, not
just the client's own rendering, computes per-level explored-fog. The
`"footprints"` channel is not itself level-scoped by subscription (it always
covers the whole scene); each entry instead carries its own resolved `level`
(the level `scene::elevation::level_of` resolves for that token at its stored
elevation, or `null` for a level-less scene), so a client scopes it
client-side the same way it scopes any other point-elevation document type.

Movement is server-authoritative end to end: `move_request` → the server
validates and *executes* the move → every viewer receives `move_stream`, whose
position samples and mover-vision polygons are **clipped per recipient** before
sending — an observer who cannot see a stretch of the path simply never
receives it (the nullable `cost` exists for the same reason: the true cost can
leak secret terrain). "See" means line of sight **and** illumination: on a
lighting-enabled scene a normal-vision observer is sent only the samples that
fall in a lit cell (a carried torch counts, composed per instant from the
in-flight `mover_light` timelines), while darkvision within its range and a GM
see in the dark. The carried-light timeline is admitted per sample wherever its
glow lights a cell in the recipient's sight; a recipient the glow reaches but
the token never does gets a **glow-only** frame — `samples` empty,
`mover_light` present, `stop`/`duration_ms` at the last admitted light sample —
and a recipient reached by neither gets no frame at all. At least one of
`samples`/`mover_light` is non-empty in every delivered frame. Each
[`MoveLightSample`](/api/ts/interfaces/_shadowcat_core.MoveLightSample.html)
carries the emission's `intensity` and `falloff` alongside its reach, so the
frame describes its own light.

The `"combat"` channel carries every combat's resolved per-combatant resource numbers
(`current`/`max`/`error` for each registry-key binding) and movement-in-cells conversion
(`movement_cells` — present only when the combat's movement resource is a `tracked` binding
that resolves, through the same resolution the movement-budget gate reads, so a `mirror`
binding is `null` here exactly as the gate refuses it), recomputed on every combat/combatant
mutation. There is no client-side formula evaluation for
combat resources: a hidden combatant is absent from the payload entirely, and a visible
combatant's `resources` is `null` for a recipient the `/engine/resources` property tier does not
admit (a non-owner, non-GM reader) — the same two-gate discipline the `"footprints"` channel
follows.

The `"audibility"` channel carries the recipient's resolved spatial-audio listener (if any)
and every carried `SoundEmission` emitter in the world's active scene, already resolved to a
final `gain`/`pan` — falloff, wall occlusion, and the world's spatial/occlusion overlay are
computed once, server-side (`scene::audibility`), never re-derived client-side. A carried
emitter on a token the recipient cannot whole-document read is OMITTED from the payload
entirely (the same identity-disclosure gate `RecipientSight::sensed` applies to creature-sense
perception) — unlike a standalone light, which discloses no emitting-token identity and is
never gated this way.

## Local audio-monitor protocol

`shadowcat audio-monitor` is a SEPARATE localhost WebSocket, unrelated to the `/ws` protocol
above — no login, no world, no `ClientMsg`/`ServerMsg`. The ducking module's `OsMonitorSource`
connects to `ws://127.0.0.1:<port>/levels`; the connection's `Origin` header must be in the
monitor's allowlist or it is refused before any frame is sent.

| Frame | Direction | Shape |
|---|---|---|
| `hello` | monitor → client | `{ "type": "hello", "os": string, "supported": boolean, "reason"?: string }`, sent once on connect |
| `levels` | monitor → client | `{ "type": "levels", "sessions": [{ "process": string, "peak": number }] }`, at 10 Hz, already filtered to the watch list |
| `watch` | client → monitor | `{ "type": "watch", "names": string[] }`, replaces the live watch list without a restart |
