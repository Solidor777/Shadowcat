# Local Gate Enforcement

## Problem

An agent pushed a commit that failed `pnpm lint:comments` in CI. The violating gate runs in
two seconds. The gate list is followed by discipline alone, and discipline has now failed
repeatedly: memory records five prior recurrences of a missed CI gate, making this the sixth.

Under the project rule that a repeat failure is a stop signal, the fix is the cause, not the
instance.

### Diagnosed cause

The gate set has no executable form. It exists as twenty-odd `run:` steps spread across five
jobs in `.github/workflows/ci.yml`, and every agent re-derives the list by reading that YAML as
prose. A re-derivation that can drop an entry eventually drops one. Nothing in the commit or
push path executes the list, so a dropped entry produces no signal until CI.

Two independent sub-causes, both of which must be removed:

1. **No bundle.** There is no command that runs the gates. `package.json` exposes each checker
   separately; the composition lives only in the agent's head.
2. **No interception.** `.git/hooks` holds only the stock samples and `core.hooksPath` is
   unset. No mechanism observes a commit or a push.

### A third defect found while measuring

The checkers run against the working tree, not against what is being committed or pushed. At the
time this was diagnosed the local tree was green *because of an uncommitted edit* while `HEAD`
was red — the exact inversion. An agent that had dutifully run the gate would have obtained a
green result that did not describe the commit. Any fix that only adds "run the gate" reproduces
this defect.

## Measurements

Warm caches, sequential, one desktop, every step green:

| Step | Time | Step | Time |
| --- | --- | --- | --- |
| six static checkers, combined | 6s | `cargo clippy --all-targets` | 44s |
| `pnpm lint` | 6s | `cargo clippy -D missing-docs` | 71s |
| `check:svelte-runtime` | 2s | `pnpm -r typecheck` | 87s |
| `cargo fmt --check` | 2s | `docs:check-examples` | 90s |
| `test:scripts` | 8s | `pnpm -r test` | 118s |
| `lint:props` | 10s | `docs:generate` | 202s |
| `lint:docs` | 35s | `cargo test --all` | 237s |
| `pnpm build` and worked examples | 10s | `cargo build --release` | 235s |

The full non-e2e set is ~19m20s sequentially. CI's comparable wall clock comes from running
three jobs concurrently, not from being faster per step.

The load-bearing observation is the cliff: **~70 seconds buys every pure-checker gate**; the
remaining ~18 minutes is entirely compile-and-test. The failure that motivated this work, and by
memory's account the five before it, all sit in the 70-second column.

## Decisions

Owner rulings taken during design:

- **Depth.** Two tiers. Commit is gated by the ~70s checker tier; push is gated by full non-e2e
  parity.
- **Bypass.** None. `--no-verify` is denied at the harness layer. An agent that cannot pass the
  gate stops and reports; only the owner can push by hand.
- **Privacy.** Nothing carrying personally identifiable information is committed. This
  constrains the harness-layer work, which requires tracking a settings file.

## Design

### 1. The gate manifest

`scripts/gates.toml` enumerates **every** `run:` step in `ci.yml`, each classified into exactly
one tier:

| Tier | Meaning |
| --- | --- |
| `commit` | runs in the ~70s pre-commit tier |
| `push` | runs in the full pre-push tier |
| `setup` | not a gate — toolchain install, artifact upload, environment export |
| `ci-only` | cannot run locally; **requires a stated reason** |

`scripts/check-gate-manifest.mjs` parses `ci.yml` and fails when:

- a `run:` step in `ci.yml` has no manifest entry (a new gate arrived unclassified);
- a manifest entry names a command absent from `ci.yml` (the entry went stale);
- a `ci-only` entry carries no reason.

Multi-line `run: |` blocks and steps carrying `${{ }}` expressions are matched on a normalised
form of their whole body, so a step cannot evade classification by being multi-line.

This checker runs in the commit tier **and** as a CI step. Adding a step to `ci.yml` without
classifying it therefore fails at the very next commit. This is the cause fix: the gate list
stops being prose an agent re-derives and becomes an artifact whose divergence from `ci.yml` is
itself a gate.

### 2. Two tiers

`pnpm gate:commit` — the manifest's `commit` tier, ~70s: the six static checkers, `pnpm lint`,
`lint:docs`, `lint:props`, `cargo fmt --check`, `test:scripts`, `check:svelte-runtime`, and the
manifest drift check.

`pnpm gate:push` — the `commit` tier plus every `push` entry, ~19m: typecheck, `pnpm -r test`,
`docs:check-examples`, both builds, the worked examples, both clippy invocations,
`cargo test --all`, `docs:generate`.

Both runners derive their step list from the manifest at run time. Neither hardcodes a list.

### 3. The push receipt

The push tier cannot execute inside `git push`. An agent's shell tool caps at 600 seconds; a
19-minute pre-push hook is killed mid-run, and a killed push is precisely the pressure that
produces a `--no-verify`. The hook therefore verifies a receipt rather than running the tier.

On success `pnpm gate:push` writes `.git/shadowcat-gate-receipt` — inside `.git`, never tracked —
recording the tree hash of `HEAD`, the commit SHA, the run's completion time, and a hash of
`gates.toml`.

`pre-push` refuses unless all of:

- a receipt exists and records success;
- the receipt's tree hash equals the tree of the commit being pushed;
- the working tree is clean, so what was gated is what is pushed;
- the recorded `gates.toml` hash matches the current one, so editing the gate list invalidates
  every outstanding receipt.

Keying on the tree hash rather than a timestamp removes the third defect above: a receipt
describes an exact tree, and cannot be inherited by a different one.

**Stated limitation.** `pre-commit` runs against the working tree, not the staged index, so a
partial commit is gated on more than it contains. The alternative — exporting the index to a
temporary tree — would silently change the corpus the checkers scan, because several resolve
paths relative to the repository root and one additionally scans the external skills checkout.
A known, bounded gap is preferred to an unknown one. The push receipt is the exact gate.

### 4. Enforcement

Two layers, because each covers what the other cannot.

**Git layer.** `core.hooksPath` points at a tracked `scripts/git-hooks/` holding `pre-commit`
and `pre-push`. This catches every commit and push regardless of which tool issues it.

Because sibling worktrees share one `.git/config`, a single repository-wide `core.hooksPath`
would make every worktree run one checkout's hooks. The installer therefore enables
`extensions.worktreeConfig` and writes `core.hooksPath` into the per-worktree config, so each
worktree runs its own branch's hooks.

`scripts/install-git-hooks.mjs` performs that setup and is wired to `prepare` in
`package.json`, so `pnpm install` arms it. Every fresh clone and every new worktree runs
`pnpm install` already, which closes the un-armed-repository hole without a human remembering
anything. The installer no-ops under CI.

**Harness layer.** A tracked `.claude/hooks/guard-git.mjs` on `PreToolUse` for `Bash` returns
`permissionDecision: "deny"` for:

- `git commit --no-verify` and `git commit -n`;
- `git push --no-verify`;
- an inline `git -c core.hooksPath=...` override, and any command mutating `core.hooksPath`;
- **any** `git commit` or `git push` while `core.hooksPath` is unset, so an un-armed repository
  blocks rather than silently permitting.

The last clause is what makes the git layer non-optional rather than merely present.

This requires `.claude/settings.json` to become tracked, which means deleting its entry at
`.gitignore:39`. The file was audited: it contains only permission globs and the graphify
context hooks, addresses the repository through `$CLAUDE_PROJECT_DIR`, and holds no absolute
user path, address, or credential. `.claude/settings.local.json` does contain machine-specific
paths and stays untracked; its existing ignore entry is retained.

A `commit`-tier check enforces that privacy constraint mechanically rather than by discipline:
it fails if the tracked settings file acquires an absolute user path, an electronic address, or
a credential-shaped value.

### 5. Scope boundary

Two CI jobs stay unreachable locally and are recorded as `ci-only` manifest entries with their
reasons: the cross-runtime `e2e` job and the browser `ui-e2e` job. The `rust` job's macOS and
Windows matrix legs are likewise out of local reach from any one desktop.

That is the complete surface CI can catch that the local gate cannot. It is enumerated in the
manifest rather than left open, and the drift check keeps it enumerated.

## Testing

Every new script gets a `*.test.mjs` sibling under `scripts/`, matching the existing convention;
`pnpm run test:scripts` is already CI-enforced.

Three tests are load-bearing and are written first and demonstrated failing before the code they
cover exists:

1. The drift check **fails** on a fixture `ci.yml` carrying a `run:` step absent from the
   manifest. A positive control that only proves the checker runs is insufficient; the fixture
   mutates a real step shape, including a multi-line `run: |` block.
2. `pre-push` **refuses** a receipt whose tree hash does not match the commit being pushed, and
   separately refuses one whose `gates.toml` hash is stale.
3. The harness guard **denies** each bypass form, including the inline `-c` override.

## Files

| Path | Change |
| --- | --- |
| `scripts/gates.toml` | new — the classified gate list |
| `scripts/check-gate-manifest.mjs` | new — drift check, plus its test sibling |
| `scripts/run-gate-tier.mjs` | new — tier runner and receipt writer, plus its test sibling |
| `scripts/install-git-hooks.mjs` | new — hook installer, plus its test sibling |
| `scripts/git-hooks/pre-commit` | new |
| `scripts/git-hooks/pre-push` | new |
| `scripts/check-tracked-settings-privacy.mjs` | new — privacy check, plus its test sibling |
| `.claude/hooks/guard-git.mjs` | new — harness bypass guard |
| `.claude/settings.json` | becomes tracked; gains the guard hook |
| `.gitignore` | drops the `.claude/settings.json` entry |
| `package.json` | gains `gate:commit`, `gate:push`, `lint:gate-manifest`, `prepare` |
| `.github/workflows/ci.yml` | gains the drift check and the privacy check as steps |
| `.claude/CLAUDE.md` | records the gate as the mechanism behind the existing push rule |

## Non-goals

- Speeding up any existing gate. The tiers are composed from the steps as they stand.
- Changing what CI runs, beyond adding the two new checks.
- Any bypass mechanism, ledger, or carve-out. The owner's ruling was that none exists.
