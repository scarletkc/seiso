"""Check that a pull request title is a Conventional Commits header."""

import json
import os
from pathlib import Path
import re
import sys


# Keep in sync with the commit types in CONTRIBUTING.md.
TYPES = ("feat", "fix", "perf", "refactor", "docs", "test", "build", "ci", "style", "chore", "revert")
MAX_LENGTH = 72
GUIDE = "https://github.com/scarletkc/seiso/blob/main/CONTRIBUTING.md#write-commits"
HEADER = re.compile(r"(?P<type>[A-Za-z]+)(?:\((?P<scope>[^()]*)\))?!?: (?P<description>.*)")
SCOPE = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")


def problems(title):
    errors = []
    if len(title) > MAX_LENGTH:
        errors.append(f"Shorten the title to at most {MAX_LENGTH} characters; it has {len(title)}.")
    match = HEADER.fullmatch(title)
    if match is None:
        errors.append("Use the format <type>(<scope>): <description>; the scope and a ! before the colon are optional.")
        return errors
    if match["type"] not in TYPES:
        errors.append(f"Replace the type {match['type']!r} with one of: {', '.join(TYPES)}.")
    if match["scope"] is not None and not SCOPE.fullmatch(match["scope"]):
        errors.append("Write the scope in lowercase letters and digits separated by single hyphens, such as rules.")
    description = match["description"]
    if not description or description != description.strip():
        errors.append("Put exactly one space after the colon, followed by the description.")
    elif re.match(r"[A-Z][a-z]", description):
        errors.append("Start the description with a lowercase word; rule codes and other identifiers keep their case.")
    if description.endswith("."):
        errors.append("Remove the trailing period.")
    return errors


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if argv:
        title = argv[0]
    elif path := os.environ.get("GITHUB_EVENT_PATH"):
        title = json.loads(Path(path).read_text(encoding="utf-8"))["pull_request"]["title"]
    else:
        print("usage: pr_title.py TITLE (or run in a pull_request workflow)", file=sys.stderr)
        return 2
    shown = " ".join(title.splitlines())
    errors = problems(title)
    if not errors:
        print(f"Title follows Conventional Commits: {shown}")
        return 0
    prefix = "::error title=PR title::" if os.environ.get("GITHUB_ACTIONS") == "true" else "error: "
    print(f"Title does not follow Conventional Commits: {shown}")
    for error in errors:
        print(prefix + error)
    print(f"Edit the pull request title; this check runs again after the edit. Format: {GUIDE}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
