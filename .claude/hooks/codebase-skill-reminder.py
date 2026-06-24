#!/usr/bin/env python3
"""PreToolUse(Edit|Write|MultiEdit): remind the agent which shadowcat-codebase-* skill
covers the file being edited. Deduped once per (session, subsystem). Fails open.

Ordering: most-specific subsystems first — `assets` precedes `documents-permissions`
(shared `src/server/src/data/`) and `realtime-sync` (shared `src/server/src/http/`) so
asset files route to `assets`, not the broader globs. First match wins."""
import sys, json, os, tempfile, re

# (subsystem-id, [path regexes]). Order = priority; first match wins.
SUBSYSTEMS = [
    ("assets",               [r"src/modules/assets/", r"src/server/src/data/asset\.rs", r"src/server/src/http/assets\.rs"]),
    ("documents-permissions", [r"src/server/src/data/", r"src/client/core/src/wire\.ts"]),
    ("actors-tokens",        [r"src/modules/actors/", r"src/modules/factions/", r"src/client/core/src/actor\.ts"]),
    ("scene-rendering",      [r"src/server/src/scene/", r"src/modules/stage/", r"src/modules/scene-tools/", r"src/client/render/"]),
    ("realtime-sync",        [r"src/server/src/ws/", r"src/server/src/http/", r"src/server/src/auth/", r"src/client/core/src/(store|optimistic|ws-client)\.ts"]),
    ("client-shell",         [r"src/modules/entry/", r"src/modules/core-ui/", r"src/modules/topbar/", r"src/modules/statusbar/", r"src/modules/settings/", r"src/client/shell/", r"src/client/ui-kit/"]),
]


def main():
    try:
        d = json.loads(sys.stdin.read())
    except Exception:
        return  # fail open on any parse error
    # SAFETY: routing keys purely on file_path; tool_name is intentionally not checked.
    # Read-exclusion is enforced by the settings.json matcher (Edit|Write|MultiEdit) — that
    # matcher scoping is load-bearing; widening it would fire reminders on reads.
    t = d.get("tool_input", d) or {}
    path = str(t.get("file_path") or "").replace(chr(92), "/").lower()
    session = str(d.get("session_id") or "nosession")
    if not path:
        return
    sub = None
    for name, pats in SUBSYSTEMS:
        if any(re.search(p.lower(), path) for p in pats):
            sub = name
            break
    if sub is None:
        return
    # Per-(session, subsystem) dedup via marker file; fire once per subsystem per session.
    try:
        mdir = os.path.join(tempfile.gettempdir(), "shadowcat-skill-markers")
        os.makedirs(mdir, exist_ok=True)
        marker = os.path.join(mdir, "%s-%s.seen" % (re.sub(r"[^A-Za-z0-9_.-]", "_", session), sub))
        if os.path.exists(marker):
            return
        with open(marker, "w") as f:
            f.write("1")
    except Exception:
        pass  # if dedup bookkeeping fails, still emit (a repeat beats silence)
    skill = "shadowcat-codebase-%s" % sub
    msg = ("You are editing the %s subsystem. Consider invoking the `%s` skill "
           "(plus `shadowcat-codebase-core`) for invariants and pointers before changing it."
           % (sub, skill))
    print(json.dumps({"hookSpecificOutput": {
        "hookEventName": "PreToolUse", "additionalContext": msg}}))


if __name__ == "__main__":
    main()
