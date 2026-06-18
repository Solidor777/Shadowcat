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

- Title: no protection against removing/demoting the last GM. Summary:
  `remove_member`/`set_role` allow a world's only GM to be removed or demoted,
  after which only a server admin can manage that world. Availability footgun,
  not a security defect. Status: Needs triage — reject the operation when it
  would leave a world with zero GMs, or document admin-recovery as the intended
  path.
