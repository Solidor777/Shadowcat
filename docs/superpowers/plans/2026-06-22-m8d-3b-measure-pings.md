# M8d-3b — Measurement + Pings: Plan (completes M8)

> **For agentic workers:** execute with **`mainline-plan-execution`** (inline enumerative
> per-task spec check + ONE dispatched final branch review; buddy-check it). Checkbox steps.

**Goal:** Client-local **measurement** (drag → distance overlay via the M8d-3a preview API)
and **pings** (§11 — the only M8 **server** work: a new out-of-band broadcast frame mirroring
`AssetChanged`, rendered as transient expanding rings). **Completes M8 → push.**

**Spec:** [`../specs/2026-06-22-m8d-scene-entities-tools-design.md`](../specs/2026-06-22-m8d-scene-entities-tools-design.md)
§10 (measurement), §11 (pings), §14b (the 3a/3b split).

**Server context (ping mirrors `AssetChanged`):** `ClientMsg`/`ServerMsg` are ts-rs enums in
`src/server/src/ws/protocol.rs` (`#[serde(tag="type", rename_all="snake_case")]`, exported on
`cargo test`). Out-of-band relay = `Room::broadcast_aux(msg)` (`src/server/src/ws/room.rs`; no seq,
not ringed). Inbound dispatch = the `match ClientMsg` in `src/server/src/ws/conn.rs` (has
`user_id`, `world_id`, `ctx`, `room`); membership is already gated. Rate limit pattern =
`UploadRateLimiter::check(user, now_ms, per_min)` (`src/server/src/http/assets.rs`), held on
`AppState`. Tests: `protocol.rs` serde round-trip + `tests/ws_convergence.rs` broadcast harness.

## Global constraints
- **#3 ephemerals:** measurement is client-local (no doc, no broadcast); a ping is a transient
  out-of-band frame (no seq, not in the event log, not a document).
- **#6 server structural-only:** the ping relay does NO coordinate validation (just stamps `user`
  + membership-gated relay).
- **#10:** pointer-based; touch-sized. **Testability:** only `pixi-backend.ts` imports `pixi.js`.
- **No raw `console.log`; commit per task.** Push ONLY after M8 (this milestone) is green on CI.

---

### Task 1: `Grid.distance` (square Chebyshev / hex axial)

**Files:** `src/client/render/src/grid.ts` (+`distance`), `index.ts` if needed; test `grid.test.ts`.
- `distance(a: Point, b: Point): number` — square: Chebyshev of cell indices (`cellOf`);
  hex: axial distance `(|dq|+|dr|+|dq+dr|)/2` from the axial `cellOf`. Returns **whole cells**.
- [ ] Test: square 100-grid, (0,0)→(250,40) → 2 cells (Chebyshev); diagonal (0,0)→(250,250) → 2;
  hex distances on known axial pairs. Implement. Pass.
- [ ] **Commit:** `feat(m8d-3b): Grid.distance (square Chebyshev / hex axial)`

---

### Task 2: measure overlay backend + tool-host API

**Files:** `backend.ts`/`backend.mock.ts`/`pixi-backend.ts`; `types.ts`/`engine.ts`/`index.ts`
(`SceneToolHost`); `src/client/ui/src/lib/sceneInteraction.ts`; tests `backend.mock.test.ts`,
`engine.test.ts`, `sceneInteraction.test.ts`.
- Backend: `drawMeasure(from: Point, to: Point, label: string): void` (a line + a centered text
  in the `overlays` layer, a dedicated `measureOverlay` Graphics + `Text`, separate from the tool
  preview) + `clearMeasure(): void`. Mock records `measure: {from,to,label} | null`.
- `SceneToolHost` += `gridDistance(a,b): number` (engine → `Grid.distance`) + `drawMeasure` +
  `clearMeasure`; engine forwards; bridge forwards (no-op detached, gridDistance→0).
- [ ] Test: mock records drawMeasure/clearMeasure; engine.gridDistance delegates to the grid;
  bridge forwards. Implement (Pixi `Text` import is fine — still the only pixi.js file). Pass; pixi grep.
- [ ] **Commit:** `feat(m8d-3b): measure overlay backend + tool-host gridDistance/drawMeasure`

---

### Task 3: measure tool (client-local)

**Files:** `controller.svelte.ts` (+`makeMeasureTool`, `ToolId += "measure"`), `ToolRail.svelte`
(measure button); test `measure-tool.test.ts`.
- `makeMeasureTool(ctx)`: down → anchor (claim gesture, `true`); move → `dist = gridDistance(anchor,p)`,
  `drawMeasure(anchor, p, String(dist))`; up → `clearMeasure()`. No scene needed (works anywhere);
  no document, no intent.
- [ ] Test (fake host capturing drawMeasure/clearMeasure + a `gridDistance` stub): a drag draws the
  measure with the computed label; up clears it; **no dispatchIntent ever fires**.
- [ ] **Commit:** `feat(m8d-3b): measure tool (client-local distance overlay)`

---

### Task 4: server `Ping` frame (Rust)

**Files:** `src/server/src/ws/protocol.rs` (`ClientMsg::Ping`, `ServerMsg::Ping`), `src/server/src/ws/conn.rs`
(ingress arm), `src/server/src/http/mod.rs` + `assets.rs` (a `PingRateLimiter` or reuse the limiter
shape on `AppState`), regenerate ts-rs; tests in `protocol.rs` + `tests/ws_convergence.rs`.
- `ClientMsg::Ping { scene: Uuid, x: f64, y: f64 }`; `ServerMsg::Ping { scene: Uuid, x: f64, y: f64, user: Uuid }`.
- Ingress arm: `ClientMsg::Ping { scene, x, y }` → rate-limit (`state.ping_rate.check(user_id, now, PER_MIN)`,
  e.g. 30/min; drop silently if over) → `room.broadcast_aux(ServerMsg::Ping { scene, x, y, user: user_id })`.
  No coordinate validation (#6). `event_seq()` must return `None` for the new variant (out-of-band).
- Add a `ping_rate: Arc<PingRateLimiter>` to `AppState` (same sliding-window code as upload; a small
  reused/duplicated limiter). Default 30/min, not role-scoped.
- [ ] Rust test: `Ping` serde round-trips snake_case + `event_seq()==None`; a convergence test —
  client sends `{"type":"ping",scene,x,y}`, a second connected member receives `{"type":"ping",...,user}`.
- [ ] `cargo test --all` regenerates `src/types/generated/{ClientMsg,ServerMsg}.ts`; commit those too.
- [ ] **Commit:** `feat(m8d-3b): server Ping out-of-band broadcast frame + rate limit`

---

### Task 5: client wire `Ping` (Zod)

**Files:** `src/client/core/src/wire.ts` (`ClientMsg` += ping; `ServerMsgSchema` += ping); test `wire.test.ts`.
- `ClientMsg` union += `{ type: "ping"; scene: string; x: number; y: number }` (client sends).
- `ServerMsgSchema` += `z.object({ type: z.literal("ping"), scene: z.string(), x: z.number(), y: z.number(), user: z.string() })` (received).
- **Naming clash:** there is already a server `{type:"ping"}` keepalive (no fields). The scene ping
  carries `scene/x/y/user`; the keepalive is bare. Resolve: the server keepalive variant must be
  renamed (e.g. `heartbeat`) OR the scene-ping uses a distinct tag (`scene_ping`). **Use `scene_ping`**
  to avoid touching the keepalive — update Task 4's Rust variant names to `ScenePing` accordingly.
- [ ] Test: a `scene_ping` server frame parses with all fields; the drift guard tags update.
- [ ] **Commit:** `feat(m8d-3b): client wire scene_ping (Zod + drift guard)`

---

### Task 6: `WsClient.sendPing` + `onPing` + WorldSession + AppContext

**Files:** `src/client/core/src/ws-client.ts` (`sendPing`, `onScenePing` handler), `worldSession.svelte.ts`
(ping listener + `sendPing`), `appContext.ts` (`sendPing(x,y)` + `onPing(cb)`), `Table.svelte`, fixtures;
tests `ws-client.test.ts`, `worldSession.test.ts`.
- `WsClient`: `sendPing(scene, x, y)` → `send({type:"scene_ping", scene, x, y})` (guarded by `connected`);
  handle inbound `scene_ping` → `handlers.onScenePing?.({scene,x,y,user})`.
- `WorldSession`: `onPing(cb)` listener set (like `onAssetChanged`) + `sendPing(x,y)` (uses the active
  scene id from `documents.query("scene")[0]`; no-op if no scene / disconnected).
- `AppContext`: `sendPing(x,y): void` + `onPing(cb): () => void`. Populate in Table; fixtures default.
- [ ] Test: `sendPing` transmits a `scene_ping` frame; an inbound `scene_ping` fires `onPing`. Pass.
- [ ] **Commit:** `feat(m8d-3b): WsClient sendPing/onScenePing + WorldSession + AppContext ping seam`

---

### Task 7: `PingView` + backend ping rings (ticker-animated)

**Files:** `src/client/render/src/ping-view.ts` (pure age→ring math), `backend.ts`/mock/pixi
(`drawPings(rings: {x,y,radius,alpha}[])`), `engine.ts` (own a `PingView`; ticker drives it; `addPing(x,y)`
on the host), `types.ts`/`index.ts`, bridge; tests `ping-view.test.ts`, `engine.test.ts`.
- `PingView`: `add(x,y)`; `tick(dtMs): {x,y,radius,alpha}[]` — each ping expands `radius` 0→R and fades
  `alpha` 1→0 over `PING_MS` (~2000); drops when faded. Pure + headless-testable.
- Backend `drawPings(rings)` — redraw a dedicated `pingOverlay` Graphics (in `overlays`, separate from
  measure/tool preview) each call. Mock records `pings: rings`.
- Engine: construct `PingView`; the existing ticker also calls `pingView.tick` → `backend.drawPings`;
  `SceneToolHost += addPing(x,y)` (forwards to `PingView.add`); bridge forwards.
- [ ] Test: `PingView.add` then `tick` yields an expanding/fading ring that settles to empty after
  `PING_MS`; engine `addPing` → a ring renders via the ticker (mock `pings` non-empty), then clears.
- [ ] **Commit:** `feat(m8d-3b): PingView expanding-ring animation + backend ping rings`

---

### Task 8: ping tool + Stage wiring + e2e (completes M8)

**Files:** `controller.svelte.ts` (`makePingTool`, `ToolId += "ping"`), `ToolRail.svelte` (ping button +
measure button), `Stage.svelte` (subscribe `onPing` → `engine.addPing` in scene coords; the ping tool's
`sendPing` via AppContext), `en.ts`; `e2e/stage.spec.ts` (a ping renders); ToolRail test.
- `makePingTool(ctx)`: down → `ctx.sendPing(p)` (scene coords), return `true`; the server echoes the ping
  back to all members (incl. the sender), so the local ring comes through `onPing` → `addPing` like any
  other — no separate local echo.
- Stage: `const offPing = onPing((m) => engine.addPing(m.x, m.y))` in the effect; dispose on teardown.
  (The ping payload x,y are scene coords as sent.)
- ToolRail: add `measure` + `ping` buttons. `sendPing` reaches the ping tool via the `ctx` (add `sendPing`
  to `ToolContext`, sourced from AppContext).
- e2e: as GM, activate Ping, click the canvas → assert a ping rendered (a `data-ping-count`/transient
  signal on the host, or that no error occurs + the ring graphics path ran). Use a `data-last-ping`
  host signal set by Stage's `onPing` handler for determinism.
- [ ] Implement; ToolRail test (measure/ping buttons activate). Pass; `pnpm -r typecheck`/`test`/`lint`;
  `cargo test --all` (server); `pnpm --filter @shadowcat/ui e2e`.
- [ ] **Commit:** `feat(m8d-3b): ping + measure tools + Stage ping wiring + e2e (completes M8)`

---

## Final verification (before the branch review)
- [ ] `pnpm -r typecheck`/`test`/`lint`; `pnpm --filter @shadowcat/ui e2e`; `cargo test --all` (incl. ts-rs sync); `cargo clippy`/`fmt`.
- [ ] `grep -rn "pixi.js" src/client/render/src` → only `pixi-backend.ts`.
- [ ] ts-rs generated `ClientMsg.ts`/`ServerMsg.ts` committed + in sync (CI gate).

## Buddy-check directive
Pings add the **only M8 server frame** (a new out-of-band broadcast + rate limit + ts-rs) and the
animated ping/measure overlays. **Run a buddy-check at the final branch review**, focusing on: the
server ingress arm (membership gate, rate-limit drop, `event_seq()==None`, the `ping` keepalive-vs-
scene_ping tag clash), the ts-rs sync, the `sendPing` connection guard, the PingView age/fade math +
ticker teardown, and measurement being truly client-local (no intent, no broadcast).

## Deviations (surfaced)
- **Rate limit is per-connection, not per-user (Task 4).** Implemented as a local sliding
  window in `handle_socket` rather than a per-user `PingRateLimiter` on `AppState`, avoiding
  5-site `AppState` churn. A defensible merits choice for a transient cosmetic ping (membership-
  gated, silent drop, best-effort relay); weaker only in that N sockets → N×30/min. Buddy-check
  accepted; per-user upgrade logged to `docs/TODO.md`.

## Spec coverage self-check
- §10 measurement (client-local, grid distance) → Tasks 1,2,3.
- §11 pings (out-of-band server frame, transient rings, rate-limited) → Tasks 4,5,6,7,8.
- §12 testability (Rust serde+convergence; headless TS PingView/measure; GL e2e) → all.
- **Completes M8** (M8a server foundation + M8b assets + M8c render + M8d tokens/tools/entities) → **push**.
