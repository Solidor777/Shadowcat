# Phase 3 — Dispatcher brief

> For the session that dispatches and coordinates the Phase-3 build. Read this, then the
> master spec, then each milestone's plan. Everything here is measured state as of the
> planning session's hand-off; re-measure before acting (`git worktree list`, `git log`,
> `git status` per worktree).

## 0. Directives (verbatim into EVERY dispatched agent's first prompt)

> The iron rule is no deferrals of existing work, or new work as it comes up - we fix this now
> unless I give my EXPRESS authorization. The only exception is if a bug or to-do has a genuine
> blocker that is already logged in a milestone in PLAN.md that has not been started yet. Another
> iron clad is rule is that when faced with a design fork, determine the best long term shape in
> keeping with our plans and goals, and implement accordingly. You only need to ask me if the
> question "what is the best long term shape in keeping with our plans and goals?" is not able to
> answer the question. Churn is not a concern. This paragraph must be copied verbatim to any
> agents dispatched in this campaign.

Also in the first prompt of every dispatch: the reporting channel (Agent result — omit `name`;
or `SendMessage` to you; or a named file), and "opus is banned; sonnet only". Never end your own
turn to ask whether to continue; the campaign ends at master §8.

## 1. Where everything is

| Artifact | Path |
|---|---|
| Master integration spec | `docs/superpowers/specs/2026-09-11-phase3-master-integration-design.md` |
| Milestone specs | `docs/superpowers/specs/2026-09-11-m22-performance-settings-design.md`, `…-m23-audio-design.md`, `…-m24-vfx-design.md`, `…-m25-levels-portals-design.md`, `…-m26-dice-3d-design.md`, `…-m27-voice-ducking-design.md`, `…-m28-sandboxed-validators-design.md` |
| Milestone plans | `docs/superpowers/plans/2026-09-11-m22-performance-settings.md`, `…-m23-audio.md`, `…-m24-vfx.md`, `…-m25-levels-portals.md`, `…-m26-dice-3d.md`, `…-m27-voice-ducking.md`, `…-m28-sandboxed-validators.md` |
| Worktrees / branches | `C:/Dev/Shadowcat-m22` `m22-performance` · `-m23` `m23-audio` · `-m24` `m24-vfx` · `-m25` `m25-levels` · `-m26` `m26-dice-3d` · `-m27` `m27-ducking` · `-m28` `m28-sandbox`; main checkout `C:/Dev/Shadowcat` (merges land here via PR) |
| Planning branch | `phase3-planning` (the docs commits; every milestone branch was fast-forwarded to it) |
| Memory | `~/.claude/projects/C--Dev-Shadowcat/memory/phase3-planning-campaign-state.md` (append your progress log there, one line per event) |
| Plugin checkout (skills) | `~/.claude/skills/shadowcat-codebase/` — its own git repo; coders edit skills there WITHOUT committing; you review + commit + push per milestone |

## 2. The loop, per milestone

1. **Dispatch the coder** — `shadowcat-codebase:shadowcat-coder` (sonnet, medium), unnamed,
   report as the Agent result. Prompt = §0 directives + the worktree path + "execute Tasks
   A–B of `<plan path>` in order; the plan's Global constraints apply; commit per task with
   explicit paths; skill edits go to the plugin checkout uncommitted; STOP before the
   integration task". One coder per worktree at a time (sequential editors on one branch).
   Seven coders may run at once, one per worktree.
2. **On report:** measure (`git log --oneline main..<branch>`, `git status --short` in the
   worktree) before believing it. Then buddy-check the diff: pre-generate
   `git diff main...<branch> > <scratchpad>/<m>.diff` (reviewers have no Bash) and dispatch
   `shadowcat-codebase:shadowcat-spec-reviewer` + `shadowcat-codebase:shadowcat-code-reviewer`
   blind (sonnet, high), each reporting as the Agent result; broker rounds until they converge
   (the `superpowers:buddy-checking` skill); apply agreed findings through a fix coder; unresolved
   disagreements are design questions for the user.
3. **Integration task** (each plan's last task) only after the milestone's consumed seams have
   merged to `main` (master §5 order: M22 → M28 → M24 → M23 → M25 → M26 → M27). The coder
   merges `origin/main` INTO the branch (merge commit, never rebase), resolves by master §3,
   wires the seams, runs the full battery.
4. **Playwright** — YOU run the browser suite, one worktree at a time, on port 31999:
   `pnpm --filter @shadowcat/shell e2e:build` then `pnpm --filter @shadowcat/shell exec
   playwright test <spec>` (never two suites at once on the machine; the shell e2e reuses ANY
   server on 31999). New specs per milestone are named in each spec's Tests section.
5. **Skills** — dispatch a spec-reviewer on `git -C ~/.claude/skills/shadowcat-codebase diff`
   for the milestone's skill edits; run from the PLUGIN dir `node <repo>/scripts/check-skill-
   symbol-refs-cli.mjs` (0 broken), `pnpm run test:scripts` in the repo, `bash
   hooks/test-codebase-skill-reminder.sh` in the plugin dir, `node scripts/check-skill-api-refs-
   cli.mjs` after `pnpm build:all`; then commit + push in the plugin repo. Bump `plugin.json`
   `1.6.0` → `1.7.0` ONCE at campaign end.
6. **PR** — `pnpm gate:push` on the branch HEAD (a commit after the receipt invalidates it),
   `git push -u origin <branch>`, `gh pr create` (body ends with the attribution lines from the
   session), wait for BOTH the push-event and pull_request-event CI runs, `gh pr merge <n>
   --merge` (main is branch-protected; `--auto` is off), then `git -C C:/Dev/Shadowcat pull`
   and measure `git rev-parse origin/main main`.
7. **After the last merge:** `docs/PLAN.md` Phase 3 heading gets ✅ with the M22–M28 pointer
   paragraph; ARCHITECTURE §3/§4 rows as the specs say; delete every branch + worktree (local +
   origin), measured.

## 3. Known hazards (from the previous campaigns' memory — verified rules)

- A named agent never returns a result; idle is not dead — do not dispatch a second editor
  onto a scope until the first has acknowledged a stand-down.
- `test:scripts` in the commit hook has timed out under heavy concurrency once (5s/20s fixed
  timeouts); a SECOND occurrence is a stop signal: serialize, don't retry.
- Worktree paths must stay short (`C:/Dev/Shadowcat-m2X`) — deep paths break vitest on Windows.
- `pnpm build` must precede any cargo build (rust-embed reads `dist/` at compile time); the
  bootstrap script already built `dist/` in every worktree — rebuild after client changes.
- Reviewers fabricate git output when they lack Bash — always pre-generate the diff.
- `trash`, never `rm`; run `git status` after every `trash`.
- A host crash mid-dispatch: measure each worktree's `git log`/`git status` and the plugin
  checkout before resuming any agent, and paste the measured state into the resume message.
- Every plan's fenced code blocks were scanned with the repo's own comment gate
  (`scripts/check-comment-refs.mjs`'s `scanContent`) to zero hits before hand-off. If an
  implementer's `pnpm lint:comments` still fails on a copied comment, the fix is to rewrite the
  comment as a present-tense constraint citing symbols — never a suppression, never a plan edit.

## 4. Exit condition

Master spec §8, measured: seven PRs merged, CI green on `main`, docs synced, skills pushed,
`plugin.json` 1.7.0, no leftover branches/worktrees.
