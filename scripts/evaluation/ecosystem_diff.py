"""Compare diagnostic evidence from two runs over the same pinned corpus."""

import argparse
from collections import Counter
import gzip
import hashlib
import html
import json
from pathlib import Path


def read_report(path):
    raw = path.read_bytes()
    report = json.loads(gzip.decompress(raw) if path.suffix == ".gz" else raw)
    return report, hashlib.sha256(raw).hexdigest()


def identity(item):
    return (item["source"], item["path"], item["code"], item["span"]["start"], item["span"]["end"])


def indexed(report):
    result = {}
    for item in report["diagnostics"]:
        key = identity(item)
        if key in result:
            raise ValueError(f"Duplicate diagnostic identity: {key}")
        result[key] = item
    return result


def compare(before, after):
    for field in ["corpus_sha256", "kind_profile_sha256", "inventory_sha256"]:
        if not before.get(field) or before[field] != after.get(field):
            raise ValueError(f"Cannot compare different evaluation inputs: {field}")
    sources = []
    for report in [before, after]:
        hashes = {}
        for item in report["diagnostics"]:
            key = (item["source"], item["path"])
            if key in hashes and hashes[key] != item["input_sha256"]:
                raise ValueError(f"Conflicting source hashes for {key}")
            hashes[key] = item["input_sha256"]
        sources.append(hashes)
    for key in sources[0].keys() & sources[1].keys():
        if sources[0][key] != sources[1][key]:
            raise ValueError(f"Source bytes differ for {key}")
    old, new = indexed(before), indexed(after)
    shared = old.keys() & new.keys()
    for key in shared:
        if old[key]["input_sha256"] != new[key]["input_sha256"]:
            raise ValueError(f"Source bytes differ for {key}")
    added = [new[key] for key in sorted(new.keys() - old.keys())]
    removed = [old[key] for key in sorted(old.keys() - new.keys())]
    changed = [{"before": old[key], "after": new[key]} for key in sorted(shared)
               if old[key]["diagnostic"] != new[key]["diagnostic"]]
    return {"schema_version": 1, "corpus_sha256": after["corpus_sha256"],
            "added": added, "removed": removed, "changed": changed,
            "counts": {"added": dict(sorted(Counter(item["code"] for item in added).items())),
                       "removed": dict(sorted(Counter(item["code"] for item in removed).items())),
                       "changed": dict(sorted(Counter(item["after"]["code"] for item in changed).items()))}}


def literal(value):
    return "<code>" + html.escape(str(value)).replace("\n", " ").replace("\r", " ") + "</code>"


def markdown(report, limit=80):
    lines = ["<!-- seiso-ecosystem -->", "## Corpus diagnostic changes", "",
             f"Added: {len(report['added'])}; removed: {len(report['removed'])}; changed: {len(report['changed'])}.",
             "", "Both runs use the same document, kind-profile, and Git-tree snapshot.", "",
             "| Rule | Added | Removed | Changed |", "| --- | ---: | ---: | ---: |"]
    counts = report["counts"]
    for rule in sorted(set().union(*(value.keys() for value in counts.values()))):
        lines.append(f"| {literal(rule)} | {counts['added'].get(rule, 0)} | {counts['removed'].get(rule, 0)} | {counts['changed'].get(rule, 0)} |")
    items = [("Added", item) for item in report["added"]]
    items += [("Removed", item) for item in report["removed"]]
    items += [("Changed", item["after"]) for item in report["changed"]]
    if items:
        lines += ["", "### Diagnostics", ""]
        for action, item in items[:limit]:
            diagnostic = item["diagnostic"]
            location = diagnostic["location"]
            label = f"{item['source']}/{item['path']}:{location['row']}:{location['column']} {item['code']}"
            message = diagnostic["message"]
            if len(message) > 500:
                message = message[:497] + "..."
            lines.append(f"- {action} {literal(label)}: {literal(message)}")
    if len(items) > limit:
        lines += ["", f"Showing {limit} of {len(items)} changes. The workflow artifact contains every diagnosis and related location."]
    lines += ["", "Review changed diagnostics before accepting a rule change. Counts do not establish precision.", ""]
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    before, before_sha = read_report(args.before)
    after, after_sha = read_report(args.after)
    result = compare(before, after)
    result.update(before_report_sha256=before_sha, after_report_sha256=after_sha)
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "changes.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    (args.output / "changes.md").write_text(markdown(result), encoding="utf-8")
    print(json.dumps(result["counts"], sort_keys=True))


if __name__ == "__main__":
    main()
