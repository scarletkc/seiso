"""Freeze M2 diagnostics with pinned inputs and reversed-order equivalence."""

import argparse
from collections import Counter
import gzip
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
M2_RULES = ("PTR002", "LNK002", "DUP001", "DUP002", "DUP003", "OWN001", "OWN002")


def encode(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode()


def digest(content):
    return hashlib.sha256(content).hexdigest()


def read(path):
    return json.loads(path.read_bytes())


def source_digest(path):
    """Identify implementation text independently of checkout line endings."""
    return digest(path.read_bytes().replace(b"\r\n", b"\n"))


def fingerprints():
    paths = [ROOT / "Cargo.toml", ROOT / "Cargo.lock", Path(__file__)]
    paths += sorted((ROOT / "src").rglob("*.rs"))
    paths += sorted((ROOT / "docs/rules").glob("*.md"))
    paths.append(ROOT / "examples/evaluate_m2.rs")
    return {path.relative_to(ROOT).as_posix(): source_digest(path) for path in sorted(paths)}


def reverse_inputs(batch):
    return batch | {"sources": [source | {"documents": list(reversed(source["documents"])),
                                  "entries": list(reversed(source["entries"]))}
                        for source in reversed(batch["sources"])]}


def verify_blob(path, record):
    raw = path.read_bytes()
    if len(raw) != record["bytes"] or digest(raw) != record["sha256"]:
        raise ValueError(f"Corpus content changed: {path.name}")
    git_blob = hashlib.sha1(f"blob {len(raw)}\0".encode() + raw).hexdigest()
    if git_blob != record["git_blob"]:
        raise ValueError(f"Corpus Git blob changed: {path.name}")


def prepare_inputs(corpus, lock, profiles, inventory):
    records = {item["id"]: item for item in inventory["sources"]}
    sources = []
    for source in lock["sources"]:
        record = records[source["id"]]
        tree_raw = (corpus / "inventory" / record["archive"]).read_bytes()
        if digest(tree_raw) != record["sha256"] or record["commit"] != source["commit"]:
            raise ValueError(f"Inventory changed: {source['id']}")
        tree = json.loads(gzip.decompress(tree_raw))
        config = "preview=true\n"
        for mapping in profiles["profiles"][source["id"]]["mappings"]:
            config += f'\n[[kinds]]\npath={json.dumps(mapping["path"], ensure_ascii=False)}\nkind={json.dumps(mapping["kind"])}\n'
        documents = []
        for document in source["documents"]:
            blob = corpus / "data/blobs" / document["git_blob"]
            verify_blob(blob, document)
            documents.append({"path": document["path"], "blob": str(blob), "sha256": document["sha256"]})
        sources.append({"id": source["id"], "config": config, "entries": tree["entries"], "documents": documents})
    return {"sources": sources}


def diagnostic_rows(files, lock):
    sources = {source["id"]: source for source in lock["sources"]}
    blobs = {(source["id"], document["path"]): document for source in lock["sources"] for document in source["documents"]}
    diagnostics = []
    for file in files:
        source = sources[file["source"]]
        for diagnostic in file["result"]["diagnostics"]:
            identity = {"source": file["source"], "path": file["path"], "input_sha256": file["sha256"], "code": diagnostic["code"], "span": diagnostic["byte_range"]}
            related_inputs = []
            for related in diagnostic["related"]:
                record = blobs.get((file["source"], related["filename"]))
                related_inputs.append({"path": related["filename"], "git_blob": record["git_blob"] if record else None,
                                       "sha256": record["sha256"] if record else None, "span": related["byte_range"]})
            diagnostics.append(identity | {"id": digest(encode(identity)), "split": source["split"], "commit": source["commit"],
                "repository": source["repository"], "language": file["language"], "kind": file["result"]["kind"]["value"] or "unknown",
                "git_blob": blobs[(file["source"], file["path"])]["git_blob"], "related_inputs": related_inputs, "diagnostic": diagnostic})
    diagnostics.sort(key=lambda row: (row["source"], row["path"], row["span"]["start"], row["code"]))
    if len({row["id"] for row in diagnostics}) != len(diagnostics):
        raise ValueError("Duplicate diagnostic identities")
    return diagnostics


def counts(diagnostics):
    all_counts = {split: dict(sorted(Counter(row["code"] for row in diagnostics if row["split"] == split).items())) for split in ["tuning", "holdout"]}
    m2_counts = {split: {code: all_counts[split].get(code, 0) for code in M2_RULES} for split in all_counts}
    return all_counts, m2_counts


def run(output, corpus=ROOT / "corpus", split=None, sections=False):
    if output.exists():
        raise ValueError(f"Preserve the existing run before creating another: {output}")
    corpus = corpus.resolve()
    lock_path = corpus / "corpus.lock.json"
    lock = read(lock_path)
    if split is not None:
        if split not in {"tuning", "holdout"}:
            raise ValueError("Unknown evaluation split")
        lock = lock | {"sources": [source for source in lock["sources"] if source["split"] == split]}
    lock_hash = digest(lock_path.read_bytes())
    profile_path = corpus / "evaluation/kinds.json"
    profiles = read(profile_path)
    inventory_path = corpus / "inventory/inventory.lock.json"
    inventory = read(inventory_path)
    if profiles["corpus_sha256"] != lock_hash or inventory["corpus_lock_sha256"] != lock_hash:
        raise ValueError("Kind profile and inventory must describe this corpus lock")
    subprocess.run([sys.executable, str(corpus / "inventory.py"), "verify"], check=True)
    inputs = prepare_inputs(corpus, lock, profiles, inventory)
    if sections:
        inputs["sections"] = True
    work = ROOT / "target" / ("m3-evaluation" if sections else "m2-evaluation")
    work.mkdir(parents=True, exist_ok=True)
    (work / "forward-input.json").write_bytes(encode(inputs))
    (work / "reverse-input.json").write_bytes(encode(reverse_inputs(inputs)))
    before = fingerprints()
    cargo = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo.exe")
    build = subprocess.run([cargo, "build", "--release", "--locked", "--example", "evaluate_m2", "--message-format=json"], cwd=ROOT, capture_output=True, check=True)
    binary = next(Path(event["executable"]) for line in build.stdout.splitlines() if (event := json.loads(line)).get("reason") == "compiler-artifact" and event.get("target", {}).get("name") == "evaluate_m2" and event.get("executable"))
    for order in ["forward", "reverse"]:
        subprocess.run([str(binary), str(work / f"{order}-input.json"), str(work / f"{order}.json")], check=True)
    raw = (work / "forward.json").read_bytes()
    if raw != (work / "reverse.json").read_bytes() or before != fingerprints():
        raise ValueError("Evaluation output or implementation changed during reversed-order verification")
    files = json.loads(raw)
    if len(files) != sum(len(source["documents"]) for source in lock["sources"]):
        raise ValueError("Evaluation omitted documents")
    diagnostics = diagnostic_rows(files, lock)
    all_counts, m2_counts = counts(diagnostics)
    metadata = {"schema_version": 1, "corpus_sha256": lock_hash, "kind_profile_sha256": digest(profile_path.read_bytes()),
        "inventory_sha256": digest(inventory_path.read_bytes()), "implementation": before, "implementation_hash_format": "sha256-lf", "probe_sha256": digest(binary.read_bytes()),
        "link_backend": "case-sensitive pinned Git trees; symlinks and submodules undetermined; anchors only in pinned corpus documents",
        "reverse_byte_identical": True, "reverse_dimensions": ["sources", "documents", "tree_entries"], "raw_result_sha256": digest(raw)}
    if sections:
        metadata.update(evaluation_split=split or "all", section_classifier="deterministic heuristic; annotations are predictions, not review labels")
    packed = gzip.compress(encode(metadata | {"files": files, "diagnostics": diagnostics}), mtime=0)
    output.mkdir(parents=True)
    (output / "diagnostics.json.gz").write_bytes(packed)
    (output / "run.json").write_bytes(encode(metadata | {"report_sha256": digest(packed), "files": len(files), "diagnostics": len(diagnostics),
        "counts": all_counts, "m2_counts": m2_counts, "incomplete_rule_files": dict(sorted(Counter(code for file in files for code in file["incomplete_rules"]).items())),
        "annotation_status": "unlabeled; diagnostic counts are not precision measurements"}))
    print(json.dumps(m2_counts, sort_keys=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--corpus-dir", type=Path, default=ROOT / "corpus", help="Pinned corpus, profiles, inventories, and cached original documents")
    args = parser.parse_args()
    run(args.output, args.corpus_dir)


if __name__ == "__main__":
    main()
