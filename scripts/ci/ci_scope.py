"""Select documentation checks only when every changed path is ordinary prose."""

import json
import os
from pathlib import Path
import re
import subprocess


ROOT = Path(__file__).resolve().parents[2]


def is_documentation(path):
    if "\\" in path or any(part in {"", ".", ".."} for part in path.split("/")):
        return False
    # Rule pages are compiled into the binary; history is a parser test input.
    if path.startswith("docs/rules/") or path == "docs/design/history.md":
        return False
    return path in {"README.md", "CONTRIBUTING.md", "LICENSE", "corpus/README.md"} or (
        path.startswith(("docs/", "corpus/docs/")) and path.endswith(".md")
    )


def docs_only(event_name, event, root=ROOT):
    if event_name not in {"push", "pull_request"}:
        return False
    try:
        if event_name == "pull_request":
            base = event["pull_request"]["base"]["sha"]
            head = event["pull_request"]["head"]["sha"]
            separator = "..."
        else:
            base, head = event["before"], event["after"]
            separator = ".."
        if not all(isinstance(ref, str) and re.fullmatch(r"[0-9a-f]{40}", ref)
                   and ref != "0" * 40 for ref in (base, head)):
            return False
        # Disabling rename detection includes both the old and new paths.
        result = subprocess.run(
            ["git", "diff", "--name-only", "--no-renames", "-z", f"{base}{separator}{head}", "--"],
            cwd=root, check=True, capture_output=True,
        )
        raw = result.stdout.decode("utf-8")
        if not raw.endswith("\0"):
            return False
        return all(is_documentation(path) for path in raw[:-1].split("\0"))
    except (KeyError, TypeError, ValueError, OSError, subprocess.CalledProcessError):
        return False


def main():
    documentation = False
    if os.environ.get("GITHUB_EVENT_NAME") in {"push", "pull_request"}:
        try:
            event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8"))
            documentation = docs_only(os.environ["GITHUB_EVENT_NAME"], event)
        except (KeyError, ValueError, OSError):
            pass
    value = str(documentation).lower()
    print(f"docs_only={value}")
    if output := os.environ.get("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8") as stream:
            stream.write(f"docs_only={value}\n")


if __name__ == "__main__":
    main()
