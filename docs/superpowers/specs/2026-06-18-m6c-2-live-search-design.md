# M6c-2 — Live Search Subscriptions: Design Spec

> Status: **DRAFT for review.** Second increment of M6c (M6c-1 one-shot search ✅
> merged). M6c-2 adds **live, server-pushed top-N search subscriptions** on top of
> M6c-1's search core and the M4 broadcast path. Completing it finishes M6c and
> the M6 milestone. No UI (M7).

## 1. Goal

Let a client subscribe to a search query and receive the current top-N results
again whenever a world change could affect them — powering a query-backed
"smart list" panel that stays current without re-querying. Live updates reuse
M6c-1's `Repository::search` verbatim, so every push is per-recipient filtered
against the visibility-split index; confidentiality is inherited, not re-solved.

## 2. Invariants preserved

- **Server-authoritative, per-recipient filtering** (ARCHITECTURE #1, #4). Each
  push re-runs `Repository::search` with *that connection's* `PermissionContext`,
  so a live update can never contain a document or field the actor could not get
  from a one-shot search. The M6c-1 visibility-split index (a non-GM matches and
  snippets only GM-only-stripped text) applies unchanged.
- **Structural-only** (ARCHITECTURE #6). No new indexing or body interpretation;
  re-evaluation is the same FTS query.
- **Ordered realtime untouched** (ARCHITECTURE #2). Subscriptions ride the
  existing per-world broadcast as a *trigger* only; `SearchUpdate` frames are not
  sequenced and do not participate in the seq stream or resync.

## 3. Scope

- **Top-N only.** A subscription is a query + a result cap; updates always
  re-deliver the current top-N (cursor `None`). No live pagination (cursors over
  a mutating ranked set are out of scope); deep paging stays a one-shot
  `Core.search`.
- **Full-replace updates.** Each push carries the whole current top-N; the client
  replaces its list. No server-side delta/diff state — except a last-sent
  fingerprint to suppress no-op pushes (§5.3).
- No change to the data layer: M6c-2 is a thin layer over M6c-1's search core and
  M4's broadcast/egress path.

## 4. Protocol & client API

Extends M6c-1's frames (no breaking change):

```rust
// ClientMsg
Search { request_id: Uuid, query: String, limit: u32, cursor: Option<String>, subscribe: bool }
Unsubscribe { request_id: Uuid }
// ServerMsg
SearchUpdate { request_id: Uuid, hits: Vec<SearchHit> }   // top-N; no cursor
```

- `Search { subscribe: true }` runs the search, returns the initial
  `SearchResult` exactly as one-shot, **and** registers a subscription keyed by
  `request_id` (which is its subscription id). `subscribe: false` is the M6c-1
  one-shot behavior, unchanged.
- `SearchUpdate { request_id, hits }` is pushed on each relevant, coalesced
  change (§5).
- `Unsubscribe { request_id }` drops the subscription (idempotent; unknown id
  ignored). All of a connection's subscriptions are dropped on disconnect.

Client (`@shadowcat/core`):

```ts
subscribeSearch(
  query: string,
  opts: { limit?: number },
  onUpdate: (hits: WireSearchHit[]) => void,
): Promise<{ unsubscribe(): void }>
```

The promise resolves with the initial page (and `onUpdate` fires for it);
subsequent `SearchUpdate`s call `onUpdate`. The `WsClient` correlation map (M6c-1)
gains a **persistent** entry kind for subscriptions (not deleted on first reply),
distinct from one-shot entries (deleted on reply). `unsubscribe()` sends
`Unsubscribe` and removes the local entry. On disconnect, in-flight subscribes
reject and active subscriptions are cleared (the M6c-1 `failPending` drain
extends to subscription entries).

## 5. Re-evaluation, cost control, and no-op suppression

The per-connection **egress task** owns the subscription registry (a map
`request_id → { query, limit, last_fingerprint }`) because it already receives
every authoritative `Event` on the per-world broadcast `rx`.

### 5.1 Trigger

An `Event` on `rx` marks the connection's subscriptions dirty and arms a
debounce timer. Subscriptions are not re-run inline per Event.

### 5.2 Coalescing

The debounce window is **150 ms** (server-side, not client-configurable in v1).
A burst of Events (bulk import, rapid drags) triggers **one** re-run per
subscription per window, bounding re-run rate. Live "smart lists" do not need
sub-150 ms freshness.

### 5.3 No-op suppression

After re-running a subscription, the server computes a cheap **fingerprint** of
the result (the ordered list of `(doc_id, score)`) and compares it to the
last-sent fingerprint. If unchanged, **no** `SearchUpdate` is sent. This keeps
unrelated world activity from spamming every subscriber. (This is an equality
check, not a diff — the only per-subscription result state retained.)

### 5.4 Caps

A maximum of **16** active subscriptions per connection. A `Search { subscribe:
true }` beyond the cap is rejected with `SearchError` and not registered.
Combined with M6c-1's per-search `MAX_SCAN`, total work per Event-burst per
connection is bounded by `16 × MAX_SCAN`, re-run at most once per 150 ms.

## 6. Data flow

1. Client → `Search { request_id, query, limit, subscribe: true }`.
2. Server runs the search → `SearchResult { request_id, hits }`; registers
   `{ request_id → { query, limit, last_fingerprint = fingerprint(hits) } }`.
3. A document mutates → authoritative `Event` on `rx` → each connection's egress
   task marks subscriptions dirty + arms the 150 ms timer.
4. Timer fires → for each subscription, re-run `Repository::search` (top-N,
   `cursor = None`) with the connection's `ctx`; if the fingerprint changed, push
   `SearchUpdate { request_id, hits }` and store the new fingerprint.
5. Client `onUpdate(hits)` replaces its list. `Unsubscribe` / disconnect removes
   the subscription.

## 7. Error handling

| Condition | Behavior |
|---|---|
| Re-run errors for a subscription | One `SearchError { request_id }`; drop that subscription; egress loop continues |
| Subscribe beyond the cap | `SearchError { request_id }`; not registered |
| Unknown `Unsubscribe` id | Ignored (idempotent) |
| Disconnect | All subscriptions dropped; client in-flight subscribes rejected |
| Change to an unreadable doc | Re-run yields an unchanged top-N → no-op suppressed (no push) |

## 8. Testing

- **Rust integration:** subscribe returns initial results + registers; a mutation
  that changes the result set pushes a `SearchUpdate` with the new top-N; a
  mutation to a doc the actor cannot read does not leak (and is suppressed as a
  no-op); a no-op change pushes nothing; `Unsubscribe` stops updates; the
  (cap+1)th subscribe is rejected; a burst coalesces into one update.
- **TS unit:** `subscribeSearch` resolves the initial page and fires `onUpdate`;
  a `SearchUpdate` frame calls `onUpdate`; `unsubscribe()` sends the frame and
  stops local dispatch; disconnect drains a pending subscribe.
- **e2e (extends the M6c harness):** a player subscribes; a GM creates a readable
  doc matching the query → the player receives a `SearchUpdate` containing it; a
  GM-only doc matching the query → the player's updates never contain it.

## 9. Execution slices (for the implementation plan)

1. Server: subscription registry + `Search { subscribe }` handling + coalesced
   re-eval + no-op fingerprint suppression + caps + `SearchUpdate` /
   `Unsubscribe` frames (Rust integration-tested).
2. Wire schemas for the new/extended frames + ts-rs regen.
3. Client: `subscribeSearch` + persistent correlation entries + `unsubscribe` +
   disconnect drain (TS unit).
4. e2e live-update + no-leak test + docs sync (completes M6c / M6).

## 10. Decisions settled in brainstorming

1. **Scope** — top-N subscriptions only; no live pagination.
2. **Update model** — full-replace top-N each push (no delta), inheriting M6c-1
   per-recipient filtering.
3. **Re-eval** — in the egress task off the existing broadcast; coalesced with a
   150 ms server-side debounce.
4. **No-op suppression** — skip a push when the `(doc_id, score)` fingerprint is
   unchanged (the only retained per-subscription result state).
5. **Caps** — 16 subscriptions per connection; subscribe-over-cap → `SearchError`.
6. **API** — `Search { subscribe }` returns the initial `SearchResult` then
   pushes `SearchUpdate`s; `Unsubscribe` + disconnect cleanup; client
   `subscribeSearch(query, opts, onUpdate)` on a persistent correlation entry.

## 11. Open decisions (for review)

1. **Debounce value** — 150 ms proposed; revisit if updates feel laggy or too
   chatty in real use.
2. **Cap value** — 16 per connection proposed; raise if real panels need more.
3. **Re-eval scope optimization** — v1 re-runs every active subscription on any
   world Event (within the debounce). A later optimization could skip
   subscriptions a given Event provably cannot affect, but determining that
   approximates re-running the query, so it is deferred.
