#!/usr/bin/env python3
"""Assert that the packaged WebKitGTK UI is exposed through Linux AT-SPI."""

from __future__ import annotations

import sys
import time

import pyatspi


def children(node):
    try:
        return [node.getChildAtIndex(index) for index in range(node.childCount)]
    except (LookupError, RuntimeError):
        return []


def walk(node):
    pending = [node]
    while pending:
        current = pending.pop()
        yield current
        pending.extend(reversed(children(current)))


def describe(node):
    try:
        return node.getRoleName(), node.name or ""
    except (LookupError, RuntimeError):
        return "defunct", ""


def find_application(timeout_seconds=30):
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        desktop = pyatspi.Registry.getDesktop(0)
        for application in children(desktop):
            role, name = describe(application)
            if role == "application" and "bettertricks" in name.casefold():
                return application
        time.sleep(0.25)
    raise AssertionError("Bettertricks did not appear in the AT-SPI desktop tree")


def main():
    application = find_application()
    deadline = time.monotonic() + 30
    while True:
        nodes = list(walk(application))
        described = [describe(node) for node in nodes]
        roles = [role for role, _ in described]
        names = [name for _, name in described if name]
        if roles.count("push button") >= 5 or time.monotonic() >= deadline:
            break
        time.sleep(0.25)

    if not any(role in {"frame", "window"} for role in roles):
        raise AssertionError("AT-SPI tree has no application window")
    if roles.count("push button") < 5:
        summary = sorted({f"{role}:{name}" for role, name in described})[:30]
        raise AssertionError(
            f"expected at least five exposed buttons, found {roles.count('push button')}; "
            f"tree sample: {summary}"
        )
    expected_names = {"Components", "Activity", "Settings"}
    missing = sorted(
        expected
        for expected in expected_names
        if not any(name == expected or name.startswith(f"{expected} ") for name in names)
    )
    if missing:
        raise AssertionError(
            f"missing accessible navigation names: {', '.join(missing)}; "
            f"available names: {sorted(set(names))[:80]}"
        )

    focusable = next(
        (
            node
            for node in nodes
            if describe(node)[0] == "push button"
            and pyatspi.STATE_FOCUSABLE in node.getState().getStates()
        ),
        None,
    )
    if focusable is None or not focusable.queryComponent().grabFocus():
        raise AssertionError("could not move accessibility focus to an exposed button")

    print(
        "AT-SPI bridge passed: "
        f"{len(nodes)} nodes, {roles.count('push button')} buttons, "
        f"application={application.name!r}"
    )


if __name__ == "__main__":
    try:
        main()
    except Exception as error:  # noqa: BLE001 - smoke probe must print bridge failures
        print(f"accessibility smoke failed: {error}", file=sys.stderr)
        raise
