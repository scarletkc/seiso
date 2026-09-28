"""Parse every locked corpus document in isolated processes and write reproducible evidence."""

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def build_probe():
    cargo = shutil.which("cargo") or str(Path.home() / ".cargo/bin" / ("cargo.exe" if os.name == "nt" else "cargo"))
    process = subprocess.run(
        [cargo, "build", "--locked", "--release", "--example", "corpus_probe", "--message-format=json"],
        cwd=ROOT, capture_output=True,
    )
    if process.returncode:
        raise RuntimeError(process.stderr.decode("utf-8", errors="replace"))
    for line in process.stdout.splitlines():
        event = json.loads(line)
        if event.get("reason") == "compiler-artifact" and event.get("target", {}).get("name") == "corpus_probe" and event.get("executable"):
            return Path(event["executable"])
    raise RuntimeError("Cargo did not report a corpus_probe executable")


def parser_sources():
    paths = [ROOT / "Cargo.toml", ROOT / "Cargo.lock", Path(__file__).resolve()]
    paths.extend((ROOT / "src").rglob("*.rs"))
    paths.extend((ROOT / "docs/rules").glob("*.md"))
    paths.append(ROOT / "examples/corpus_probe.rs")
    return {path.relative_to(ROOT).as_posix(): digest(path) for path in sorted(paths)}


def load_corpus(corpus):
    lock_path = corpus / "corpus.lock.json"
    lock = json.loads(lock_path.read_text(encoding="utf-8"))
    if lock.get("schema_version") != 1 or not lock.get("sources"):
        raise ValueError("Unsupported or empty corpus lock")
    sources = {}
    seen_repositories = set()
    jobs = []
    for source in lock["sources"]:
        identifier = source["id"]
        if not re.fullmatch(r"[a-z0-9-]+", identifier) or identifier in sources:
            raise ValueError("Duplicate or invalid corpus source ID")
        if source["split"] not in {"tuning", "holdout"} or source["language_focus"] not in {"en", "zh", "ja"}:
            raise ValueError(f"Invalid source split or language: {identifier}")
        if source["repository"].lower() in seen_repositories:
            raise ValueError("Corpus repositories must be disjoint across splits")
        seen_repositories.add(source["repository"].lower())
        if not re.fullmatch(r"[a-f0-9]{40}", source["commit"]):
            raise ValueError(f"Source {identifier} is not pinned to a full commit")
        sources[identifier] = {key: source[key] for key in ["repository", "commit", "split", "language_focus", "category"]}
        seen_paths = set()
        for document in source["documents"]:
            path = PurePosixPath(document["path"])
            if path.is_absolute() or ".." in path.parts or "\\" in str(path) or not path.parts or str(path) in seen_paths:
                raise ValueError(f"Unsafe or duplicate document path: {document['path']}")
            seen_paths.add(str(path))
            if not re.fullmatch(r"[a-f0-9]{40}", document["git_blob"]) or not re.fullmatch(r"[a-f0-9]{64}", document["sha256"]):
                raise ValueError(f"Document has no valid content hashes: {identifier}/{path}")
            if not isinstance(document["bytes"], int) or document["bytes"] < 0:
                raise ValueError("Invalid document byte count")
            jobs.append((identifier, document))
        if not seen_paths:
            raise ValueError(f"Corpus source has no documents: {identifier}")
    return digest(lock_path), sources, jobs


def run_one(probe, corpus, source, document, timeout):
    result = {"source": source, "path": document["path"], "sha256": document["sha256"], "bytes": document["bytes"]}
    path = corpus / "data/blobs" / document["git_blob"]
    try:
        content = path.read_bytes()
        git_hash = hashlib.sha1(b"blob " + str(len(content)).encode() + b"\0" + content).hexdigest()
        if len(content) != document["bytes"] or hashlib.sha256(content).hexdigest() != document["sha256"] or git_hash != document["git_blob"]:
            raise ValueError("cached input differs from the pinned bytes/hash")
    except (OSError, ValueError) as error:
        return result | {"status": "input_error", "error": str(error)}
    try:
        process = subprocess.run([str(probe), str(path)], capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return result | {"status": "timeout", "error": f"isolated process exceeded {timeout} seconds"}
    except OSError as error:
        return result | {"status": "process_error", "error": str(error)}
    try:
        outcome = json.loads(process.stdout)
        if not isinstance(outcome, dict) or outcome.get("status") not in {"ok", "panic", "model_error", "input_error", "nondeterministic"}:
            raise ValueError("invalid probe status")
        if outcome["status"] == "ok":
            flavors = outcome.get("flavors", [])
            if process.returncode != 0 or [item["flavor"] for item in flavors] != ["common_mark", "gfm"]:
                raise ValueError("successful probe did not cover both Markdown flavors")
            if any(item["source_sha256"] != document["sha256"] for item in flavors):
                raise ValueError("probe parsed bytes different from the pinned document")
    except (ValueError, KeyError, TypeError):
        return result | {"status": "process_error", "exit_code": process.returncode,
                         "error": process.stderr.decode("utf-8", errors="replace") or "probe did not return a valid complete result"}
    return result | outcome


def evaluate(corpus, probe, jobs, workers, timeout, reverse=False):
    results = []
    ordered = sorted(jobs, key=lambda item: (item[0], item[1]["path"]), reverse=reverse)
    with ThreadPoolExecutor(max_workers=workers) as executor:
        futures = [executor.submit(run_one, probe, corpus, source, document, timeout) for source, document in ordered]
        for count, future in enumerate(as_completed(futures), 1):
            result = future.result()
            results.append(result)
            if result["status"] != "ok":
                print(f"{result['status']}: {result['source']}/{result['path']}: {result.get('error')}", file=sys.stderr, flush=True)
            if count % 100 == 0 or count == len(jobs):
                print(f"Parsed {count}/{len(jobs)} locked documents", file=sys.stderr, flush=True)
    return sorted(results, key=lambda result: (result["source"], result["path"]))


def make_report(lock_hash, sources, results, probe):
    status = dict(sorted(Counter(result["status"] for result in results).items()))
    by_source = {}
    for identifier, metadata in sorted(sources.items()):
        documents = [result for result in results if result["source"] == identifier]
        by_source[identifier] = metadata | {"files": len(documents), "bytes": sum(item["bytes"] for item in documents),
                                             "status": dict(sorted(Counter(item["status"] for item in documents).items()))}
    return {
        "schema_version": 1,
        "corpus_sha256": lock_hash,
        "parser_version": tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"],
        "parser_sources": parser_sources(),
        "probe_sha256": digest(probe),
        "flavors": ["common_mark", "gfm"],
        "parses_per_flavor_per_file": 2,
        "summary": {
            "repositories": len(sources), "files": len(results), "bytes": sum(result["bytes"] for result in results),
            "status": status, "passed": bool(results) and status == {"ok": len(results)},
            "repositories_by_split": dict(sorted(Counter(source["split"] for source in sources.values()).items())),
            "repositories_by_language_focus": dict(sorted(Counter(source["language_focus"] for source in sources.values()).items())),
            "gfm_detected_languages": dict(sorted(Counter(flavor["language"] for result in results for flavor in result.get("flavors", []) if flavor["flavor"] == "gfm").items())),
            "gfm_frontmatter_content_errors": sum(flavor["frontmatter_errors"] for result in results for flavor in result.get("flavors", []) if flavor["flavor"] == "gfm"),
        },
        "sources": by_source,
        "documents": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--timeout", type=int, default=60)
    parser.add_argument("--reverse", action="store_true")
    args = parser.parse_args()
    if args.jobs < 1 or args.timeout < 1:
        parser.error("jobs and timeout must be positive")
    corpus = args.corpus.resolve()
    try:
        lock_hash, sources, jobs = load_corpus(corpus)
        probe = build_probe()
        results = evaluate(corpus, probe, jobs, args.jobs, args.timeout, args.reverse)
        report = make_report(lock_hash, sources, results, probe)
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2) + "\n", encoding="utf-8", newline="\n")
        print(json.dumps(report["summary"], ensure_ascii=False, sort_keys=True))
        return 0 if report["summary"]["passed"] else 1
    except (OSError, ValueError, RuntimeError, KeyError) as error:
        print(f"corpus: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
