"""Check the installed CLI and optional Python metadata against Cargo.toml."""

import argparse
from importlib.metadata import version as installed_version
from pathlib import Path
import shutil
import subprocess
import tomllib

from .versions import python_version


ROOT = Path(__file__).resolve().parents[2]


def verify(root, command, python_package=False):
    version = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    expected_python = python_version(version)
    output = subprocess.check_output(
        [shutil.which(command[0]) or command[0], *command[1:], "--version"],
        cwd=root, text=True, encoding="utf-8",
    ).strip()
    if output != f"seiso {version}":
        raise ValueError(f"CLI version mismatch: expected 'seiso {version}', got {output!r}")
    if python_package:
        actual = installed_version("seiso")
        if actual != expected_python:
            raise ValueError(f"Python version mismatch: expected {expected_python}, got {actual}")
    print(f"Verified installed {output}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--python-package", action="store_true", help="Also check installed Python metadata")
    parser.add_argument("command", nargs=argparse.REMAINDER, help="CLI command after -- (default: seiso)")
    args = parser.parse_args()
    command = args.command
    if command[:1] == ["--"]:
        command = command[1:]
    verify(ROOT, command or ["seiso"], args.python_package)


if __name__ == "__main__":
    main()
