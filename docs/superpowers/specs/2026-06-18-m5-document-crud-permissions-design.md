# M5 — Document CRUD + Permissions + Server-side Rollback: Design

Status: approved (brainstorm). Date: 2026-06-18.
Roadmap: [`docs/PLAN.md`](../../PLAN.md) M5. Architecture source of truth: [`docs/design/ARCHITECTURE.md`](../../design/ARCHITECTURE.md).

## 1. Goal

Build the authoritative document read/write path on top of M2's data layer and M4's event bus: per-world membership/roles, a `PermissionContext` that gates reads/writes and filters every broadcast per recipient, document CRUD over HTTP **and** WebSocket intents through one core write path, field-level optimistic concurrency (intent / confirm / reject) over the existing pre-image substrate, compendium/world/embedded copy independence, asset-UUID references, and wiring the dormant `validation.rs` into the write path.

## 2. Scope & non-goals

**In scope**
- `world_members` table + per-world roles (GM / player / spectator); world creation; membership management.
- `PermissionContext` (per-connection and per-request) gating writes, reads, and broadcast filtering.
- Document CRUD: WS `Intent` (realtime) and HTTP `POST/PATCH/DELETE` + `GET`, both through one core.
- Field-level optimistic concurrency: per-op pre-image check inside the apply transaction → confirm or `Reject{Conflict}`.
- Per-recipient broadcast filtering with seq-preserving redaction (invariant #4).
- Compendium → world and embedded copy independence (deep copy, fresh UUIDs, `Source` provenance).
- Asset-UUID references as a validated data-model property.
- Wire `validation::validate_system_size` + `validate_field_path` into the write path (resolves `docs/TODO.md` item).

**Out of scope (and why)**
- **Client-side optimistic-apply + rollback UX, the Zod client store** — M6. M5 ships the *server* contract (confirm/reject + reversible commands) that M6 consumes.
- **Doc-state snapshot resync tier** (invariant #2's "full snapshot") — M6. M5 keeps M4's hot-ring → cold-`events_since` replay; the snapshot seam is documented, not built.
- **Asset upload / serving surface** — M8. M5 only validates UUID references.
- **Semantic validation of the `system` body** — never (invariant #6): structural only (size cap, field-path validity, `deny_unknown_fields`).

## 3. Schema & membership

### `world_members` table (exists since `0001_init.sql`)
```sql
CREATE TABLE world_members (
  world_id  TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
  user_id   TEXT NOT NULL REFERENCES users(id)  ON DELETE CASCADE,
  role      TEXT NOT NULL,                  -- 'gm' | 'player' | 'spectator'
  PRIMARY KEY (world_id, user_id)
);
```
- The table already exists (column **`role`**) with `add_member`/`member_role` from M2; M5 reuses it — **no new migration**. Implementation note: the originally-planned `0003_world_members.sql` was dropped on discovering the table.
- `role` serializes via the existing `WorldRole` serde (`gm`/`player`/`spectator`).
- **World creation** lands in M5: `SqliteRepository::create_world` (already inherent) gains an HTTP surface; the creating user is inserted as the world's first **GM** in the same transaction.
- **Membership management:** a server admin or a world GM may add/remove members and set roles.

### `data/membership.rs`
`Repository`-level (or inherent) queries: `add_member(world, user, role)`, `remove_member(world, user)`, `set_role(world, user, role)`, `world_role(world, user) -> Option<WorldRole>`, `list_members(world)`. Server admins resolve to GM without a row.

## 4. PermissionContext

```rust
pub struct PermissionContext { pub user_id: Uuid, pub world_role: WorldRole }
```
Built from `world_members` (server admin ⇒ GM). One object, three uses:
- **writes:** `resolve_access(ctx.user_id, ctx.world_role, doc).can_write` before applying any op to a doc;
- **reads:** `filter_properties(doc, access)` on query results;
- **broadcasts:** held by each WS egress task to filter every outgoing command (see §6).

Constructed once per WS connection at join (from the joined world's membership row) and once per HTTP request (from session + membership). Lives in `permission.rs` beside `resolve_access` / `filter_properties`.

## 5. Write path — one core, two entry points, field-level OCC

A single ordering-guarded core generalizes M4's `Room::publish` from empty-ops to real, checked ops. The per-world publish guard (M4 invariant: broadcast order == seq order) still wraps the whole critical section.

```
Room::publish(repo, ctx, ops, intent_id)            // holds the per-world publish guard
  ├─ cmd = repo.apply_intent(ctx, world_id, ops)     // ONE transaction:
  │     for each op:
  │       load target doc
  │       resolve_access(ctx, doc).can_write          else Reject(Forbidden)
  │       structural validation (system size, field-path validity)   // wires validation.rs
  │       Update: every FieldChange.old == current value at its path else Reject(Conflict)
  │       Create: id absent (else Conflict) · Delete: id present (else Conflict)
  │     all pass → apply ops, allocate seq, append world_events       (atomic; else rollback)
  ├─ ring.push(Event{command: cmd})
  └─ broadcast(Event{command: cmd, intent_id})
```

- **Confirm:** success broadcasts `Event{command, intent_id}`; the originating client matches `intent_id`.
- **Reject:** `Reject{intent_id, reason}` (`Forbidden` | `Conflict` | `Invalid`) to the originator only; the transaction rolled back, authoritative state untouched, nothing broadcast.
- **No TOCTOU:** authorize + validate + pre-image check + apply + seq + event all execute inside `apply_intent`'s single transaction. The existing single-writer SQLite pool serializes apply across worlds; the publish guard serializes the broadcast send per world.
- **Field-level merge:** concurrent intents on *different* fields both pass (their pre-images match); same-field races → the loser gets `Conflict` and reconciles to authoritative.
- **Entry points:** WS `ClientMsg::Intent{intent_id, ops}` (realtime) and HTTP `POST /api/worlds/:id/documents` (create), `PATCH /api/documents/:id` (update), `DELETE /api/documents/:id` (delete). All call `Room::publish`, so HTTP writes broadcast to WS subscribers too. HTTP responses carry the confirmed `Command` or the `Reject` reason as a status code (403/409/422).
- M4's `EmitTest` intent is **retired**; real intents replace it (the convergence harness switches to real create/update intents).

## 6. Per-recipient broadcast filtering (invariant #4)

The broadcast carries the full authoritative command; filtering is per recipient in the egress task, which holds the connection's `PermissionContext`:
- drop ops whose document the recipient cannot read (`!can_read`);
- for readable docs, strip `GmOnly` properties (`filter_properties`) and drop `FieldChange`s whose path carries a `GmOnly` override;
- **seq-preserving redaction:** a fully-hidden event still ships as `Event{command}` with the same seq and empty `ops`, so the recipient's sequence guard never sees a false gap. Resync replay (M4 hot/cold tiers) applies the identical filter.

A `permission.rs` helper `filter_command(cmd, ctx) -> Command` performs this op-level redaction (reusing `filter_properties` for each document).

## 7. Read surface

Per-recipient filtered HTTP GET:
- `GET /api/documents/:id` → the filtered document, or 404 if `!can_read`.
- `GET /api/worlds/:id/documents?type=<doc_type>` → filtered list.

Initial client load = GET current docs, then WS-subscribe from the returned seq and apply deltas. Doc-snapshot resync tier deferred to M6.

## 8. Copy independence & assets

- **Copy independence** (ARCHITECTURE §6): independence is inherent in the row-per-document store — each document (including each embedded copy, stored inline) is its own record, so mutating a world/embedded copy cannot reach the compendium template or source. M5's contributions: (a) the **client** assigns a **fresh UUID** and a `Source { id, pack, version }` provenance link when instantiating from a template (the GM is authoritative for module-computed content, invariant #6); (b) the server's `Operation::Create` enforces **id-absent** (a colliding id → `Conflict`, never an overwrite) and that the document's scope matches the command's world; (c) the `Source` link is persisted but never followed for writes. The server does not auto-deep-copy; it guarantees no id collision and faithful storage of what the client sends.
- **Asset references:** documents reference assets by stable UUID (validated as a structural data-model property). Upload/serving is M8; M5 only ensures references are well-formed UUIDs.

## 9. Components (files)

| File | Responsibility |
|---|---|
| `migrations/0003_world_members.sql` (create) | membership table |
| `src/server/src/data/membership.rs` (create) | world_members queries + role lookup |
| `src/server/src/data/permission.rs` (modify) | add `PermissionContext`, `filter_command` |
| `src/server/src/data/repository.rs` + `sqlite.rs` (modify) | `apply_intent` (checked, transactional); world-create returns first-GM; membership methods |
| `src/server/src/ws/room.rs` (modify) | generalize `publish` to real ops via `apply_intent` |
| `src/server/src/ws/conn.rs` (modify) | build `PermissionContext` at join; filter live + resync sends |
| `src/server/src/ws/protocol.rs` (modify) | `Intent`, `Reject`; retire `EmitTest` |
| `src/server/src/http/routes.rs` (modify) | document CRUD, membership, world-create handlers |
| `src/server/tests/ws_convergence.rs` (modify) | intents replace EmitTest; add permission/conflict cases |

## 10. Testing

**Unit**
- `apply_intent`: pre-image match applies; mismatch → `Conflict`; unauthorized → `Forbidden`; oversized system / bad field-path → `Invalid`.
- copy independence: mutating a world copy leaves the compendium template byte-identical.
- `filter_command`: GmOnly property stripped for a player; unreadable doc's op dropped; seq preserved on full redaction.
- membership: `world_role` lookup; admin ⇒ GM without a row.

**Integration (WS + HTTP harness)**
- Two clients edit *different* fields of one doc → both confirm; *same* field → one `Conflict`.
- A GmOnly property is stripped for a player but the event's seq still advances (no false resync).
- An HTTP `PATCH` broadcasts the resulting Event to WS subscribers.
- A membership role change alters a recipient's subsequent filtered view.
- A spectator's write intent is rejected `Forbidden`.

## 11. Decisions locked in this brainstorm

1. **`world_members` table** (world_id, user_id, world_role) is the per-world role source; admin/GM-managed; world creation lands in M5 with the creator as first GM.
2. **Field-level optimistic concurrency via pre-images:** Intent → in-transaction per-op pre-image check → confirm+broadcast (tagged with intent_id) or `Reject{Conflict}`. Different-field edits both succeed (the merge); same-field losers reject and reconcile. "Server-side rollback" = transactional atomicity + pre-image rejection; client-side rollback UX is M6, consuming the reversible `invert()` substrate M5 keeps storing.
3. **One core write path** (`Room::publish` → `apply_intent`) behind both a WS `Intent` and HTTP CRUD endpoints; both broadcast.
4. **Per-recipient filtering in the egress task** via `PermissionContext`, reusing `filter_properties`, with **seq-preserving redaction** so the sequence guard never sees a false gap; resync filters identically.
5. **Reads via HTTP filtered GET**; initial load = GET-then-subscribe; doc-snapshot resync tier deferred to M6.
6. **Copy independence** via deep copy + fresh UUIDs + `Source` provenance; **asset-UUID references** validated structurally (upload is M8).
7. **`EmitTest` retired**; the convergence harness switches to real create/update intents.
