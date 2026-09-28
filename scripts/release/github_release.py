"""Compose handwritten notes plus the commit range, and optionally create a release."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from .release import ROOT, release_metadata, run
from .versions import VERSION, parse_version, version_key


def git(*args, root=ROOT):
    return subprocess.check_output(["git", *args], cwd=root, text=True, encoding="utf-8").strip()


def validate_tag(version, root=ROOT):
    parse_version(version)
    tag = f"v{version}"
    sha = git("rev-parse", "HEAD", root=root)
    tags = git("tag", "--list", tag, root=root).splitlines()
    if tags and git("rev-parse", f"refs/tags/{tag}^{{commit}}", root=root) != sha:
        raise ValueError(f"{tag} already points to another commit; choose a new release version")
    return tag, sha


def handwritten_note(path):
    if not path.exists():
        return ""
    contents = path.read_text(encoding="utf-8").strip()
    lines = contents.splitlines()
    if not lines or not re.fullmatch(r"## \S.*", lines[0]):
        raise ValueError(f"{path} must start with a '## <title>' heading")
    if not "\n".join(lines[1:]).strip():
        raise ValueError(f"{path} has a heading but no body; write the release note or remove it")
    return contents


def comparison_base(version, root=ROOT):
    """Use the highest earlier reachable version; stable releases compare to stable."""
    current = version_key(version)
    prerelease = parse_version(version)[1]
    candidates = [name for name in git("tag", "--merged", "HEAD", root=root).splitlines()
                  if name.startswith("v") and VERSION.fullmatch(name[1:])
                  and version_key(name[1:]) < current
                  and (prerelease or parse_version(name[1:])[1] is None)]
    return max(candidates, key=lambda name: version_key(name[1:]), default="")


def compose_notes(version, repository, root=ROOT):
    tag, _ = validate_tag(version, root)
    note = handwritten_note(root / "docs/release-notes" / f"{version}.md")
    previous = comparison_base(version, root)
    commit_range = f"{previous}..HEAD" if previous else "HEAD"
    commits = git("log", commit_range, "--pretty=format:- %s (%h)", "--reverse", root=root)
    paragraphs = [note] if note else []
    paragraphs.append("## Changelog")
    if previous:
        paragraphs.append(f"Changes since {previous}:")
    paragraphs.append(commits)
    if previous:
        paragraphs.append(f"[Full diff](https://github.com/{repository}/compare/{previous}...{tag})")
    return "\n\n".join(paragraphs) + "\n"


def get_release(repository, tag):
    token = os.environ["GH_TOKEN"]
    request = Request(
        f"https://api.github.com/repos/{repository}/releases/tags/{tag}",
        headers={"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json",
                 "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "seiso-release"},
    )
    try:
        with urlopen(request, timeout=30) as response:
            return json.load(response)
    except HTTPError as error:
        if error.code == 404:
            return None
        raise


def publish_github(version, repository, notes, directory, execute=False, root=ROOT):
    tag, sha = validate_tag(version, root)
    prerelease = parse_version(version)[1] is not None
    assets = sorted(path for path in directory.iterdir()
                    if path.is_file() and path.name.endswith((".whl", ".tar.gz", ".tgz", ".crate")))
    if not notes.is_file() or not assets:
        raise ValueError("GitHub Release requires generated notes and distribution artifacts")
    if not execute:
        print(f"Would create {repository} {tag} at {sha} with {len(assets)} assets")
        return
    existing = get_release(repository, tag)
    if existing is None:
        command = ["gh", "release", "create", tag, "--repo", repository, "--target", sha,
                   "--title", f"seiso {tag}", "--notes-file", str(notes)]
        if prerelease:
            command.extend(["--prerelease", "--latest=false"])
        run(command, root)
        published = set()
    else:
        if existing["draft"]:
            raise ValueError(f"{tag} has a draft release; review it before retrying")
        if existing["prerelease"] != prerelease:
            raise ValueError(f"{tag} has an inconsistent prerelease flag; correct it before retrying")
        published = {asset["name"] for asset in existing["assets"]}
        print(f"GitHub Release {tag} already exists; uploading only missing assets")
    missing = [str(asset) for asset in assets if asset.name not in published]
    if missing:
        run(["gh", "release", "upload", tag, "--repo", repository, *missing], root)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["notes", "publish"])
    parser.add_argument("--output", type=Path, default=ROOT / "release-notes.md")
    parser.add_argument("--dist", type=Path, default=ROOT / "dist")
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args()
    repository = os.environ.get("GITHUB_REPOSITORY", "scarletkc/seiso")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("GITHUB_REPOSITORY must be OWNER/REPO")
    version = release_metadata()
    if args.command == "notes":
        contents = compose_notes(version, repository)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(contents, encoding="utf-8", newline="\n")
        print(f"Wrote {args.output}")
    else:
        publish_github(version, repository, args.output, args.dist, args.execute)


if __name__ == "__main__":
    main()
