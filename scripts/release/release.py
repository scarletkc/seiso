"""Validate release versions and publish verified packages only with --execute."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tomllib
from urllib.error import HTTPError
from urllib.parse import quote
from urllib.request import Request, urlopen

from .prepare_npm import NPM_PLATFORMS, PACKAGE, optional_dependencies
from .versions import parse_version


ROOT = Path(__file__).resolve().parents[2]
NPM_REGISTRY = "https://registry.npmjs.org"


def run(args, root=ROOT):
    print("+ " + " ".join(map(str, args)), flush=True)
    subprocess.run([shutil.which(args[0]) or args[0], *map(str, args[1:])], cwd=root, check=True)


def validate(root, metadata, crates=False):
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    version = manifest["package"]["version"]
    if manifest["package"]["name"] != "seiso":
        raise ValueError("The root Cargo package must be named seiso")
    parse_version(version)
    npm = json.loads((root / "npm/seiso/package.json").read_text(encoding="utf-8"))
    if npm["name"] != PACKAGE or npm["version"] != version:
        raise ValueError(f"npm name/version must be {PACKAGE} and the Cargo package version")
    if npm.get("optionalDependencies") != optional_dependencies(version):
        raise ValueError(f"npm optionalDependencies must pin every platform package to {version}")
    python = tomllib.loads((root / "pyproject.toml").read_text(encoding="utf-8"))
    if (python["project"]["name"] != "seiso"
            or "version" not in python["project"].get("dynamic", [])
            or "version" in python["project"]
            or python["tool"]["maturin"]["manifest-path"] != "Cargo.toml"):
        raise ValueError("PyPI must derive seiso's version from the root Cargo.toml")
    packages = [p for p in metadata["packages"] if p["name"] == "seiso"
                and p["id"] in metadata["workspace_members"]
                and Path(p["manifest_path"]).resolve() == (root / "Cargo.toml").resolve()]
    if len(packages) != 1:
        raise ValueError("Cargo metadata must identify the root seiso package")
    package = packages[0]
    lock = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))
    locked = [p["version"] for p in lock["package"] if p["name"] == "seiso" and "source" not in p]
    if package["version"] != version or locked != [version]:
        raise ValueError(f"seiso: Cargo manifest/lock version differs from {version}")
    if crates and package["publish"] is not None and "crates-io" not in package["publish"]:
        raise ValueError("seiso: publishing to crates.io must be enabled")
    # Cargo package validates publishable dependencies and compiles the archive.
    return version


def release_metadata(root=ROOT, crates=False):
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--locked", "--format-version", "1"],
        cwd=root, check=True, capture_output=True, text=True,
    )
    version = validate(root, json.loads(result.stdout), crates=crates)
    print(f"Release seiso {version}", flush=True)
    return version


def registry_text(url):
    request = Request(url, headers={"User-Agent": "seiso-release (https://github.com/scarletkc/seiso)"})
    try:
        with urlopen(request, timeout=30) as response:
            return response.read().decode("utf-8")
    except HTTPError as error:
        if error.code == 404:
            return None
        raise


def crate_exists(name, version):
    lower = name.lower()
    prefix = ("1" if len(lower) == 1 else "2" if len(lower) == 2
              else f"3/{lower[0]}" if len(lower) == 3 else f"{lower[:2]}/{lower[2:4]}")
    contents = registry_text(f"https://index.crates.io/{prefix}/{lower}")
    if contents is None:
        return False
    if not contents.strip():
        raise ValueError(f"Empty crates.io index response for {name}")
    for line in contents.splitlines():
        entry = json.loads(line)
        if entry["vers"] == version:
            if entry["yanked"]:
                raise ValueError(f"{name} {version} is yanked; resolve it before resuming publication")
            return True
    return False


def publish_crates(version, execute=False):
    parse_version(version)
    command = ["cargo", "publish" if execute else "package", "--package", "seiso", "--locked", "--registry", "crates-io"]
    if not execute:
        run(command)
        return
    if crate_exists("seiso", version):
        print(f"Skip crates.io: seiso {version} already exists", flush=True)
        return
    run(command)


def npm_archives(version, directory):
    """Return the packed npm archives in upload order: platform packages, then the main package."""
    archives = {}
    for archive in sorted(directory.glob("*.tgz")):
        with tarfile.open(archive) as bundle:
            package = json.load(bundle.extractfile("package/package.json"))
        if package["version"] != version:
            raise ValueError(f"{archive.name}: npm archive version differs from the release")
        if package["name"] in archives:
            raise ValueError(f"More than one npm archive supplies {package['name']}")
        archives[package["name"]] = archive.resolve()
    expected = [*(f"{PACKAGE}-{platform}" for platform in NPM_PLATFORMS), PACKAGE]
    if sorted(archives) != sorted(expected):
        raise ValueError(f"Expected npm archives for exactly {', '.join(expected)} in {directory}")
    return [(name, archives[name]) for name in expected]


def publish_npm(version, directory, execute=False):
    stage = parse_version(version)[1]
    # Publishing the main package last keeps its optional dependencies resolvable once it is visible.
    for name, archive in npm_archives(version, directory):
        if execute:
            contents = registry_text(f"{NPM_REGISTRY}/{quote(name, safe='')}/{version}")
            if contents is not None:
                existing = json.loads(contents)
                if existing["name"] != name or existing["version"] != version:
                    raise ValueError("npm registry returned unexpected package metadata")
                print(f"Skip npm: {name} {version} already exists", flush=True)
                continue
        # npm publish --dry-run rejects an existing version, blocking partial-release retries.
        # Packing the tested archive validates it without requiring registry availability.
        command = (["npm", "publish", str(archive), "--access", "public", "--ignore-scripts",
                    "--registry", NPM_REGISTRY, "--tag", stage or "latest"] if execute else
                   ["npm", "pack", str(archive), "--dry-run", "--ignore-scripts"])
        run(command)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["check", "crates", "npm"])
    parser.add_argument("--execute", action="store_true", help="Upload missing versions to the registry")
    parser.add_argument("--dist", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    version = release_metadata(crates=args.command == "crates")
    if args.command == "check":
        if output := os.environ.get("GITHUB_OUTPUT"):
            with open(output, "a", encoding="utf-8") as stream:
                stream.write(f"version={version}\n")
    elif args.command == "crates":
        publish_crates(version, args.execute)
    else:
        publish_npm(version, args.dist, args.execute)


if __name__ == "__main__":
    main()
