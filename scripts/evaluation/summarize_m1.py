"""Validate bound annotations and report the M1 acceptance gates by rule."""

import argparse
from collections import Counter, defaultdict
import gzip
import hashlib
import json
from pathlib import Path
import subprocess

RULES = ["KND001", "KND002", "STL001", "STL003", "PTR001", "PTR003", "LNK001", "RAT002", "VOX001", "SUP001", "SUP002"]
CONSISTENCY = {"KND001", "KND002", "LNK001", "SUP001", "SUP002"}
PROTOCOL = {"KND002", "SUP001", "SUP002"}


def sha(content):
    return hashlib.sha256(content).hexdigest()


def stats(labels):
    counts = Counter(label["label"] for label in labels)
    judged = counts["tp"] + counts["fp"]
    total = len(labels)
    return {"samples": total, "tp": counts["tp"], "fp": counts["fp"], "uncertain": counts["uncertain"],
        "precision": counts["tp"] / judged if judged else None,
        "conservative_precision": counts["tp"] / total if total else None}


def summarize(report, report_hash, annotation_files, protocol_accepted=False):
    diagnoses = {item["id"]: item for item in report["diagnostics"]}
    if len(diagnoses) != len(report["diagnostics"]):
        raise ValueError("Duplicate diagnostic IDs")
    labels = {}
    manually_reviewed = set()
    for annotations in annotation_files:
        if annotations.get("report_sha256") != report_hash:
            raise ValueError("Annotations belong to a different frozen report")
        file_ids = set()
        for label in annotations["labels"]:
            identifier = label["id"]
            if identifier not in diagnoses or identifier in labels:
                raise ValueError("Unknown or duplicate annotation ID")
            if label["label"] not in {"tp", "fp", "uncertain"} or not label.get("evidence", "").strip():
                raise ValueError("Each label needs a verdict and evidence")
            labels[identifier] = label
            file_ids.add(identifier)
        reviewed = annotations.get("manually_reviewed_sample_ids", [])
        if len(set(reviewed)) != len(reviewed) or not set(reviewed) <= file_ids:
            raise ValueError("Manual review IDs must uniquely reference this annotation file")
        manually_reviewed.update(reviewed)
    if set(labels) != set(diagnoses):
        raise ValueError(f"Missing annotations for {len(set(diagnoses) - set(labels))} diagnoses")
    results = {}
    for code in RULES:
        rule = {}
        for split in ["tuning", "holdout"]:
            ids = [identifier for identifier, item in diagnoses.items() if item["code"] == code and item["split"] == split]
            groups = defaultdict(list)
            for identifier in ids:
                item = diagnoses[identifier]
                groups[f'{item["language"]}/{item["kind"]}'].append(labels[identifier])
            rule[split] = stats([labels[identifier] for identifier in ids]) | {
                "individually_reviewed_by_agent": sum(identifier in manually_reviewed for identifier in ids),
                "by_language_and_kind": {group: stats(values) for group, values in sorted(groups.items())}}
        holdout = rule["holdout"]
        natural_pass = holdout["samples"] >= 100 and (holdout["conservative_precision"] or 0) >= .95 and holdout["individually_reviewed_by_agent"] >= 100
        protocol_pass = code in PROTOCOL and protocol_accepted and rule["holdout"]["samples"] == 0 and rule["tuning"]["samples"] == 0
        rule["acceptance_route"] = "protocol_conformance" if protocol_pass else "natural_holdout"
        rule["eligible_for_stable"] = natural_pass or protocol_pass
        rule["decision"] = "stable" if rule["eligible_for_stable"] else "preview"
        results[code] = rule
    return {"schema_version": 1, "report_sha256": report_hash, "protocol_exception_accepted": protocol_accepted,
        "annotation_method": "Agent review with independent consistency oracles; no human labels claimed.",
        "effective_kind_coverage": dict(sorted(Counter(file["result"]["kind"]["value"] or "unknown" for file in report.get("files", [])).items())),
        "diagnostics_annotated": len(labels), "rules": results,
        "m1_exit_passed": all(results[code]["eligible_for_stable"] for code in CONSISTENCY)}


def verify_protocol(path, root, source_ref=None):
    protocol = json.loads(path.read_bytes())
    audit = path.parent / "syntax-audit.json"
    if protocol["syntax_audit_sha256"] != sha(audit.read_bytes()):
        raise ValueError("Protocol audit differs from its acceptance receipt")
    recorded_ref = protocol.get("source_revision")
    if not (source_ref or recorded_ref):
        raise ValueError("Protocol receipt has no source_revision; pass --protocol-source-ref with its accepted Git revision")

    def git(*args):
        result = subprocess.run(["git", *args], cwd=root, capture_output=True, check=False)
        if result.returncode:
            raise ValueError(f"Cannot read accepted protocol sources: {result.stderr.decode('utf-8', errors='replace').strip()}")
        return result.stdout

    revision = git("rev-parse", "--verify", "--end-of-options", f"{source_ref or recorded_ref}^{{commit}}").decode().strip()
    if recorded_ref and source_ref:
        recorded = git("rev-parse", "--verify", "--end-of-options", f"{recorded_ref}^{{commit}}").decode().strip()
        if revision != recorded:
            raise ValueError("--protocol-source-ref differs from the receipt's source_revision")
    for name, expected in protocol["test_sources"].items():
        if sha(git("show", f"{revision}:{name}")) != expected:
            raise ValueError(f"Contract test differs from the acceptance receipt at {revision}: {name}")
    accepted = (protocol["status"] == "accepted" and protocol["owner_approved"]
                and protocol["conformance_passed"] and set(protocol["approved_rules"]) == PROTOCOL)
    return accepted, revision


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    parser.add_argument("--protocol", type=Path)
    parser.add_argument("--protocol-source-ref", help="Git revision of accepted protocol test sources for historical replay")
    parser.add_argument("--output", type=Path, help="Summary destination (defaults to RUN/summary.json)")
    args = parser.parse_args()
    if args.protocol_source_ref and not args.protocol:
        parser.error("--protocol-source-ref requires --protocol")
    packed = (args.run / "diagnostics.json.gz").read_bytes()
    report_hash = sha(packed)
    receipt = json.loads((args.run / "run.json").read_bytes())
    if receipt["report_sha256"] != report_hash:
        raise ValueError("Report differs from its frozen receipt")
    accepted = False
    source_revision = None
    if args.protocol:
        root = Path(__file__).resolve().parents[2]
        accepted, source_revision = verify_protocol(args.protocol, root, args.protocol_source_ref)
    paths = sorted(args.run.glob("labels-*.json"))
    summary = summarize(json.loads(gzip.decompress(packed)), report_hash, [json.loads(path.read_bytes()) for path in paths], accepted)
    summary["annotations"] = {path.name: sha(path.read_bytes()) for path in paths}
    summary["protocol_receipt_sha256"] = sha(args.protocol.read_bytes()) if args.protocol else None
    summary["protocol_source_revision"] = source_revision
    output = args.output or args.run / "summary.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps({"m1_exit_passed": summary["m1_exit_passed"], "rules": {code: rule["decision"] for code, rule in summary["rules"].items()}}, sort_keys=True))


if __name__ == "__main__":
    main()
