"""Assemble the npm package from the verified binary wheels."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import tomllib
import zipfile
from email.parser import BytesParser

from .versions import python_version


def prepare(wheels: Path, output: Path, root: Path) -> None:
    version = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    package = json.loads((output / "package.json").read_text(encoding="utf-8"))
    if package["name"] != "@scarletkc/seiso" or package["version"] != version:
        raise ValueError("npm package must use @scarletkc/seiso and the Cargo package version")
    binaries = {}
    for wheel in sorted(wheels.glob("*.whl")):
        if wheel.name.endswith("-win_amd64.whl"):
            target, name = "win32-x64/seiso.exe", "seiso.exe"
        elif "manylinux" in wheel.name and wheel.name.endswith("_x86_64.whl"):
            target, name = "linux-x64/seiso", "seiso"
        else:
            continue
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
    if set(binaries) != {"linux-x64/seiso", "win32-x64/seiso.exe"}:
        raise ValueError("Both Linux x64 manylinux and Windows x64 wheels are required")
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
