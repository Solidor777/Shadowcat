# Post-Work Findings

Living record of issues surfaced during review/audit. NOT a to-do list — entries
are observations awaiting triage, not committed work.

- Title: `slow_reader_recovers_via_resync` does not guarantee the `Lagged` path
  fires. Summary: the M4 convergence test (`src/server/tests/ws_convergence.rs`)
  floods 400 small events to a non-reading client to pressure a broadcast
  `Lagged` → resync, but the OS TCP buffer may absorb all 400 frames so the
  server egress never lags. The test still asserts convergence (final seq = 400,
  no dups/reordering), which holds via either live or resync delivery — so it is
  a valid convergence test but NOT a reliable regression guard for the
  lag-driven resync path specifically. Status: Needs triage — to assert the lag
  path deterministically, check `gaps_detected`/`resyncs_*`/`lagged_drops` via
  `/api/debug/rooms` (or shrink `BROADCAST_CAPACITY` under a test cfg). The
  reconnect test (`all_clients_converge_after_reconnect`) does exercise the
  resync replay path explicitly via `ResyncRequest`.

- Title: capability model — `core:delete` is GM-only by default (behavior change
  from M5). Summary: the capability floor grants Owners `core:read` +
  `core:write_fields` but NOT `core:delete`, so a document Owner can no longer
  delete by default (M5's binary `can_write` allowed it). Intended per the
  capability spec; grant `core:delete` per-document or via a world default to
  restore owner-delete. Status: Accepted (documented behavior change).

- Title: capability model — grants can target `DocRole::None`. Summary: a GM may
  add capabilities to the `None` (no-access) role via `by_role`, widening what
  the floor denies. GM-authored only (not an escalation), and a coherent way to
  raise the default tier; recorded as intentional flexibility rather than
  restricted (restricting only the world-defaults endpoint's `validate_grants`
  would be inconsistent — per-document grants set at create / via PATCH
  `/permissions` bypass it). Status: Accepted (design note from Phase 1 review).

- Title: capability model — `core:create` world authorization deferred.
  Summary: Phase 1 does not gate document creation by a world-level
  `core:create`; current behavior is M5's (any member who owns the new doc may
  create it). The capability constant exists. Status: Needs triage — wire a
  world-level create grant (GM always allowed) when create restriction is
  required.

- Title: capability model — world defaults are not doc_type-scoped. Summary:
  `world_cap_defaults` stores one `CapabilityGrants` per world applied to all
  doc types; the spec allows per-`doc_type` scoping (§7.2). Status: Needs triage
  — extend the stored shape to {all, by_type} when type-specific defaults are
  needed (additive, no migration of the per-world form).

- Title: a saturated lagged WS connection is slow to auto-converge on the
  ubuntu-latest CI runner. Summary: `converges_with_publishing_during_resync`
  originally asserted the deliberately-lagged client reached the tail seq (300)
  in real time while the publisher ran concurrently. On ubuntu-latest the lagged
  connection delivered a contiguous-but-incomplete prefix (e.g. 1..234) and then
  emitted nothing for >10s — even after an explicit `ResyncRequest` on that same
  connection (zero frames). A fresh connection's `ResyncRequest` converges fine
  on the same runner (`all_clients_converge_after_reconnect` passes), so the
  durable resync path is sound; the symptom is auto-convergence latency/stall on
  an already-saturated lagged egress under Linux scheduling. The test now asserts
  the load-bearing invariant (no DROPS during the overlap → contiguous prefix)
  plus full recoverability via a fresh client. Status: Needs triage — determine
  whether the lagged egress genuinely stalls (a latency bug in the egress
  select/replay loop under heavy backpressure) or it is purely CI-runner
  saturation; reproduce with a constrained-CPU local run before changing
  `conn.rs`. Update (M8b-1 push, 2026-06-22): a *second* manifestation observed —
  the authoritative-seq assertion at `ws_convergence.rs:408`
  (`h.authoritative_seqs().last() == Some(300)`) failed `Some(277)` on
  ubuntu-latest after the test's 30s drain-wait budget (300×100ms), with the whole
  test taking 45s; i.e. even the *server-side* single-writer ingress→apply of 300
  queued intents didn't finish in 30s under runner saturation. Passed on
  windows+macos in the same run and locally (4.5s); cleared on job re-run.
  Unrelated to M8b-1 (the failing assertion is on DB ingress throughput, which
  M8b-1 does not touch). If it recurs, widen the drain budget (e.g. 600×100ms) or
  gate the count on a constrained-CPU repro before touching the ingress path.

- Title: `filter_command` redacts replayed history against the *current*
  PermissionSet. Summary: `src/server/src/data/permission.rs` loads each
  `Update` op's document via `get_document` to resolve visibility, so on
  resync/replay a property whose `GmOnly`↔`All` visibility was flipped after the
  event is redacted under the *new* policy, not the policy in force at the
  command's seq. Acceptable for M5 (visibility flips are rare; replay is
  recovery, not audit) but the redaction is not point-in-time faithful. Status:
  Needs triage — if audit-grade replay is ever required, snapshot the relevant
  permissions into the event or attach them to the broadcast.

- Title: an `Update` to a since-deleted document is silently dropped on replay.
  Summary: `filter_command`'s `Update` arm does `let Ok(Some(cur)) =
  get_document(..) else { continue }`; if the doc was later deleted the op is
  skipped. seq/command ordering is preserved and the later `Delete` still
  replays, so final-state convergence and the sequence guard are unaffected — a
  client just sees Create → (missing Update) → Delete. Harmless for end state;
  noted as a replay-fidelity limitation. Status: Accepted.

- Title: by-id document routes leak existence to non-members (403 vs 404).
  Summary: `GET/PATCH/DELETE /api/documents/{id}` load the doc, resolve its
  world, then call `permission_context`, which returns `Forbidden`→403 for a
  non-member — distinguishable from 404 for a nonexistent id. The in-world
  unreadable case already collapses to 404. Low impact (document UUIDs are
  unguessable). Status: Needs triage — map the non-member case on by-id routes
  to `NotFound` for a uniform authz surface.

- Title: `validate_system_size` ignores `embedded` children. Summary:
  `src/server/src/data/validation.rs` measures only `doc.system`; embedded
  copies are stored inline in the parent JSON, so a Create/Update with a large
  `embedded` tree bypasses the 256 KiB opaque-body cap. Bounded in practice by
  axum's default ~2 MB JSON limit and the WS frame cap, so not unbounded.
  Status: Needs triage — validate total serialized size or recurse into embedded
  `system` bodies when embedded documents carry untrusted bulk.

- Title: embedded children's `GmOnly` property overrides are not redacted.
  Summary: `filter_properties` strips only the parent document's
  `property_overrides`; an embedded child carrying its own
  `property_overrides: {"/system/x": "gm_only"}` is delivered to players
  unredacted. Embedded per-property visibility appears out of M5 scope (the
  filtering contract is per-document). Status: Needs triage — recurse redaction
  into embedded children if embedded docs are meant to carry independent
  visibility.

- Title: no smaller "caption" text-size token in the M7d token set. Summary: the
  M8b-2 asset panel's tile filename (`Assets.svelte` `.name`) renders at inherited
  body size — `_primitives.scss`/`_semantic.scss` define `--space-*`, `--radius-*`,
  `--font-sans`, and `--text-*` *colors* but no smaller font-*size* token (the plan's
  assumed `--text-sm` does not exist). Captions/secondary labels therefore can't be
  visually de-emphasized by size via a token. Status: Needs triage — add a tier-2
  `--text-sm` / `--text-caption` size token, then apply it to the asset tile name and
  other secondary labels.

- Title: no protection against removing/demoting the last GM. Summary:
  `remove_member`/`set_role` allow a world's only GM to be removed or demoted,
  after which only a server admin can manage that world. Availability footgun,
  not a security defect. Status: Needs triage — reject the operation when it
  would leave a world with zero GMs, or document admin-recovery as the intended
  path.
