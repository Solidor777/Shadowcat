#!/usr/bin/env python3
"""Invokes the named push button inside ONE process's accessibility tree through AT-SPI.

It acts only on that process's own element tree (never global keyboard or mouse input), so it cannot
reach another application. Exit status: 0 invoked, 3 no such button, anything else is a failure of
the driver itself (for example no accessibility bus).

Why AT-SPI: Chromium shows the HTTPS-First "Continue to site" prompt as browser UI, not as a web
page, so it is absent from the DOM, the accessibility tree DevTools exposes and screenshots of the
page; only the operating system's accessibility API reaches it.

usage: invoke-button.py <pid> <button name>
Requires PyGObject and the Atspi typelib (Debian/Ubuntu: python3-gi, gir1.2-atspi-2.0) and a
running accessibility bus (at-spi2-core).
"""
import sys


def load_atspi():
    """Imports the Atspi typelib; a missing one fails here with its own message."""
    import gi

    gi.require_version("Atspi", "2.0")
    from gi.repository import Atspi

    return Atspi


NOT_FOUND = 3
# Deep enough for a browser window's widget tree, shallow enough to stop on a cycle.
MAX_DEPTH = 40


def find_button(Atspi, node, name, depth=0):
    """Returns the first push button named `name` at or below `node`, or None."""
    if depth > MAX_DEPTH:
        return None
    try:
        if node.get_role() == Atspi.Role.PUSH_BUTTON and node.get_name() == name:
            return node
        count = node.get_child_count()
    except Exception:  # an element that vanished mid-walk is skipped, not fatal
        return None
    for index in range(count):
        try:
            child = node.get_child_at_index(index)
        except Exception:
            continue
        if child is None:
            continue
        found = find_button(Atspi, child, name, depth + 1)
        if found is not None:
            return found
    return None


def main(argv):
    Atspi = load_atspi()
    pid = int(argv[1])
    name = argv[2]
    desktop = Atspi.get_desktop(0)
    # The application pids the bus exposes, printed to stderr when no button matches so the failure
    # names whether the browser registered with the accessibility bus at all.
    seen = []
    for index in range(desktop.get_child_count()):
        app = desktop.get_child_at_index(index)
        if app is None:
            continue
        seen.append(app.get_process_id())
        if app.get_process_id() != pid:
            continue
        button = find_button(Atspi, app, name)
        if button is not None:
            Atspi.Action.do_action(button, 0)
            print("INVOKED")
            return 0
    print("no push button named %r in process %d; bus applications by pid: %s" % (name, pid, seen), file=sys.stderr)
    return NOT_FOUND


if __name__ == "__main__":
    sys.exit(main(sys.argv))
