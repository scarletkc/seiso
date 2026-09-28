"""Freeze diagnostics from the pinned documents, reviewed kinds, and full Git trees."""

import argparse
from collections import Counter
import gzip
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]


def encode(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode()


def digest(content):
    return hashlib.sha256(content).hexdigest()


def read(path):
    return json.loads(path.read_bytes())


def fingerprints():
    paths = [ROOT / "Cargo.toml", ROOT / "Cargo.lock", Path(__file__)]
    paths += sorted((ROOT / "src").rglob("*.rs"))
    paths += sorted((ROOT / "docs/rules").glob("*.md"))
    paths.append(ROOT / "examples/evaluate.rs")
    return {p.relative_to(ROOT).as_posix(): digest(p.read_bytes()) for p in sorted(paths)}


def run(output):
    if output.exists():
        raise ValueError(f"Preserve the existing run before creating another: {output}")
    corpus = ROOT / "corpus"
    lock = read(corpus / "corpus.lock.json")
    lock_hash = digest((corpus / "corpus.lock.json").read_bytes())
    profile_path = corpus / "evaluation/kinds.json"
    profiles = read(profile_path)
    if profiles["corpus_sha256"] != lock_hash:
        raise ValueError("Kind profile does not describe this corpus lock")
    subprocess.run([sys.executable, str(corpus / "inventory.py"), "verify"], check=True)
    trees = read(corpus / "inventory/inventory.lock.json")
    records = {item["id"]: item for item in trees["sources"]}
    inputs = []
    for source in lock["sources"]:
        tree = read_gzip(corpus / "inventory" / records[source["id"]]["archive"])
        profile = profiles["profiles"][source["id"]]
        config = "preview=true\n"
        for mapping in profile["mappings"]:
            config += f'\n[[kinds]]\npath={json.dumps(mapping["path"], ensure_ascii=False)}\nkind={json.dumps(mapping["kind"])}\n'
        inputs.append({"id": source["id"], "config": config, "entries": tree["entries"],
            "documents": [{"path": doc["path"], "blob": str(corpus / "data/blobs" / doc["git_blob"]), "sha256": doc["sha256"]} for doc in source["documents"]]})
    work = ROOT / "target/m1-evaluation"
    work.mkdir(parents=True, exist_ok=True)
    (work / "input.json").write_bytes(encode({"sources": inputs}))
    cargo = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo.exe")
    build = subprocess.run([cargo, "build", "--release", "--locked", "--example", "evaluate", "--message-format=json"], cwd=ROOT, capture_output=True, check=True)
    binary = next(Path(event["executable"]) for line in build.stdout.splitlines() if (event := json.loads(line)).get("reason") == "compiler-artifact" and event.get("target", {}).get("name") == "evaluate" and event.get("executable"))
    before = fingerprints()
    for name in ["forward", "repeat"]:
        subprocess.run([str(binary), str(work / "input.json"), str(work / f"{name}.json")], check=True)
    if (work / "forward.json").read_bytes() != (work / "repeat.json").read_bytes() or before != fingerprints():
        raise ValueError("Evaluation changed during its repeated run")
    files = read(work / "forward.json")
    by_source = {source["id"]: source for source in lock["sources"]}
    diagnostics = []
    for file in files:
        source = by_source[file["source"]]
        for diagnostic in file["result"]["diagnostics"]:
            identity = {"source": file["source"], "path": file["path"], "input_sha256": file["sha256"], "code": diagnostic["code"], "span": diagnostic["byte_range"]}
            diagnostics.append(identity | {"id": digest(encode(identity)), "split": source["split"], "commit": source["commit"],
                "repository": source["repository"], "language": file["language"], "kind": file["result"]["kind"]["value"] or "unknown", "diagnostic": diagnostic})
    diagnostics.sort(key=lambda d: (d["source"], d["path"], d["span"]["start"], d["code"]))
    assert len({d["id"] for d in diagnostics}) == len(diagnostics), "duplicate diagnostic identities"
    report = {"schema_version": 1, "corpus_sha256": lock_hash, "kind_profile_sha256": digest(profile_path.read_bytes()),
        "inventory_sha256": digest((corpus / "inventory/inventory.lock.json").read_bytes()), "implementation": before,
        "probe_sha256": digest(binary.read_bytes()), "link_backend": "case-sensitive pinned Git trees; symlinks and submodules undetermined",
        "repeat_byte_identical": True, "files": files, "diagnostics": diagnostics}
    output.mkdir(parents=True)
    packed = gzip.compress(encode(report), mtime=0)
    (output / "diagnostics.json.gz").write_bytes(packed)
    summary = {split: dict(sorted(Counter(d["code"] for d in diagnostics if d["split"] == split).items())) for split in ["tuning", "holdout"]}
    (output / "run.json").write_bytes(encode({key: value for key, value in report.items() if key not in {"files", "diagnostics"}} | {
        "report_sha256": digest(packed), "files": len(files), "diagnostics": len(diagnostics), "counts": summary}))
    print(json.dumps(summary, sort_keys=True))


def read_gzip(path):
    return json.loads(gzip.decompress(path.read_bytes()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    run(args.output)


if __name__ == "__main__":
    main()
