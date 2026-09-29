"""Assemble the npm platform packages from the verified binary wheels."""

import argparse
import json
from pathlib import Path
import re
import shutil
import tomllib
import zipfile
from email.parser import BytesParser

from .versions import python_version


PACKAGE = "@scarletkc/seiso"
# Each platform package declares the os, cpu, and libc values npm matches before installing it.
NPM_PLATFORMS = {
    "darwin-arm64": ("darwin", "arm64", None),
    "darwin-x64": ("darwin", "x64", None),
    "linux-arm64": ("linux", "arm64", "glibc"),
    "linux-arm64-musl": ("linux", "arm64", "musl"),
    "linux-x64": ("linux", "x64", "glibc"),
    "linux-x64-musl": ("linux", "x64", "musl"),
    "win32-arm64": ("win32", "arm64", None),
    "win32-x64": ("win32", "x64", None),
}
ARCHITECTURES = {"_x86_64": "x64", "_aarch64": "arm64", "_arm64": "arm64"}
WINDOWS = {"win_amd64": "win32-x64", "win_arm64": "win32-arm64"}


def npm_platform(wheel):
    """Return the npm platform that a binary wheel supplies, or None."""
    tag = wheel.stem.split("-")[-1]
    if tag in WINDOWS:
        return WINDOWS[tag]
    prefix = re.match(r"[a-z]*", tag)[0]
    system = {"manylinux": "linux", "musllinux": "linux", "macosx": "darwin"}.get(prefix)
    arch = next((npm for suffix, npm in ARCHITECTURES.items() if tag.endswith(suffix)), None)
    libc = "-musl" if prefix == "musllinux" else ""
    return f"{system}-{arch}{libc}" if system and arch else None


def executable(platform):
    return "seiso.exe" if platform.startswith("win32-") else "seiso"


def optional_dependencies(version):
    """Return the exact platform package pins that the main npm package must declare."""
    return {f"{PACKAGE}-{platform}": version for platform in NPM_PLATFORMS}


def platform_package(main, platform):
    system, cpu, libc = NPM_PLATFORMS[platform]
    repository = {key: value for key, value in main["repository"].items() if key != "directory"}
    package = {
        "name": f"{PACKAGE}-{platform}",
        "version": main["version"],
        "description": f"The seiso executable for {platform}; install {PACKAGE} instead",
        "license": main["license"],
        "author": main["author"],
        "repository": repository,
        "homepage": main["homepage"],
        "bugs": main["bugs"],
        "files": [executable(platform)],
        "os": [system],
        "cpu": [cpu],
    }
    if libc:
        package["libc"] = [libc]
    # Yarn Plug'n'Play would otherwise keep the executable inside a zip archive.
    package["preferUnplugged"] = True
    return package


def platform_readme(platform):
    return (f"# {PACKAGE}-{platform}\n\n"
            f"The `seiso` executable for `{platform}`. Install\n"
            f"[{PACKAGE}](https://www.npmjs.com/package/{PACKAGE}) instead; npm installs\n"
            "this package only on matching systems.\n")


def prepare(wheels: Path, npm: Path, root: Path) -> None:
    """Write npm/seiso-PLATFORM packages next to the main npm/seiso package."""
    version = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    main = json.loads((npm / "seiso" / "package.json").read_text(encoding="utf-8"))
    if main["name"] != PACKAGE or main["version"] != version:
        raise ValueError(f"npm package must use {PACKAGE} and the Cargo package version")
    if main.get("optionalDependencies") != optional_dependencies(version):
        raise ValueError(f"npm optionalDependencies must pin every platform package to {version}")
    binaries = {}
    for wheel in sorted(wheels.glob("*.whl")):
        platform = npm_platform(wheel)
        if platform is None:
            continue
        if platform in binaries:
            raise ValueError(f"More than one wheel supplies {platform}")
        name = executable(platform)
        with zipfile.ZipFile(wheel) as archive:
            metadata_paths = [entry for entry in archive.namelist() if entry.endswith(".dist-info/METADATA")]
            scripts = [entry for entry in archive.namelist() if entry.endswith(f".data/scripts/{name}")]
            if len(metadata_paths) != 1 or len(scripts) != 1:
                raise ValueError(f"{wheel.name} must contain exactly one metadata record and seiso executable")
            metadata = BytesParser().parsebytes(archive.read(metadata_paths[0]))
            if metadata["Name"] != "seiso" or metadata["Version"] != python_version(version):
                raise ValueError(f"{wheel.name} does not match seiso {version}")
            binaries[platform] = archive.read(scripts[0])
            if not binaries[platform]:
                raise ValueError(f"{wheel.name} contains an empty executable")
    missing = sorted(set(NPM_PLATFORMS) - set(binaries))
    if missing:
        raise ValueError(f"Wheels are required for every npm platform; missing {', '.join(missing)}")
    for stale in npm.glob("seiso-*"):
        shutil.rmtree(stale)
    for platform, data in sorted(binaries.items()):
        directory = npm / f"seiso-{platform}"
        directory.mkdir()
        binary = directory / executable(platform)
        binary.write_bytes(data)
        binary.chmod(0o755)
        (directory / "package.json").write_text(
            json.dumps(platform_package(main, platform), indent=2) + "\n", encoding="utf-8")
        (directory / "README.md").write_text(platform_readme(platform), encoding="utf-8")
        shutil.copyfile(root / "LICENSE", directory / "LICENSE")
    for name in ["README.md", "LICENSE"]:
        shutil.copyfile(root / name, npm / "seiso" / name)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wheels", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    prepare(args.wheels, root / "npm", root)
