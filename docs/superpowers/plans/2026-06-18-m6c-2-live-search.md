# M6c-2 — Live Search Subscriptions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Live, server-pushed top-N search subscriptions: a client subscribes to a query and receives the current top-N again whenever a world change affects it.

**Architecture:** Extends M6c-1 — `Search{subscribe:true}` registers a subscription in the per-connection egress task (which already receives every authoritative Event on the per-world broadcast). On a coalesced (150 ms debounce) Event, the egress task re-runs `Repository::search` with the connection's own `PermissionContext` (so per-recipient filtering + the visibility-split index are inherited) and pushes `SearchUpdate` — unless the result fingerprint is unchanged. No data-layer changes.

**Tech Stack:** Rust + axum + tokio (server egress task); ts-rs; TypeScript + Zod + vitest (`@shadowcat/core`); the M6c-1 Node↔Rust e2e harness + the `tests/ws_convergence.rs`-style Rust WS harness.

## Global Constraints

- **Per-recipient filtering inherited** — every push re-runs `Repository::search` with the connection's `ctx`; a live update can never contain a doc/field a one-shot search wouldn't (ARCHITECTURE #1, #4). Do not reimplement filtering.
- **Top-N only** — subscriptions re-deliver the current top-N (`cursor = None`); no live pagination.
- **Full-replace updates** — each push carries the whole top-N; the only retained per-subscription result state is a `(doc_id, score)` fingerprint for no-op suppression.
- **Coalesce** — Events trigger a re-run at most once per **150 ms** per connection (server-side, not client-configurable).
- **Cap** — max **16** active subscriptions per connection; over-cap subscribe → `SearchError`.
- **Backward compatible** — `subscribe` is `#[serde(default)]`; M6c-1 one-shot `Search` frames (no `subscribe`) keep working unchanged.
- **Ordered realtime untouched** — `SearchUpdate` is not sequenced and does not participate in the seq stream/resync.
- **ts-rs sync** — regenerate `src/types/generated/`; the CI `git diff --exit-code` gate must stay green.
- **No `console.*`** in core; core stays Svelte/DOM-free.

Reference spec: `docs/superpowers/specs/2026-06-18-m6c-2-live-search-design.md`.

---

## Slice 1 — Server: frames + subscription engine

### Task 1: Protocol frames (`subscribe`, `Unsubscribe`, `SearchUpdate`)

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (extend `Search`; add `Unsubscribe`, `SearchUpdate`; fix the existing serialization test)

**Interfaces:**
- Consumes: `SearchHit` (M6c-1).
- Produces:
  - `ClientMsg::Search { request_id, query, limit, cursor, subscribe: bool }` (`subscribe` is `#[serde(default)]`).
  - `ClientMsg::Unsubscribe { request_id: Uuid }`.
  - `ServerMsg::SearchUpdate { request_id: Uuid, hits: Vec<SearchHit> }`.

- [ ] **Step 1: Write the failing test** — add to `protocol.rs` tests:

```rust
    #[test]
    fn subscribe_defaults_false_and_live_frames_round_trip() {
        // A one-shot Search frame (no `subscribe`) still deserializes (default false).
        let oneshot: ClientMsg =
            serde_json::from_str(r#"{"type":"search","request_id":"00000000-0000-0000-0000-000000000001","query":"x","limit":20,"cursor":null}"#).unwrap();
        match oneshot {
            ClientMsg::Search { subscribe, .. } => assert!(!subscribe),
            _ => panic!("expected Search"),
        }
        let unsub = ClientMsg::Unsubscribe { request_id: Uuid::from_u128(1) };
        assert!(serde_json::to_string(&unsub).unwrap().contains("\"type\":\"unsubscribe\""));
        let upd = ServerMsg::SearchUpdate { request_id: Uuid::from_u128(2), hits: Vec::new() };
        assert!(serde_json::to_string(&upd).unwrap().contains("\"type\":\"search_update\""));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat subscribe_defaults_false_and_live_frames_round_trip`
Expected: FAIL — `subscribe` field / variants don't exist.

- [ ] **Step 3: Extend the frames** in `protocol.rs`

In `ClientMsg::Search`, add the field (note `#[serde(default)]`):
```rust
    Search {
        request_id: Uuid,
        query: String,
        limit: u32,
        cursor: Option<String>,
        /// When true, register a live subscription keyed by `request_id`: the
        /// initial `SearchResult` is followed by `SearchUpdate`s on change.
        #[serde(default)]
        subscribe: bool,
    },
```
Add to `ClientMsg`:
```rust
    /// Cancel a live search subscription (idempotent; unknown id ignored).
    Unsubscribe { request_id: Uuid },
```
Add to `ServerMsg`:
```rust
    /// A live subscription's refreshed top-N (full replace). Documents are
    /// already filtered for the recipient.
    SearchUpdate { request_id: Uuid, hits: Vec<SearchHit> },
```

- [ ] **Step 4: Fix the existing M6c-1 construction** — in `protocol.rs` test `search_frames_round_trip`, the `ClientMsg::Search { … }` literal now needs `subscribe: false`:
```rust
        let req = ClientMsg::Search {
            request_id: Uuid::from_u128(1),
            query: "dragon".into(),
            limit: 20,
            cursor: None,
            subscribe: false,
        };
```

- [ ] **Step 5: Run tests + regen**

Run: `cargo test -p shadowcat subscribe_defaults_false_and_live_frames_round_trip search_frames_round_trip`
Expected: PASS.
Run: `cargo test -p shadowcat` then confirm `git status src/types/generated` shows `ClientMsg.ts`, `ServerMsg.ts` updated.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/ws/protocol.rs src/types/generated/ClientMsg.ts src/types/generated/ServerMsg.ts
git commit -m "feat(m6c-2): live-search protocol frames (Search.subscribe, Unsubscribe, SearchUpdate)"
```

---

### Task 2: Egress subscription engine (registry + coalesced re-eval + suppression + caps)  *(concurrency/security-critical)*

**Files:**
- Modify: `src/server/src/ws/conn.rs` (`Egress` variants; ingress routing; egress registry + debounce + re-eval)
- Create: `src/server/tests/ws_live_search.rs` (integration)

**Interfaces:**
- Consumes: `Repository::search` (M6c-1), `SearchHit`, the `Egress` mpsc channel, `rx` broadcast.
- Produces: live `SearchUpdate` pushes; `Egress::Subscribe`/`Egress::Unsubscribe` internal variants.

**Design notes (read before coding):**
- The subscription registry lives in the **egress task** (it owns `rx` and the sink). Ingress forwards subscribe/unsubscribe over the existing `Egress` mpsc channel.
- One-shot search stays in ingress (unchanged). `subscribe:true` routes entirely to egress: egress enforces the cap, runs the initial search (→ `SearchResult`), registers `{query, limit, last_fingerprint}`.
- Re-eval is coalesced: an Event arms a 150 ms deadline; a `select!` timer branch fires once per window and re-runs every subscription. Push only when the `(doc_id, score)` fingerprint changed.

- [ ] **Step 1: Write the failing integration test** (`src/server/tests/ws_live_search.rs`)

```rust
// Live search: a subscriber receives SearchUpdate when a matching doc appears,
// and never sees a document it cannot read. Mirrors the ws_convergence harness.
mod common; // if the harness is shared; otherwise inline a minimal spawn()
```

> If `ws_convergence.rs` keeps its harness private, replicate its `spawn()` /
> `connect_with` / `login` / `add_member` helpers at the top of this file (copy
> the ~120-line harness prelude verbatim). Then:

```rust
#[tokio::test]
async fn live_subscription_pushes_update_on_matching_create() {
    let h = spawn().await;
    let gm_cookie = h.login("gm", "pw").await;            // GM seeded by harness
    let pl_cookie = h.add_member("pl", WorldRole::Player).await; // returns a logged-in cookie

    // Player subscribes to "dragon".
    let mut sub = h.connect_with(&pl_cookie).await;
    drain_welcome(&mut sub).await;
    send_json(&mut sub, serde_json::json!({
        "type": "search", "request_id": "11111111-1111-1111-1111-111111111111",
        "query": "dragon", "limit": 20, "cursor": null, "subscribe": true
    })).await;
    let initial = drain_one_of_type(&mut sub, "search_result").await;
    assert_eq!(initial["hits"].as_array().unwrap().len(), 0);

    // GM creates a readable (default observer) doc matching "dragon".
    let mut gm = h.connect_with(&gm_cookie).await;
    drain_welcome(&mut gm).await;
    send_json(&mut gm, create_intent_json(h.world, "Red Dragon", "observer")).await;

    // Player receives a SearchUpdate containing it (within the debounce window).
    let upd = drain_one_of_type(&mut sub, "search_update").await;
    assert_eq!(upd["hits"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn live_subscription_never_pushes_unreadable_docs() {
    let h = spawn().await;
    let gm_cookie = h.login("gm", "pw").await;
    let pl_cookie = h.add_member("pl", WorldRole::Player).await;

    let mut sub = h.connect_with(&pl_cookie).await;
    drain_welcome(&mut sub).await;
    send_json(&mut sub, serde_json::json!({
        "type": "search", "request_id": "22222222-2222-2222-2222-222222222222",
        "query": "secret", "limit": 20, "cursor": null, "subscribe": true
    })).await;
    drain_one_of_type(&mut sub, "search_result").await;

    // GM creates a GM-only (default none) doc matching "secret".
    let mut gm = h.connect_with(&gm_cookie).await;
    drain_welcome(&mut gm).await;
    send_json(&mut gm, create_intent_json(h.world, "Secret Item", "none")).await;

    // The player gets no search_update (no-op suppressed — empty top-N unchanged).
    assert!(no_frame_of_type(&mut sub, "search_update", Duration::from_millis(600)).await);
}
```

> Implement the small JSON helpers (`send_json`, `drain_welcome`,
> `drain_one_of_type`, `no_frame_of_type`, `create_intent_json`) using the same
> tungstenite `Message::Text` patterns as `ws_convergence.rs` (`drain_frames`,
> `intent_msg`, `create_op`). `create_intent_json(world, name, default_role)`
> builds a `{type:intent,...}` with a `create` op whose `permissions.default` is
> the given role and `system.name` is `name`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat --test ws_live_search`
Expected: FAIL — `subscribe` not handled; no `search_update` ever arrives.

- [ ] **Step 3: Add the `Egress` variants** in `conn.rs`:

```rust
enum Egress {
    Frame(Arc<ServerMsg>),
    TimePong { client_t0: i64, server_t: i64 },
    Resync(i64),
    Subscribe { request_id: Uuid, query: String, limit: u32 },
    Unsubscribe { request_id: Uuid },
}
```

- [ ] **Step 4: Route subscribe/unsubscribe from ingress** — in the ingress `match` (where `ClientMsg::Search` is handled), branch on `subscribe`:

```rust
                        Ok(ClientMsg::Search { request_id, query, limit, cursor, subscribe }) => {
                            if subscribe {
                                if etx.send(Egress::Subscribe { request_id, query, limit }).await.is_err() {
                                    break;
                                }
                            } else {
                                let from = cursor.as_deref().and_then(|c| c.parse::<i64>().ok());
                                let frame = match repo.search(&ctx, world_id, &query, limit, from).await {
                                    Ok(page) => ServerMsg::SearchResult {
                                        request_id,
                                        hits: page.hits,
                                        next_cursor: page.next_cursor.map(|n| n.to_string()),
                                    },
                                    Err(e) => {
                                        tracing::debug!(world = %world_id, %request_id, error = %e, "search failed");
                                        ServerMsg::SearchError { request_id, message: "search failed".into() }
                                    }
                                };
                                if etx.send(Egress::Frame(Arc::new(frame))).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Ok(ClientMsg::Unsubscribe { request_id }) => {
                            if etx.send(Egress::Unsubscribe { request_id }).await.is_err() {
                                break;
                            }
                        }
```

- [ ] **Step 5: Add the registry + helpers to the egress task** — near the top of `egress_loop`, after `world_reqs`:

```rust
    use std::collections::HashMap;
    const MAX_SUBSCRIPTIONS: usize = 16;
    const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(150);

    struct Sub {
        query: String,
        limit: u32,
        fingerprint: Vec<(Uuid, u64)>,
    }
    fn fingerprint(hits: &[crate::data::search::SearchHit]) -> Vec<(Uuid, u64)> {
        hits.iter().map(|h| (h.document.id, h.score.to_bits())).collect()
    }

    let mut subs: HashMap<Uuid, Sub> = HashMap::new();
    let mut deadline: Option<tokio::time::Instant> = None;
```

- [ ] **Step 6: Handle `Subscribe`/`Unsubscribe` in the `erx.recv()` arm** — add arms alongside `Egress::Frame`:

```rust
                Some(Egress::Subscribe { request_id, query, limit }) => {
                    if subs.len() >= MAX_SUBSCRIPTIONS {
                        let f = ServerMsg::SearchError { request_id, message: "too many subscriptions".into() };
                        if sink.send(text(&f)).await.is_err() { break; }
                    } else {
                        match repo.search(&ctx, world_id, &query, limit, None).await {
                            Ok(page) => {
                                let fp = fingerprint(&page.hits);
                                let f = ServerMsg::SearchResult { request_id, hits: page.hits, next_cursor: None };
                                if sink.send(text(&f)).await.is_err() { break; }
                                subs.insert(request_id, Sub { query, limit, fingerprint: fp });
                            }
                            Err(e) => {
                                tracing::debug!(world = %world_id, %request_id, error = %e, "subscribe search failed");
                                let f = ServerMsg::SearchError { request_id, message: "search failed".into() };
                                if sink.send(text(&f)).await.is_err() { break; }
                            }
                        }
                    }
                }
                Some(Egress::Unsubscribe { request_id }) => {
                    subs.remove(&request_id);
                }
```

- [ ] **Step 7: Arm the debounce when an Event is delivered** — in the `rx.recv()` `Ok(msg)` arm, after a `command`/event frame is sent (i.e. when `msg.event_seq().is_some()`), arm the deadline if there are subscriptions. Add right after `next_expected = seq + 1;`:

```rust
                        if !subs.is_empty() {
                            deadline = Some(tokio::time::Instant::now() + DEBOUNCE);
                        }
```

- [ ] **Step 8: Add the debounce timer branch to the `select!`** — add a third branch:

```rust
            _ = tokio::time::sleep_until(deadline.unwrap_or_else(tokio::time::Instant::now)),
                if deadline.is_some() =>
            {
                deadline = None;
                let mut dead: Vec<Uuid> = Vec::new();
                for (id, sub) in subs.iter_mut() {
                    match repo.search(&ctx, world_id, &sub.query, sub.limit, None).await {
                        Ok(page) => {
                            let fp = fingerprint(&page.hits);
                            if fp != sub.fingerprint {
                                sub.fingerprint = fp;
                                let f = ServerMsg::SearchUpdate { request_id: *id, hits: page.hits };
                                if sink.send(text(&f)).await.is_err() { return; }
                            }
                        }
                        Err(e) => {
                            tracing::debug!(world = %world_id, subscription = %id, error = %e, "live re-eval failed");
                            let f = ServerMsg::SearchError { request_id: *id, message: "search failed".into() };
                            let _ = sink.send(text(&f)).await;
                            dead.push(*id);
                        }
                    }
                }
                for id in dead { subs.remove(&id); }
            }
```

(`return;` on a send error ends the egress task, matching the existing `break`-on-send-error behavior; the function returns `()`.)

- [ ] **Step 9: Run the integration tests**

Run: `cargo test -p shadowcat --test ws_live_search`
Expected: PASS — subscriber gets an update on a matching readable create; no update for an unreadable create.

- [ ] **Step 10: Full sweep + commit**

Run: `cargo test -p shadowcat && cargo clippy --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: PASS / clean.

```bash
git add src/server/src/ws/conn.rs src/server/tests/ws_live_search.rs
git commit -m "feat(m6c-2): egress live-search subscriptions (registry, 150ms coalesced re-eval, no-op suppression, cap)"
```

---

## Slice 2 — Client

### Task 3: Wire schemas for the live frames

**Files:**
- Modify: `src/client/core/src/wire.ts` (Search `subscribe`; `Unsubscribe`; `search_update`)
- Modify: `src/client/core/src/wire.test.ts`

**Interfaces:**
- Produces: `ServerMsg` gains `search_update`; `ClientMsg` gains `subscribe` on `search` and an `unsubscribe` variant.

- [ ] **Step 1: Write the failing test** (`wire.test.ts`, in the `parseServerMsg` describe)

```ts
  it("parses a search_update frame", () => {
    const m = parseServerMsg(
      JSON.stringify({ type: "search_update", request_id: "r1", hits: [] }),
    );
    expect(m?.type).toBe("search_update");
  });
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/core test -- wire`
Expected: FAIL — `search_update` rejected.

- [ ] **Step 3: Extend `wire.ts`**

Add to the `ServerMsgSchema` union:
```ts
  z.object({
    type: z.literal("search_update"),
    request_id: z.string(),
    hits: z.array(SearchHitSchema),
  }),
```
Update the `ClientMsg` `search` variant and add `unsubscribe`:
```ts
  | {
      type: "search";
      request_id: string;
      query: string;
      limit: number;
      cursor?: string;
      subscribe: boolean;
    }
  | { type: "unsubscribe"; request_id: string };
```

- [ ] **Step 4: Run test + typecheck**

Run: `pnpm --filter @shadowcat/core test -- wire && pnpm --filter @shadowcat/core typecheck`
Expected: PASS (typecheck will then flag the M6c-1 `search()` call missing `subscribe` — fixed in Task 4).

> Note: typecheck may now error in `ws-client.ts` because the existing one-shot
> `search()` builds a `search` frame without `subscribe`. That is fixed in Task 4
> (add `subscribe: false`). If running typecheck standalone here, expect that one
> error; it clears in Task 4.

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/wire.ts src/client/core/src/wire.test.ts
git commit -m "feat(m6c-2): wire schemas for live-search frames"
```

---

### Task 4: `WsClient.subscribeSearch` + correlation + one-shot fix

**Files:**
- Modify: `src/client/core/src/ws-client.ts` (subscriptions map; `subscribeSearch`; `search_update` handling; one-shot `search()` sets `subscribe:false`; disconnect drain)
- Modify: `src/client/core/src/ws-client.test.ts`
- Modify: `src/client/core/src/index.ts` (export `SubscriptionHandle`)

**Interfaces:**
- Consumes: `WireSearchHit`, the M6c-1 `pending` map + `failPending`.
- Produces: `subscribeSearch(query, opts: { limit?: number; timeoutMs?: number }, onUpdate: (hits: WireSearchHit[]) => void): Promise<SubscriptionHandle>` where `interface SubscriptionHandle { unsubscribe(): void }`.

- [ ] **Step 1: Write the failing tests** (`ws-client.test.ts`)

```ts
  it("subscribeSearch resolves, fires onUpdate for initial + updates, and unsubscribe stops dispatch", async () => {
    const sent: string[] = [];
    let onMessage: (d: string) => void = () => {};
    const client = new WsClient({
      connect: (h) => {
        onMessage = h.onMessage;
        return Promise.resolve({ send: (d) => sent.push(d), close: () => {} });
      },
      handlers: noop,
    });
    await client.start();
    const updates: number[] = [];
    const handle = await new Promise<{ unsubscribe(): void }>((res) => {
      void client.subscribeSearch("dragon", { limit: 5 }, (hits) => updates.push(hits.length)).then(res);
      const req = JSON.parse(sent.find((s) => JSON.parse(s).type === "search")!);
      expect(req.subscribe).toBe(true);
      onMessage(JSON.stringify({ type: "search_result", request_id: req.request_id, hits: [], next_cursor: null }));
    });
    // initial fired
    expect(updates).toEqual([0]);
    const reqId = JSON.parse(sent.find((s) => JSON.parse(s).type === "search")!).request_id;
    onMessage(JSON.stringify({ type: "search_update", request_id: reqId, hits: [{}] }));
    expect(updates).toEqual([0, 1]);
    // unsubscribe sends the frame and stops dispatch
    handle.unsubscribe();
    expect(sent.some((s) => JSON.parse(s).type === "unsubscribe")).toBe(true);
    onMessage(JSON.stringify({ type: "search_update", request_id: reqId, hits: [{}, {}] }));
    expect(updates).toEqual([0, 1]); // no further dispatch
  });
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/core test -- ws-client`
Expected: FAIL — `subscribeSearch` not defined.

- [ ] **Step 3: Implement in `ws-client.ts`**

Add a subscriptions map field:
```ts
  private subscriptions = new Map<string, (hits: WireSearchHit[]) => void>();
```
Fix the one-shot `search()` frame to set `subscribe: false`:
```ts
      this.send({
        type: "search",
        request_id,
        query,
        limit: opts.limit ?? 20,
        cursor: opts.cursor,
        subscribe: false,
      });
```
Add the API + handle type:
```ts
export interface SubscriptionHandle {
  unsubscribe(): void;
}

  /**
   * Core.subscribeSearch — live top-N search. Resolves once the initial result
   * arrives (and fires `onUpdate` for it); subsequent server pushes fire
   * `onUpdate(hits)`. `unsubscribe()` stops updates and tells the server.
   */
  subscribeSearch(
    query: string,
    opts: { limit?: number; timeoutMs?: number },
    onUpdate: (hits: WireSearchHit[]) => void,
  ): Promise<SubscriptionHandle> {
    const request_id = crypto.randomUUID();
    const timeoutMs = opts.timeoutMs ?? 10_000;
    this.subscriptions.set(request_id, onUpdate);
    return new Promise<SubscriptionHandle>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(request_id);
        this.subscriptions.delete(request_id);
        reject(new Error("subscribe request timeout"));
      }, timeoutMs);
      this.pending.set(request_id, {
        resolve: (page) => {
          onUpdate(page.hits);
          resolve({
            unsubscribe: () => {
              this.subscriptions.delete(request_id);
              this.send({ type: "unsubscribe", request_id });
            },
          });
        },
        reject,
        timer,
      });
      this.send({
        type: "search",
        request_id,
        query,
        limit: opts.limit ?? 20,
        cursor: undefined,
        subscribe: true,
      });
    });
  }
```
Add `search_update` handling in `handleFrame` (alongside `search_result`):
```ts
      case "search_update": {
        this.subscriptions.get(msg.request_id)?.(msg.hits);
        break;
      }
```
Drain subscriptions on disconnect — extend `failPending` to also clear subscriptions:
```ts
  private failPending(reason: string): void {
    for (const p of this.pending.values()) {
      clearTimeout(p.timer);
      p.reject(new Error(reason));
    }
    this.pending.clear();
    this.subscriptions.clear();
  }
```

> The `pending` entry's `resolve` type is `(page: SearchPage) => void` (M6c-1).
> `subscribeSearch` reuses it for the initial `SearchResult`; the `search_result`
> handler already resolves+deletes the pending entry, leaving the persistent
> `subscriptions` entry in place for subsequent `SearchUpdate`s.

- [ ] **Step 4: Run tests + typecheck**

Run: `pnpm --filter @shadowcat/core test -- ws-client && pnpm --filter @shadowcat/core typecheck`
Expected: PASS (the Task 3 one-shot typecheck error is now resolved).

- [ ] **Step 5: Export + full client sweep + commit**

Add to `index.ts`: `SubscriptionHandle` to the `ws-client` type exports.
Run: `pnpm --filter @shadowcat/core test && pnpm --filter @shadowcat/core typecheck && pnpm lint`
Expected: PASS / clean.

```bash
git add src/client/core/src/ws-client.ts src/client/core/src/ws-client.test.ts src/client/core/src/index.ts
git commit -m "feat(m6c-2): WsClient.subscribeSearch (live updates, unsubscribe, disconnect drain)"
```

---

## Slice 3 — e2e + docs

### Task 5: e2e live-update + no-leak test

**Files:**
- Create: `src/client/core/src/e2e/live-search.e2e.test.ts`

**Interfaces:**
- Consumes: `startTestServer`, `login` (M6c harness); `WsClient.subscribeSearch`. Two cookies (gm + pl) from the fixture.

- [ ] **Step 1: Write the e2e test**

```ts
import { afterAll, beforeAll, expect, test } from "vitest";
import WebSocket from "ws";
import { WsClient } from "../ws-client";
import type { Transport, TransportHandlers } from "../transport";
import type { WireSearchHit, ClientMsg } from "../wire";
import { startTestServer, login, type TestServer } from "./server-process";

let server: TestServer;
beforeAll(async () => { server = await startTestServer(); });
afterAll(() => server?.stop());

function nodeConnect(wsUrl: string, world: string, cookie: string) {
  return (handlers: TransportHandlers): Promise<Transport> =>
    new Promise((resolve, reject) => {
      const sock = new WebSocket(`${wsUrl}?world=${world}`, { headers: { cookie } });
      sock.on("open", () => resolve({ send: (d: string) => sock.send(d), close: () => sock.close() }));
      sock.on("message", (d) => handlers.onMessage(d.toString()));
      sock.on("close", () => handlers.onClose());
      sock.on("error", reject);
    });
}
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

test("a player's live subscription updates on a readable create and never leaks GM-only docs", async () => {
  const plCookie = await login(server.baseUrl, "pl", "pw");
  const gmCookie = await login(server.baseUrl, "gm", "pw");
  const { world } = server.fixture;

  const player = new WsClient({ connect: nodeConnect(server.wsUrl, world, plCookie), handlers: { onCommand: () => {} } });
  await player.start();
  await sleep(300);

  const updates: WireSearchHit[][] = [];
  await player.subscribeSearch("griffon", { limit: 20 }, (hits) => updates.push(hits));
  // initial is empty
  expect(updates.at(-1)).toEqual([]);

  // GM connects and creates one readable + one GM-only doc, both matching "griffon".
  const gm = new WsClient({ connect: nodeConnect(server.wsUrl, world, gmCookie), handlers: { onCommand: () => {} } });
  await gm.start();
  await sleep(300);
  const mk = (id: string, name: string, role: "observer" | "none"): ClientMsg => ({
    type: "intent",
    intent_id: id,
    ops: [{
      op: "create",
      doc: {
        id, scope: { kind: "world", world_id: world }, doc_type: "actor", schema_version: 1,
        source: null, owner: null,
        permissions: { default: role, users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
        embedded: {}, system: { name }, created_at: 0, updated_at: 0,
      },
    }],
  });
  gm.send(mk("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "Readable Griffon", "observer"));
  gm.send(mk("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb", "Secret Griffon", "none"));

  // Within the debounce window the player gets an update with ONLY the readable one.
  await sleep(800);
  const last = updates.at(-1)!;
  const blob = JSON.stringify(last);
  expect(blob.includes("Readable Griffon")).toBe(true);
  expect(blob.includes("Secret Griffon")).toBe(false);

  gm.stop();
  player.stop();
});
```

- [ ] **Step 2: Run the e2e suite**

Run: `pnpm --filter @shadowcat/core test:e2e`
Expected: PASS (all three e2e suites: capabilities, search, live-search).

- [ ] **Step 3: Commit**

```bash
git add src/client/core/src/e2e/live-search.e2e.test.ts
git commit -m "test(m6c-2): e2e live subscription updates on readable create, never leaks GM-only"
```

---

### Task 6: Documentation sync

**Files:**
- Modify: `docs/PLAN.md` (M6c-2 ✅; M6c and M6 complete)

- [ ] **Step 1: Update `docs/PLAN.md`** — mark `M6c-2 ✅` and note M6c (and the M6 milestone) complete, pointing to the spec/plan.

- [ ] **Step 2: Final verification sweep**

Run: `cargo test -p shadowcat && cargo clippy --all-targets -- -D warnings && cargo fmt --all -- --check`
Run: `pnpm -r typecheck && pnpm -r test && pnpm lint`
Run: `pnpm --filter @shadowcat/core test:e2e`
Expected: all PASS / clean.

- [ ] **Step 3: Commit + graphify**

```bash
git add docs/PLAN.md
git commit -m "docs(m6c-2): mark live search complete; M6c and M6 done"
```
Run: `graphify update .`

---

## Self-Review

**Spec coverage:**
- §4 frames (`Search.subscribe`, `Unsubscribe`, `SearchUpdate`) + client `subscribeSearch` → Tasks 1, 3, 4. ✓
- §5.1–5.4 registry / trigger / 150 ms coalescing / no-op fingerprint / 16-cap → Task 2 (steps 5–8). ✓
- §5 per-recipient re-eval (reuses `Repository::search` with `ctx`) → Task 2 step 8. ✓
- §6 data flow (subscribe → initial SearchResult → SearchUpdate on change) → Task 2 steps 6/8, Task 4. ✓
- §7 error handling (re-eval error drops sub + SearchError; cap → SearchError; unknown unsub ignored; disconnect drops) → Task 2 steps 6/8, Task 4 (drain). ✓
- §8 testing (Rust integration, TS unit, e2e) → Tasks 2, 4, 5. ✓
- Backward-compat (`#[serde(default)] subscribe`) → Task 1. ✓

**Placeholder scan:** No TBD/vague steps. The harness-helper note in Task 2 step 1 ("replicate `ws_convergence.rs`'s `spawn()`/`login`/`connect_with`") references concrete existing functions to copy, not deferred work; the JSON helpers are named with their exact behavior.

**Type consistency:** `SearchUpdate { request_id, hits }` consistent across protocol.rs, wire.ts (`search_update`), and the `search_update` handler. `Search` gains `subscribe: bool` (Rust) / `subscribe: boolean` (wire.ts) / sent as `subscribe: false` (one-shot) / `true` (subscribe). `fingerprint(hits) -> Vec<(Uuid, u64)>` defined and used only in Task 2. `subscribeSearch(query, opts, onUpdate) -> Promise<SubscriptionHandle>` matches between Task 4 def and its test, and `SubscriptionHandle { unsubscribe() }` is exported. The `pending` entry `resolve: (page: SearchPage) => void` reused for the initial result matches M6c-1.

## Buddy-check directives

Task 2 (the egress subscription engine) is **concurrency- and security-sensitive**: it mutates per-connection state inside the `select!` loop, adds a debounce-timer branch, runs `Repository::search` re-evaluations that push per-recipient-filtered results, and must clean up on disconnect — a defect could leak another recipient's data, wedge the egress loop, or unbounded-push. At the execution handoff, OFFER a buddy-check (`superpowers:buddy-checking`) over Task 2 before merge; the remaining tasks (frames, wire schemas, client API, e2e, docs) suit the standard single final-branch review. The human decides whether to take the offer.

**Decision (accepted):** Execute inline (superpowers:executing-plans) with checkpoints. Run a buddy-check (`superpowers:buddy-checking`) over Task 2 (the egress subscription engine) before merge; standard single final-branch review for the remainder.
