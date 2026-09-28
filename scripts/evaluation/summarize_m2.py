"""Validate bound M2 annotations and summarize precision without promoting rules."""

import argparse
from collections import Counter
import gzip
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RULES = ("PTR002", "LNK002", "DUP001", "DUP002", "DUP003", "OWN001", "OWN002")
LABELS = {"tp", "fp", "uncertain"}
LANGUAGES = ("en", "ja", "zh")
KINDS = ("readme", "howto", "reference", "runbook", "adr", "plan", "changelog", "generated", "unknown")


def encode(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode()


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def read(path):
    return json.loads(path.read_bytes())


def validate_labels(bundles, rows, report_sha256, complete=True):
    decisions = {}
    provenance = {}
    for name, bundle in bundles:
        if bundle.get("schema_version") != 1 or bundle.get("report_sha256") != report_sha256:
            raise ValueError(f"Annotation report binding mismatch: {name}")
        if not bundle.get("reviewer_kind"):
            raise ValueError(f"Annotation reviewer is missing: {name}")
        for label in bundle["labels"]:
            identity = label["id"]
            if identity in decisions:
                raise ValueError(f"Duplicate annotation: {identity}")
            if identity not in rows:
                raise ValueError(f"Unknown M2 diagnostic annotation: {identity}")
            row = rows[identity]
            if label.get("code") != row["code"] or label.get("input_sha256") != row["input_sha256"]:
                raise ValueError(f"Annotation input binding mismatch: {identity}")
            if label.get("diagnostic_sha256") != digest(encode(row["diagnostic"])):
                raise ValueError(f"Annotation diagnostic binding mismatch: {identity}")
            if label.get("label") not in LABELS or not label.get("reason", "").strip():
                raise ValueError(f"Invalid annotation decision or missing reason: {identity}")
            decisions[identity] = label
            provenance[identity] = {"file": name, "reviewer_kind": bundle["reviewer_kind"]}
    if complete and decisions.keys() != rows.keys():
        raise ValueError(f"Missing {len(rows.keys() - decisions.keys())} M2 annotations")
    return decisions, provenance


def precision(rows, decisions):
    counts = Counter(decisions[row["id"]]["label"] for row in rows)
    known = counts["tp"] + counts["fp"]
    total = known + counts["uncertain"]
    return {"diagnostics": total, "tp": counts["tp"], "fp": counts["fp"], "uncertain": counts["uncertain"],
            "precision": counts["tp"] / known if known else None,
            "conservative_precision": counts["tp"] / total if total else None}


def dimensions(rows, decisions):
    return precision(rows, decisions) | {
        "by_language": {language: precision([row for row in rows if row["language"] == language], decisions)
                        for language in sorted(set(LANGUAGES) | {row["language"] for row in rows})},
        "by_kind": {kind: precision([row for row in rows if row["kind"] == kind], decisions)
                    for kind in sorted(set(KINDS) | {row["kind"] for row in rows})},
    }


def summarize(rows, decisions, provenance):
    result = {}
    for code in RULES:
        selected = [row for row in rows.values() if row["code"] == code]
        split_results = {split: dimensions([row for row in selected if row["split"] == split], decisions)
                         for split in ["tuning", "holdout"]}
        holdout = split_results["holdout"]
        result[code] = {"status": "preview", "promotion": False,
                       "natural_sample_status": "observed" if selected else "no_natural_samples",
                       "precision_validated": False,
                       "holdout_numeric_threshold_met": holdout["diagnostics"] >= 100 and holdout["conservative_precision"] is not None and holdout["conservative_precision"] >= 0.95,
                       "annotation_provenance": [{"file": file, "reviewer_kind": reviewer} for file, reviewer in sorted({(provenance[row["id"]]["file"], provenance[row["id"]]["reviewer_kind"]) for row in selected})],
                       "splits": split_results}
    return result


def verify_sources(rows, corpus, corpus_sha256):
    lock_path = corpus / "corpus.lock.json"
    if digest(lock_path.read_bytes()) != corpus_sha256:
        raise ValueError("Corpus lock differs from the diagnostic report")
    lock = read(lock_path)
    records = {(source["id"], document["path"]): document for source in lock["sources"] for document in source["documents"]}
    verified = set()

    def verify(source, path, expected_hash, blob):
        record = records.get((source, path))
        if record is None or record["sha256"] != expected_hash or record["git_blob"] != blob:
            raise ValueError(f"Diagnostic input is not bound to the corpus: {source}/{path}")
        if blob not in verified:
            raw = (corpus / "data/blobs" / blob).read_bytes()
            if digest(raw) != expected_hash or len(raw) != record["bytes"]:
                raise ValueError(f"Original source hash mismatch: {source}/{path}")
            verified.add(blob)

    for row in rows.values():
        verify(row["source"], row["path"], row["input_sha256"], row["git_blob"])
        for related in row["related_inputs"]:
            if related["git_blob"] is not None:
                verify(row["source"], related["path"], related["sha256"], related["git_blob"])


def review_metadata(path, rows, report_hash):
    review = read(path)
    decisions, _ = validate_labels([(path.name, review)], rows, report_hash, complete=False)
    return {"file": path.name, "sha256": digest(path.read_bytes()), "reviewer_kind": review["reviewer_kind"],
            "reviewed_diagnostics": len(decisions), "method": review.get("method"),
            "counts": dict(sorted(Counter(label["label"] for label in decisions.values()).items()))}, decisions


def run(report_path, label_paths, output, corpus, author_review=None, independent_review=None):
    if output.exists():
        raise ValueError(f"Preserve the existing summary before creating another: {output}")
    report_raw = report_path.read_bytes()
    report_hash = digest(report_raw)
    report = json.loads(gzip.decompress(report_raw))
    selected = [row for row in report["diagnostics"] if row["code"] in RULES]
    rows = {row["id"]: row for row in selected}
    if len(rows) != len(selected):
        raise ValueError("Duplicate M2 diagnostic identities in the report")
    verify_sources(rows, corpus, report["corpus_sha256"])
    bundles = [(path.name, read(path)) for path in label_paths]
    decisions, provenance = validate_labels(bundles, rows, report_hash)
    reviews = {}
    for kind, path in [("author_review", author_review), ("independent_review", independent_review)]:
        if path is not None:
            metadata, reviewed = review_metadata(path, rows, report_hash)
            metadata["disagreements_with_primary_labels"] = sorted(identity for identity, label in reviewed.items()
                                                                  if label["label"] != decisions[identity]["label"])
            reviews[kind] = metadata
    result = {"schema_version": 1, "report_sha256": report_hash, "corpus_sha256": report["corpus_sha256"],
              "summary_script_sha256": digest(Path(__file__).read_bytes()),
              "annotation_files": {path.name: digest(path.read_bytes()) for path in label_paths},
              "diagnostics_labeled": len(decisions), "rules": summarize(rows, decisions, provenance), "reviews": reviews,
              "gate": {"dup_own_reports_complete": all(code in RULES for code in ["DUP001", "DUP002", "DUP003", "OWN001", "OWN002"]),
                       "zero_samples_are_precision_validation": False, "rules_promoted": [],
                       "recall": "not measured; this report labels emitted diagnostics only"},
              "precision_definition": "tp / (tp + fp); null when there are no determined labels",
              "conservative_precision_definition": "tp / (tp + fp + uncertain); null when there are no diagnostics"}
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(encode(result))
    print(json.dumps({code: value["splits"]["holdout"] for code, value in result["rules"].items()}, sort_keys=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--labels", type=Path, action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--corpus-dir", type=Path, default=ROOT / "corpus")
    parser.add_argument("--author-review", type=Path)
    parser.add_argument("--independent-review", type=Path)
    args = parser.parse_args()
    run(args.report, args.labels, args.output, args.corpus_dir, args.author_review, args.independent_review)


if __name__ == "__main__":
    main()
