"""Measure the fixed M2 workload with process startup and file-backed output."""

from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import threading
import time


FILE_COUNT = 1_000
BYTES_PER_FILE = 10_000
FIXTURE_VERSION = 1
TARGETS_SECONDS = {"cold": 1.0, "warm": 0.5, "hook_warm": 0.15}


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def document_bytes(group: int, variant: int) -> bytes:
    """Ten files per group: five exact copies and five closely related variants."""
    revision = 0 if variant < 5 else variant
    sections = [
        "---\nkind: reference\n---\n",
        f"# Service group {group:03d}\n\n",
        f"This reference describes service group {group:03d}, revision {revision}. "
        "The shared template keeps deployment, recovery, and verification instructions together.\n\n",
        "## Fields\n\n",
        f"- `service_{group:03d}`: Identifies the service.\n"
        "- `endpoint`: Names the destination.\n"
        "- `timeout`: Bounds the wait.\n"
        "- `retries`: Limits recovery attempts.\n\n",
        "See [the field reference](#fields).\n\n",
    ]
    if group % 10 == 0:
        sections.append("See [the missing fixture](missing.md).\n\n")
    for section in range(15):
        sections.append(f"## Procedure {section:02d}\n\n")
        sections.append(
            f"For service group {group:03d}, procedure {section:02d} uses revision {revision}. "
            "Read the endpoint and timeout before submitting a request. "
            "Record the request identifier with the observed response so that another operator can reproduce the result. "
            "When the connection closes, verify the stored state before retrying. "
            "A retry preserves the request identifier and applies the same validation steps. "
            "The recovery record names the affected service and includes the response received from the endpoint.\n\n"
        )
    content = "".join(sections).encode("utf-8")
    # Fill with ordinary prose, not comments or code that the parser/rules skip.
    filler = b"The operator records the observed state before continuing the procedure. "
    available = BYTES_PER_FILE - len(content) - 1
    if available < 0:
        raise ValueError("The fixed fixture exceeds its byte budget")
    content += (filler * (available // len(filler) + 1))[:available] + b"\n"
    assert len(content) == BYTES_PER_FILE
    return content


def create_fixture(directory: Path) -> dict:
    directory.mkdir(parents=True, exist_ok=False)
    (directory / "seiso.toml").write_text('include = ["**/*.md"]\n', encoding="utf-8")
    (directory / ".gitignore").write_text(".seiso_cache/\n", encoding="utf-8")
    digests: Counter[str] = Counter()
    digest = hashlib.sha256()
    for index in range(FILE_COUNT):
        group, variant = divmod(index, 10)
        filename = f"group-{group:03d}/document-{variant:02d}.md"
        path = directory / filename
        path.parent.mkdir(exist_ok=True)
        content = document_bytes(group, variant)
        path.write_bytes(content)
        digests[sha256(content)] += 1
        digest.update(filename.encode("utf-8") + b"\0" + content + b"\0")
    return {
        "version": FIXTURE_VERSION,
        "files": FILE_COUNT,
        "bytes": FILE_COUNT * BYTES_PER_FILE,
        "sha256": digest.hexdigest(),
        "distinct_contents": len(digests),
        "files_in_exact_duplicate_groups": sum(count for count in digests.values() if count > 1),
        "construction": "100 groups of 10 documents; five exact template copies and five nearby variants per group; 15 similar procedure paragraphs per file; all padding is prose",
        "intentionally_missing_links": 100,
    }


def capture(command: list[str]) -> str | None:
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=10, check=False)
        return result.stdout.strip() if result.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired):
        return None


def host_information() -> dict:
    cpu = platform.processor()
    cpuinfo = Path("/proc/cpuinfo")
    if cpuinfo.is_file():
        for line in cpuinfo.read_text(encoding="utf-8").splitlines():
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
    standard_ci = (os.environ.get("GITHUB_ACTIONS") == "true" and platform.system() == "Linux"
                   and os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted")
    wsl = "microsoft" in platform.release().lower()
    return {
        "system": platform.system(), "release": platform.release(),
        "machine": platform.machine(), "cpu": cpu, "logical_cpus": os.cpu_count(),
        "python": platform.python_version(), "rustc": capture(["rustc", "--version"]),
        "cargo": capture(["cargo", "--version"]),
        "runner": "github_actions_linux" if standard_ci else "local_wsl_linux" if wsl else "local",
        "standard_linux_ci": standard_ci,
        "runner_os": os.environ.get("RUNNER_OS"),
        "runner_arch": os.environ.get("RUNNER_ARCH"),
        "runner_environment": os.environ.get("RUNNER_ENVIRONMENT"),
        "image_os": os.environ.get("ImageOS"),
        "image_version": os.environ.get("ImageVersion"),
    }


def timed_run(command: list[str], cwd: Path, output: Path, payload: bytes | None = None,
              hook: bool = False, timeout: float = 300.0) -> dict:
    with output.open("wb") as stdout, output.with_suffix(".stderr").open("wb") as stderr:
        started = time.perf_counter()
        process = subprocess.Popen(command, cwd=cwd, stdin=subprocess.PIPE if payload is not None else subprocess.DEVNULL,
                                   stdout=stdout, stderr=stderr)
        timed_out = threading.Event()

        def expire() -> None:
            if process.poll() is None:
                timed_out.set()
                try:
                    process.kill()
                except ProcessLookupError:
                    pass

        timer = threading.Timer(max(0.001, timeout - (time.perf_counter() - started)), expire)
        timer.daemon = True
        timer.start()
        try:
            # wait(timeout) uses polling sleeps on POSIX. A separate watchdog lets
            # communicate block in waitpid without adding polling delays.
            process.communicate(input=payload)
        finally:
            timer.cancel()
            timer.join()
        elapsed = time.perf_counter() - started
        if timed_out.is_set():
            raise subprocess.TimeoutExpired(command, timeout)
    accepted = (0, 2) if hook else (0, 1)
    if process.returncode not in accepted:
        error = output.with_suffix(".stderr").read_text(encoding="utf-8", errors="replace")
        raise RuntimeError(f"{command!r} exited {process.returncode}: {error[:4000]}")
    diagnostic_path = output.with_suffix(".stderr") if hook else output
    data = diagnostic_path.read_bytes()
    return {"seconds": elapsed, "exit_code": process.returncode,
            "output_sha256": sha256(data), "output_bytes": len(data)}


def clear_cache(workspace: Path) -> None:
    cache = workspace / ".seiso_cache"
    # This directory belongs to a fresh TemporaryDirectory generated by this script.
    if cache.exists():
        if not cache.resolve().is_relative_to(workspace.resolve()):
            raise ValueError("The cache path escaped the generated benchmark workspace")
        shutil.rmtree(cache)


def summarize_samples(samples: list[dict], target: float | None = None) -> dict:
    outputs = {sample["output_sha256"] for sample in samples}
    if len(outputs) != 1:
        raise ValueError("Diagnostic output changed between repeated benchmark samples")
    median = statistics.median(sample["seconds"] for sample in samples)
    return {"samples": samples, "median_seconds": median, "minimum_seconds": min(sample["seconds"] for sample in samples),
            "maximum_seconds": max(sample["seconds"] for sample in samples), "target_seconds": target,
            "target_met": median < target if target is not None else None,
            "output_sha256": samples[0]["output_sha256"], "output_bytes": samples[0]["output_bytes"]}


def measure_binary(binary: Path, workspace: Path, results: Path, repetitions: int,
                   timeout: float, include_preview: bool) -> dict:
    results.mkdir()
    help_text = capture([str(binary), "check", "--help"])
    if help_text is None:
        raise RuntimeError(f"Cannot inspect executable {binary}")
    cache_supported = "--no-cache" in help_text
    command = [str(binary), "check", "--output-format", "concise"]
    modes: dict[str, dict] = {}
    for name in ["cold", "warm", "disabled"]:
        samples = []
        for repeat in range(repetitions):
            if name == "cold":
                clear_cache(workspace)
            arguments = command + (["--no-cache"] if name == "disabled" and cache_supported else [])
            samples.append(timed_run(arguments, workspace, results / f"{name}-{repeat}.txt", timeout=timeout))
        modes[name] = summarize_samples(samples, TARGETS_SECONDS.get(name))
        print(f"{results.name} {name}: {modes[name]['median_seconds']:.6f} s median", file=sys.stderr, flush=True)
    if len({modes[name]["output_sha256"] for name in ("cold", "warm", "disabled")}) != 1:
        raise ValueError("Cold, warm, and disabled cache diagnostics differ")
    relative = "group-000/document-00.md"
    payload = json.dumps({"cwd": str(workspace), "tool_name": "Write", "tool_input": {"file_path": str(workspace / relative)}}).encode()
    hook_command = [str(binary), "hook", "claude-code"]
    samples = [timed_run(hook_command, workspace, results / f"hook-{repeat}.txt", payload, True, timeout)
               for repeat in range(repetitions)]
    modes["hook_warm"] = summarize_samples(samples, TARGETS_SECONDS["hook_warm"])
    print(f"{results.name} hook_warm: {modes['hook_warm']['median_seconds']:.6f} s median", file=sys.stderr, flush=True)
    expected_hook = b"".join(line for line in (results / "warm-0.txt").read_bytes().splitlines(keepends=True)
                             if line.startswith(relative.encode() + b":"))
    if sha256(expected_hook) != modes["hook_warm"]["output_sha256"]:
        raise ValueError("Hook output differs from the full report filtered to the edited file")
    if include_preview:
        print(f"{results.name} preview: separate measurement, {timeout:g} s timeout per sample", file=sys.stderr, flush=True)
        preview_samples = []
        try:
            for repeat in range(repetitions):
                preview_started = time.perf_counter()
                preview_samples.append(timed_run(command + ["--preview"], workspace, results / f"preview-{repeat}.txt", timeout=timeout))
            modes["preview_warm"] = summarize_samples(preview_samples) | {"status": "ok"}
        except subprocess.TimeoutExpired:
            modes["preview_warm"] = {"status": "timeout", "timeout_seconds": timeout, "samples": preview_samples,
                                     "elapsed_seconds": time.perf_counter() - preview_started,
                                     "median_seconds": None, "target_seconds": None, "target_met": None}
    return {"binary": str(binary), "binary_sha256": sha256(binary.read_bytes()),
            "version": capture([str(binary), "--version"]), "cache_supported": cache_supported,
            "cache_mode_note": "cold clears only seiso's parse cache; filesystem page cache is uncontrolled" if cache_supported else
                "This baseline has no parse cache. Cold, warm, and disabled labels all run its ordinary check command.",
            "cold_warm_disabled_equal": True, "hook_matches_selected_report": True, "modes": modes}


def comparison(current: dict, baseline: dict) -> dict:
    result = {}
    for name in TARGETS_SECONDS:
        old = baseline["modes"][name]["median_seconds"]
        new = current["modes"][name]["median_seconds"]
        ratio = new / old
        result[name] = {"current_seconds": new, "baseline_seconds": old,
                        "ratio": ratio, "change_seconds": new - old,
                        "change_percent": (ratio - 1.0) * 100.0,
                        "output_equal": current["modes"][name].get("output_sha256") == baseline["modes"][name].get("output_sha256")}
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--baseline-binary", type=Path)
    parser.add_argument("--workdir", type=Path, help="Parent of temporary input files; use native Linux storage on Linux")
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--timeout", type=float, default=300)
    parser.add_argument("--preview-stress", action="store_true", help="Also measure the dense preview workload; timeouts are reported separately")
    parser.add_argument("--enforce-targets", action="store_true", help="Fail on unmet latency targets; requires a standard Linux CI run")
    args = parser.parse_args()
    if args.repetitions < 1:
        parser.error("--repetitions must be positive")
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    if args.workdir is not None:
        args.workdir.mkdir(parents=True, exist_ok=True)
    report = {"schema_version": 1, "measured_at": datetime.now(timezone.utc).isoformat(),
              "host": host_information(), "includes_process_startup": True,
              "stdout_sink": "ordinary file", "rule_selection": "default stable; preview measured separately",
              "script_sha256": sha256(Path(__file__).read_bytes())}
    with tempfile.TemporaryDirectory(prefix="seiso-m2-benchmark-", dir=args.workdir) as temporary:
        base = Path(temporary)
        workspace = base / "fixture"
        report["fixture"] = create_fixture(workspace)
        report["input_filesystem_path"] = str(base)
        report["current"] = measure_binary(binary, workspace, base / "current", args.repetitions, args.timeout, args.preview_stress)
        if args.baseline_binary:
            report["baseline"] = measure_binary(args.baseline_binary.resolve(strict=True), workspace, base / "baseline", args.repetitions, args.timeout, False)
            report["comparison"] = comparison(report["current"], report["baseline"])
    report["latency_targets_met_locally"] = all(report["current"]["modes"][name]["target_met"] for name in TARGETS_SECONDS)
    report["standard_linux_ci_acceptance"] = report["host"]["standard_linux_ci"] and report["latency_targets_met_locally"]
    output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(output), "medians_seconds": {name: mode["median_seconds"] for name, mode in report["current"]["modes"].items()},
                      "standard_linux_ci_acceptance": report["standard_linux_ci_acceptance"]}, ensure_ascii=False))
    if args.enforce_targets and not report["standard_linux_ci_acceptance"]:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
