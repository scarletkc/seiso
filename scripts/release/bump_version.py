"""Bump Cargo, Cargo.lock, and npm together.

Usage: python -m scripts.release.bump_version [patch|minor|major|VERSION] [--note TITLE]
The default is patch, which promotes a prerelease to its stable version.
Use VERSION (with an optional v prefix) for alpha, beta, or rc releases.
--note creates docs/release-notes/VERSION.md; write its body before releasing.
"""

import argparse
import json
import re
import tomllib

from .release import ROOT, release_metadata
from .versions import parse_version, python_version, version_key


def next_version(current, requested):
    (major, minor, patch), stage, _ = parse_version(current)
    if requested == "major":
        result = f"{major + 1}.0.0"
    elif requested == "minor":
        result = f"{major}.{minor + 1}.0"
    elif requested == "patch":
        result = f"{major}.{minor}.{patch if stage else patch + 1}"
    else:
        result = requested.removeprefix("v")
    if version_key(result) <= version_key(current):
        raise ValueError(f"New version must be greater than {current}")
    return result


def replace_once(text, pattern, replacement, label):
    updated, count = re.subn(pattern, replacement, text, flags=re.MULTILINE)
    if count != 1:
        raise ValueError(f"Expected exactly one {label}; found {count}")
    return updated


def plan_bump(root, current, version, note=None):
    changes = {}
    manifest_path = root / "Cargo.toml"
    manifest = manifest_path.read_text(encoding="utf-8")
    if tomllib.loads(manifest)["package"]["name"] != "seiso":
        raise ValueError("The root Cargo package must be named seiso")
    sections = re.split(r"(?m)(?=^\[)", manifest)
    found_package = False
    for i, section in enumerate(sections):
        if section.startswith("[package]"):
            sections[i] = replace_once(
                section, rf'^(version\s*=\s*"){re.escape(current)}(")',
                rf'\g<1>{version}\2', "Cargo package version")
            found_package = True
    if not found_package:
        raise ValueError("No root Cargo package found")
    changes[manifest_path] = "".join(sections)

    lock_path = root / "Cargo.lock"
    blocks = re.split(r"(?m)(?=^\[\[package\]\])", lock_path.read_text(encoding="utf-8"))
    found = set()
    for i, block in enumerate(blocks):
        if not block.startswith("[[package]]"):
            continue
        package = tomllib.loads(block)["package"][0]
        if package["name"] == "seiso" and "source" not in package:
            block = replace_once(block, rf'^(version\s*=\s*"){re.escape(current)}(")',
                                 rf'\g<1>{version}\2', f"{package['name']} lock version")
            found.add(package["name"])
        blocks[i] = block
    if found != {"seiso"}:
        raise ValueError("Cargo.lock must contain the seiso package")
    changes[lock_path] = "".join(blocks)

    npm_path = root / "npm/seiso/package.json"
    changes[npm_path] = replace_once(
        npm_path.read_text(encoding="utf-8"), rf'("version"\s*:\s*"){re.escape(current)}(")',
        rf'\g<1>{version}\2', "npm version")
    if json.loads(changes[npm_path])["version"] != version:
        raise ValueError("Failed to update npm version")

    if note is not None:
        if not note.strip() or len(note.strip().splitlines()) != 1:
            raise ValueError("--note requires a non-empty, single-line title")
        note_path = root / "docs/release-notes" / f"{version}.md"
        if note_path.exists():
            raise ValueError(f"{note_path} already exists; edit it instead of overwriting it")
        changes[note_path] = f"## {note.strip()}\n\n"
    return changes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", nargs="?", default="patch")
    parser.add_argument("--note", help="Start a handwritten release note with this title")
    parser.add_argument("--dry-run", action="store_true", help="List changes without writing files")
    args = parser.parse_args()
    current = release_metadata()
    version = next_version(current, args.version)
    changes = plan_bump(ROOT, current, version, args.note)
    for path, contents in changes.items():
        if not args.dry_run:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(contents, encoding="utf-8", newline="\n")
        print(f"{'Would update' if args.dry_run else 'Updated'} {path.relative_to(ROOT)}")
    print(f"{current} -> {version}; CLI/npm: {version}; Python: {python_version(version)}")


if __name__ == "__main__":
    main()
