"""Assemble the npm package from the verified binary wheels."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import tomllib
import zipfile
from email.parser import BytesParser

from .versions import python_version


NPM_PLATFORMS = ("linux-x64", "linux-arm64", "darwin-x64", "darwin-arm64", "win32-x64")
ARCHITECTURES = {"_x86_64": "x64", "_aarch64": "arm64", "_arm64": "arm64"}


def npm_platform(wheel):
    """Return the npm platform directory that a binary wheel supplies, or None."""
    tag = wheel.stem.split("-")[-1]
    if tag == "win_amd64":
        return "win32-x64"
    system = {"manylinux": "linux", "macosx": "darwin"}.get(re.match(r"[a-z]*", tag)[0])
    arch = next((npm for suffix, npm in ARCHITECTURES.items() if tag.endswith(suffix)), None)
    return f"{system}-{arch}" if system and arch else None


def prepare(wheels: Path, output: Path, root: Path) -> None:
    version = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    package = json.loads((output / "package.json").read_text(encoding="utf-8"))
    if package["name"] != "@scarletkc/seiso" or package["version"] != version:
        raise ValueError("npm package must use @scarletkc/seiso and the Cargo package version")
    binaries = {}
    for wheel in sorted(wheels.glob("*.whl")):
        platform = npm_platform(wheel)
        if platform is None:
            continue
        name = "seiso.exe" if platform.startswith("win32-") else "seiso"
        target = f"{platform}/{name}"
        if target in binaries:
            raise ValueError(f"More than one wheel supplies {target}")
        with zipfile.ZipFile(wheel) as archive:
            metadata_paths = [entry for entry in archive.namelist() if entry.endswith(".dist-info/METADATA")]
            scripts = [entry for entry in archive.namelist() if entry.endswith(f".data/scripts/{name}")]
            if len(metadata_paths) != 1 or len(scripts) != 1:
                raise ValueError(f"{wheel.name} must contain exactly one metadata record and seiso executable")
            metadata = BytesParser().parsebytes(archive.read(metadata_paths[0]))
            if metadata["Name"] != "seiso" or metadata["Version"] != python_version(version):
                raise ValueError(f"{wheel.name} does not match seiso {version}")
            binaries[target] = archive.read(scripts[0])
            if not binaries[target]:
                raise ValueError(f"{wheel.name} contains an empty executable")
    missing = sorted(set(NPM_PLATFORMS) - {target.split("/")[0] for target in binaries})
    if missing:
        raise ValueError(f"Wheels are required for every npm platform; missing {', '.join(missing)}")
    checksums = {}
    for target, data in sorted(binaries.items()):
        destination = output / "native" / target
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
        destination.chmod(0o755)
        checksums[target] = hashlib.sha256(data).hexdigest()
    (output / "native" / "manifest.json").write_text(
        json.dumps({"version": version, "sha256": checksums}, indent=2) + "\n", encoding="utf-8"
    )
    for name in ["README.md", "LICENSE"]:
        shutil.copyfile(root / name, output / name)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wheels", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    prepare(args.wheels, root / "npm" / "seiso", root)
