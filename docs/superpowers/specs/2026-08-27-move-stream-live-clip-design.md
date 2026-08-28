# Move-Stream Live Clip Against the Recipient's Own Vision Timeline — Design

**Status:** APPROVED (user, 2026-08-27). Supersedes
`2026-08-21-realtime-move-streaming-design.md`, whose §3 mechanism was shown not to work.

**Governing principle:** `docs/design/ARCHITECTURE.md` invariant 11 — user experience outranks
data secrecy. This design happens to keep both; if implementation forces a choice, the wire
secrecy is what yields, never the visual.

## 1. The gap, precisely

An observer R's copy of token A's `MoveStream` is clipped ONCE, at egress, against R's vision
as the server holds it at that instant (`clip_move_stream` → `observer_vision_polys_for_scene`
→ `SceneEcs::player_vision_polygons`, the COMMITTED-position vision). R's client hides A during
the occluded spans by inferring gaps in the clipped sample timestamps (`TokenAnimator`
`computeGapThreshold`/`isHidden`). R's client fog changes only when (a) R's own move sweep plays
(`mover_vision` samples, held client-side in `RenderEngine.visionSweeps`) or (b) the server
rebroadcasts the `vision` channel, ~150 ms after a commit Event — i.e. after a move completes.

Two sub-cases where A "snaps" into view at A's stop instead of appearing mid-stride:

1. **R's own token is moving concurrently with A.** R's vision along its own path is already
   fully known — the server computed R's `mover_vision` timeline at R's execute time and R's
   client is sweeping the fog with it. Today's clip ignores that timeline:
   - A starts **after** R: the clip uses R's committed (= destination) vision, not R's vision at
     the instant of each sample — wrong both ways (over- and under-admits mid-path samples).
   - A starts **before** R: the clip used R's pre-move vision; when R's own sweep later opens the
     sightline the already-sent clipped stream cannot widen.
2. **A third party's moving light source would open R's sightline mid-walk.** R's vision at
   that instant exists nowhere; computing it is the per-tick loop the superseded spec rejected.

This design closes case 1 completely (both orderings) and leaves case 2 as a completion-time
snap. Case 2 stays parked (`docs/TODO.md`) unless the user asks for it to be costed.

## 2. Mechanism

### 2.1 Room retains every in-flight stream

`Room.moving: Mutex<HashMap<Uuid, i64>>` (token → move-end epoch-ms) becomes a map of
token → `ActiveStream { mover: Uuid, scene: Uuid, start_ms: i64, end_ms: i64, frame: Arc<ServerMsg>
(the full unclipped MoveStream, mover_vision included) }`. Insertion happens where the moving
lock is set today (inside `execute_move`, still under `publish_guard`); the frame is registered
at the broadcast point — the plan places the single write so the lock check and the stream
registry never fork. Lazy expiry (`retain(now < end)`) is unchanged. The existing moving-lock
check reads `end_ms` from the same entry, so there is one structure, not two.

### 2.2 Clip against the recipient's vision AT EACH SAMPLE INSTANT

`clip_move_stream` (observer branch and the GM see-as branch alike — the "clip target" user is
whichever `PermissionContext` the branch already selects) computes, for each sample `s` of A at
absolute time `t_abs = A.start_server_ms + s.t_ms`:

- Collect the clip-target user's active streams in A's scene (`mover == target.user_id`,
  `start_ms <= t_abs`), from the registry in 2.1.
- If none is active at `t_abs`: `polys = player_vision_polygons(target)` filtered to the scene,
  exactly as today.
- Otherwise `polys` = union over those streams of the stream's `mover_vision` sample with the
  greatest `t_ms <= (t_abs - stream.start_ms)` (falling back to the first sample; past the last
  sample, the last sample — it equals the mover's at-rest destination vision, which is what the
  committed-position vision becomes). This sample rule is the client's `chosenSample` rule; the
  server and client MUST agree on it — the plan adds a parity test.
- Sample `s` is kept iff its `pos` is inside any polygon of `polys`.

`stop`/`duration_ms`/`cost`/`truncated`/`mover_vision` handling for observers is unchanged.

Secrecy property: the timeline polygons are the target's OWN vision (already sent to that user
as `mover_vision`), so this never admits a sample the target's fog will not show. Wire secrecy
is preserved; nothing new reaches the client.

### 2.3 Re-emit already-in-flight streams when the recipient's own move starts

In the egress loop, when a `MoveStream` arrives whose `mover == ctx.user_id` (the connection's
own move — or, for a GM see-as, whose `mover == see_as.user_id`), after forwarding it: for every
OTHER active stream `A` in the same scene with `A.end_ms > now` and `A.mover != ctx.user_id`,
run `clip_move_stream(A.frame, …)` (which now sees the newly registered timeline) and forward
the result if `Some`. The re-emitted frame is byte-for-byte a normal `MoveStream` with A's
original `request_id`/`start_server_ms`; the client's `TokenAnimator.animateSamples` overwrites
the in-flight playback for that token id and uses `serverNow` catch-up, so it lands mid-flight
at the right elapsed time. No protocol change; no client change.

A re-emit that clips to `None` (the recipient still sees nothing of A) sends nothing — the
existing playback continues unchanged, which is correct because it was itself clipped to
nothing visible or the token would already be showing.

### 2.4 Sub-sample timing

The client cross-fades the fog between consecutive sweep samples; the token gate switches at
the sample boundary. The token may therefore be hidden for up to one inter-sample interval
(≈ 1/3 cell of travel) after the blended fog visually clears its position. Accepted: it is
below the perceptual threshold at normal animation speeds and matches how the mover's own token
already relates to its sweep.

## 3. Components touched

- `src/server/src/ws/room.rs` — `ActiveStream` registry replacing the bare `moving` map; a read
  accessor for "active streams in scene S at time t" used by the egress loop.
- `src/server/src/ws/conn.rs` — `clip_move_stream` / `observer_vision_polys_for_scene` gain the
  per-sample timeline lookup; the egress `MoveStream` arm gains the own-move re-emit pass.
- `src/server/src/ws/protocol.rs` — unchanged (`VisionSample` already carries what 2.2 reads).
- Client — unchanged in production code. A test asserting a re-emitted frame replaces in-flight
  playback at the correct elapsed offset is added if one does not already exist.

## 4. Testing

- Unit (server): a mover R with a 3-sample sweep; a concurrent A whose middle sample is visible
  only from R's sample-1 viewpoint → kept when A starts during R's sweep; dropped under the
  old committed-vision rule (the test that would have passed before must fail on the pre-change
  code — the plan runs it red first).
- Unit (server): both orderings of §1 case 1; GM see-as target with an active sweep; a user
  with two concurrently moving tokens (union); expiry of a stream removes it from clipping.
- Parity: server `chosen sample` rule vs client `chosenSample` on a shared fixture.
- Integration (server ws tests, `clip_move_stream` suite): re-emit is delivered only to the
  connection whose move started, only for streams in the same scene, never to the mover of A.
- Secrecy regression: every existing `clip_*` test still passes; a re-emitted frame to an
  observer never contains `mover_vision`, `cost`, or `truncated`.

## 5. Non-goals

- Case 2 of §1 (third-party light sources) — parked.
- No continuous per-tick recompute loop.
- No change to `vision::visibility_polygon`, `player_vision_polygons`, or the `vision` channel.
