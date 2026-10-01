"""Benchmark sparse DUP003 prefix sorting against an optional baseline binary.

Run with release binaries, for example:
  python -m scripts.evaluation.benchmark_prefix_sort --binary target/release/seiso
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import random
import shutil
import statistics
import subprocess
import tempfile
import time


def fixture(workspace: Path, files: int) -> None:
    """Write deterministic sparse prose with one duplicate pair for diagnostics."""
    (workspace / "seiso.toml").write_text(
        'include = ["**/*.md"]\npreview = true\n'
        '[[kinds]]\npath = "**/*.md"\nkind = "reference"\n',
        encoding="utf-8",
    )
    shared = None
    for number in range(files):
        rng = random.Random(number)
        words = [
            "".join(chr(ord("a") + rng.randrange(26)) for _ in range(10))
            for _ in range(180)
        ]
        paragraph = " ".join(words)
        if number == 0:
            shared = paragraph
        elif number == 1:
            paragraph = shared
        (workspace / f"document-{number:04}.md").write_text(
            f"---\nkind: reference\n---\n# Document {number}\n\n{paragraph}\n",
            encoding="utf-8",
        )


def measure(
    binaries: dict[str, Path], workspaces: dict[str, Path], repeats: int
) -> dict[str, dict]:
    """Compare timings and stable diagnostic hashes across four cache modes."""
    results = {name: {} for name in binaries}
    for mode in ("cold", "warm", "no-cache", "selected"):
        samples = {name: [] for name in binaries}
        for repeat in range(repeats):
            # Alternate order so page-cache warming does not always favor the
            # second binary. Cold runs clear Seiso's parse cache before each run.
            order = list(binaries.items())
            if repeat % 2:
                order.reverse()
            for name, binary in order:
                workspace = workspaces[name]
                if mode == "cold":
                    cache = workspace / ".seiso_cache"
                    if cache.exists():
                        shutil.rmtree(cache)
                command = [
                    str(binary), "check", "--select", "DUP003,OWN002",
                    "--output-format", "json",
                ]
                if mode == "no-cache":
                    command.append("--no-cache")
                if mode == "selected":
                    command.append("document-0000.md")
                start = time.perf_counter()
                result = subprocess.run(command, cwd=workspace, capture_output=True, check=False)
                seconds = time.perf_counter() - start
                if result.returncode not in (0, 1):
                    raise RuntimeError(
                        f"{command!r} exited {result.returncode}: {result.stderr[:1000]!r}"
                    )
                if not json.loads(result.stdout):
                    raise ValueError(f"{mode} produced no diagnostics; fixture is ineffective")
                samples[name].append((seconds, hashlib.sha256(result.stdout).hexdigest()))
        for name, runs in samples.items():
            hashes = {digest for _, digest in runs}
            if len(hashes) != 1:
                raise ValueError(f"{name} {mode} output changed between runs")
            results[name][mode] = {
                "median_seconds": round(statistics.median(seconds for seconds, _ in runs), 6),
                "output_sha256": hashes.pop(),
            }
    for name, modes in results.items():
        if len({modes[mode]["output_sha256"] for mode in ("cold", "warm", "no-cache")}) != 1:
            raise ValueError(f"{name} full-workspace output changed between cache modes")
    return results


def main() -> None:
    """Validate inputs and compare same-version binaries in isolated workspaces."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, help="same-version release binary to compare")
    parser.add_argument("--files", type=int, default=200)
    parser.add_argument("--repeats", type=int, default=4)
    args = parser.parse_args()
    if args.files < 2 or args.repeats < 1:
        parser.error("--files must be at least 2 and --repeats must be positive")
    binaries = {"current": args.binary.resolve()}
    if args.baseline:
        binaries["baseline"] = args.baseline.resolve()
        versions = {
            name: subprocess.run(
                [str(binary), "--version"], capture_output=True, check=True
            ).stdout
            for name, binary in binaries.items()
        }
        if len(set(versions.values())) != 1:
            parser.error("--baseline must have the same package version for byte-identical diagnostics")
    with tempfile.TemporaryDirectory(prefix="seiso-prefix-sort-") as directory:
        workspaces = {name: Path(directory) / name for name in binaries}
        for workspace in workspaces.values():
            workspace.mkdir()
            fixture(workspace, args.files)
        measurements = measure(binaries, workspaces, args.repeats)
    if "baseline" in measurements:
        for mode in measurements["current"]:
            if measurements["current"][mode]["output_sha256"] != measurements["baseline"][mode]["output_sha256"]:
                raise ValueError(f"{mode} diagnostics differ from baseline")
    for name, modes in measurements.items():
        for mode, result in modes.items():
            print(f"{name} {mode}: {result['median_seconds']:.6f} s, sha256={result['output_sha256']}")


if __name__ == "__main__":
    main()
