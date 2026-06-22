# Post-Work Findings

Living record of issues surfaced during review/audit. NOT a to-do list — entries
are observations awaiting triage, not committed work.

- Title: offline-intent flush can precede the async `#onWelcome` body on reconnect.
  Summary: `WsClient` fires `onResyncComplete` (→ `WorldSession.#flushOfflineQueue`)
  synchronously on the caught-up Welcome branch / on `resync_end`, while
  `#onWelcome` runs as an unawaited `void` async (it awaits a member fetch before
  re-establishing scene subscriptions). So queued offline intents can transmit
  before scene subs re-establish. Not a correctness defect: flushed intents reach
  the server regardless, and scene-derived read state is eventually consistent via
  the egress re-evaluation debounce; FIFO confirm-correlation is unaffected.
  Status: Accepted (eventually-consistent ordering). If a stricter ordering is ever
  needed, gate the flush on an "onWelcome settled" promise.

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

- Title: no smaller "caption" text-size token in the M7d token set. Summary: the
  M8b-2 asset panel's tile filename (`Assets.svelte` `.name`) renders at inherited
  body size — `_primitives.scss`/`_semantic.scss` define `--space-*`, `--radius-*`,
  `--font-sans`, and `--text-*` *colors* but no smaller font-*size* token (the plan's
  assumed `--text-sm` does not exist). Captions/secondary labels therefore can't be
  visually de-emphasized by size via a token. Status: **Deferred to M12** by the M8c-2
  §10 re-audit — canvas chrome (the M8 audit's scope) renders no text, so a font-*size*
  scale is out of scope here; it belongs with the text-dense default sheets/browsers in
  M12 (the second token re-audit point per `PLAN.md` M7).

- Title: M8c-2 §10 canvas-chrome token re-audit (outcome). Summary: re-audited the M7d
  3-tier token set against the first rendered canvas chrome. (1) Added a semantic
  `--grid-line` token (= `--slate-700`) so the canvas grid is decoupled from UI
  `--border`. (2) Fixed a latent M8c-1 bug: `Stage.svelte`'s `readColor` used
  `getComputedStyle().getPropertyValue("--token")`, which returns the unresolved
  `var(...)` string for aliased custom properties — so the grid silently used its
  fallback color and ignored the theme; it now resolves the real color via a
  computed-`color` probe. (3) Background uses `--surface-base` (already correct). (4)
  Fog-state colors (dimmed/unexplored) deferred to M9 (no visible fog in identity mode).
  Status: Resolved for M8c (canvas chrome); caption size token → M12 (above).
