# Codebase-skill activation hook

`codebase-skill-reminder.py` is a `PreToolUse(Edit|Write|MultiEdit)` hook: when a file under a
known subsystem is edited, it injects a one-line reminder to invoke the matching
`shadowcat-codebase-<subsystem>` skill. It dedupes once per `(session, subsystem)` and **fails
open** (never blocks a tool call, emits nothing on a parse error).

The hook **script and test are committed**; the **wiring lives in `.claude/settings.json`, which
is git-ignored** (local-only, per the repo's `.claude/` convention). So each machine wires it up
once.

## Per-machine wiring

Add this object to the `hooks.PreToolUse` array in your local `.claude/settings.json`:

```json
{
  "matcher": "Edit|Write|MultiEdit",
  "hooks": [
    { "type": "command", "command": "python3 .claude/hooks/codebase-skill-reminder.py" }
  ]
}
```

Requires `python3` on PATH (same dependency as the existing graphify hooks).

## Test

```bash
bash .claude/hooks/test-codebase-skill-reminder.sh   # expects: ALL HOOK TESTS PASS
```

## Maintenance

When a new `shadowcat-codebase-<subsystem>` skill is added, add its path globs to the
`SUBSYSTEMS` map in `codebase-skill-reminder.py` (most-specific subsystems first) and a routing
check to the test. See the `shadowcat-codebase-core` skill's "Maintaining this skill family".
