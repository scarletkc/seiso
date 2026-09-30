"""Pin, fetch, evaluate, and summarize independent LNK002 holdout cohorts."""

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import gzip
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

from corpus import corpus as corpus_tools
from corpus import inventory as inventory_tools
from .evaluate_m2 import (
    ROOT, diagnostic_rows, digest, encode, fingerprints, prepare_inputs, read,
    reverse_inputs, source_digest, verify_blob,
)
from .summarize_m2 import KINDS, LABELS, validate_labels

RULES = ("LNK001", "LNK002")
HEX40 = re.compile(r"[a-f0-9]{40}")
HEX64 = re.compile(r"[a-f0-9]{64}")


def preserve(path):
    if path.exists():
        raise ValueError(f"Preserve the existing evidence before creating another: {path}")


def write_new(path, content):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("xb") as stream:
        stream.write(content)


def validate_spec(spec, batch):
    if spec.get("schema_version") != 1 or not isinstance(spec.get("sources"), list) or not spec["sources"]:
        raise ValueError("A selection specification needs schema_version 1 and sources")
    if not HEX40.fullmatch(spec.get("freeze_commit", "")):
        raise ValueError("The specification must record a full freeze_commit")
    ids, repositories = set(), set()
    for source in spec["sources"]:
        if not re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", source.get("id", "")) or source["id"] in ids:
            raise ValueError("Specification source IDs must be unique safe filenames")
        repository = source.get("repository", "")
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
            raise ValueError("Invalid GitHub repository in specification")
        if repository.casefold() in repositories:
            raise ValueError("Repositories must be unique across all batches")
        if type(source.get("batch")) is not int or source["batch"] < 1:
            raise ValueError("Each source needs a positive integer batch")
        patterns = source.get("include")
        if not isinstance(patterns, list) or not patterns or len(set(patterns)) != len(patterns):
            raise ValueError("Each source needs distinct include patterns")
        for pattern in patterns + source.get("license_paths", []):
            inventory_tools.valid_path(pattern)
            corpus_tools.valid_path(pattern)
        if any(key in source for key in ("commit", "tree", "documents", "licenses", "inventory")):
            raise ValueError("Selection specifications contain scopes, not resolved source content")
        if source.get("split", "holdout") != "holdout":
            raise ValueError("Fresh link cohorts must be holdout")
        ids.add(source["id"])
        repositories.add(repository.casefold())
    selected = [source for source in spec["sources"] if source["batch"] == batch]
    if type(batch) is not int or not selected:
        raise ValueError("Requested batch is absent from the specification")
    return selected


def prior_repositories():
    paths = {ROOT / "corpus/corpus.lock.json"}
    for name in ("corpus.lock.json", "selection.json"):
        paths.update((ROOT / "corpus/results").rglob(name))
    known, receipts = set(), {}
    for path in sorted(paths):
        raw = path.read_bytes()
        previous = json.loads(raw)
        for source in previous.get("sources", []) + previous.get("skipped", []):
            known.add(source["repository"].casefold())
        receipts[path.relative_to(ROOT).as_posix()] = digest(raw)
    return known, receipts


def pin(spec_path, batch, output):
    preserve(output)
    spec_raw = spec_path.read_bytes()
    spec = json.loads(spec_raw)
    selected = validate_spec(spec, batch)
    known, receipts = prior_repositories()
    known.update(source["repository"].casefold() for source in spec["sources"] if source["batch"] < batch)
    if any(source["repository"].casefold() in known for source in selected):
        raise ValueError("Fresh holdout repositories overlap an earlier corpus or batch")
    sources, skipped = [], []
    for source in selected:
        request = source | {"split": "holdout", "language_focus": "en", "category": "developer-documentation"}
        try:
            resolved, response = corpus_tools.resolve_source(request, with_tree=True)
            tree = inventory_tools.normalize_tree(response, resolved)
            if len({entry["path"] for entry in tree["entries"]}) != len(tree["entries"]):
                raise ValueError("Duplicate Git tree paths")
        except ValueError as error:
            skipped.append(source | {"reason": str(error)})
            continue
        if resolved["repository"].casefold() in known:
            raise ValueError("Resolved repository overlaps an earlier corpus or batch")
        known.add(resolved["repository"].casefold())
        sources.append(resolved | {"inventory": tree})
    if spec_path.read_bytes() != spec_raw:
        raise ValueError("Specification changed during pinning")
    selection = {"schema_version": 1, "batch": batch, "freeze_commit": spec["freeze_commit"],
                 "spec_sha256": digest(spec_raw), "previous_inputs": receipts,
                 "selection": "All regular Markdown matching precommitted scopes; mechanical failures are skipped without replacement.",
                 "sources": sources, "skipped": skipped}
    write_new(output, encode(selection))
    print(json.dumps({"documents": {source["id"]: len(source["documents"]) for source in sources}, "skipped": skipped}))


def validate_cohort(lock, *, fetched=True):
    if lock.get("schema_version") != 1 or not isinstance(lock.get("sources"), list):
        raise ValueError("Unsupported cohort lock")
    if lock["sources"]:
        corpus_tools.validate_lock(lock)
    elif not lock.get("skipped"):
        raise ValueError("An empty cohort must retain its mechanical skip receipts")
    if type(lock.get("batch")) is not int or lock["batch"] < 1 or not HEX40.fullmatch(lock.get("freeze_commit", "")):
        raise ValueError("Cohort must bind a batch and freeze commit")
    if not HEX64.fullmatch(lock.get("spec_sha256", "")):
        raise ValueError("Cohort must bind its selection specification")
    for source in lock.get("skipped", []):
        if source.get("batch") != lock["batch"] or not source.get("reason", "").strip():
            raise ValueError("Each skipped source needs its batch and mechanical failure reason")
    for source in lock["sources"]:
        if source.get("split") != "holdout" or source.get("batch") != lock["batch"]:
            raise ValueError("Cohort source is outside the selected holdout batch")
        if not HEX40.fullmatch(source.get("tree", "")) or not source.get("licenses"):
            raise ValueError("Every source needs a pinned tree and license")
        for record in source["documents"] + source["licenses"]:
            inventory_tools.valid_path(record["path"])
            if type(record.get("bytes")) is not int or record["bytes"] < 0:
                raise ValueError("Invalid locked blob size")
            if fetched and not HEX64.fullmatch(record.get("sha256", "")):
                raise ValueError("Every fetched blob needs a SHA-256")
    if fetched and not HEX64.fullmatch(lock.get("pinned_selection_sha256", "")):
        raise ValueError("Fetched cohort must bind its pinned selection")


def fetch(selection_path, output):
    preserve(output)
    inventory_dir = output.parent / "inventory"
    preserve(inventory_dir)
    selection_raw = selection_path.read_bytes()
    selection = json.loads(selection_raw)
    validate_cohort(selection, fetched=False)
    archives = {}
    for source in selection["sources"]:
        packed = inventory_tools.archive_bytes(source["inventory"])
        inventory_tools.validate_archive(packed, source)
        archives[source["id"]] = packed
    lock = {key: value for key, value in selection.items() if key != "sources"}
    lock["sources"] = [{key: value for key, value in source.items() if key != "inventory"} for source in selection["sources"]]
    jobs = [(source, entry) for source in lock["sources"] for entry in source["documents"] + source["licenses"]]
    with ThreadPoolExecutor(max_workers=8) as pool:
        hashes = list(pool.map(lambda pair: corpus_tools.fetch_blob(*pair), jobs))
    for (_, entry), sha in zip(jobs, hashes, strict=True):
        entry["sha256"] = sha
        verify_blob(ROOT / "corpus/data/blobs" / entry["git_blob"], entry)
    lock["pinned_selection_sha256"] = digest(selection_raw)
    validate_cohort(lock)
    if selection_path.read_bytes() != selection_raw:
        raise ValueError("Pinned selection changed during fetch")
    lock_raw = encode(lock)
    records = [{**inventory_tools.identity(source), "archive": source["id"] + ".json.gz",
                "sha256": digest(archives[source["id"]]), "entries": len(original["inventory"]["entries"])}
               for source, original in zip(lock["sources"], selection["sources"], strict=True)]
    inventory_dir.mkdir(parents=True)
    for record in records:
        write_new(inventory_dir / record["archive"], archives[record["id"]])
    write_new(inventory_dir / "inventory.lock.json", encode({"schema_version": 1, "corpus_lock_sha256": digest(lock_raw), "sources": records}))
    write_new(output, lock_raw)
    print(f"Fetched and verified {len(jobs)} pinned documents and license records.")


def validate_inventory(lock, lock_hash, inventory_path, inventory):
    if inventory.get("schema_version") != 1 or inventory.get("corpus_lock_sha256") != lock_hash:
        raise ValueError("Inventory must describe this corpus lock")
    sources = {source["id"]: source for source in lock["sources"]}
    records = inventory.get("sources", [])
    if len(records) != len(sources) or {record["id"] for record in records} != sources.keys():
        raise ValueError("Inventory sources differ from corpus lock")
    trees, hashes = {}, {}
    for record in records:
        source = sources[record["id"]]
        if any(record.get(key) != value for key, value in inventory_tools.identity(source).items()):
            raise ValueError("Inventory record identity changed")
        if record.get("archive") != source["id"] + ".json.gz":
            raise ValueError("Invalid inventory archive path")
        path = inventory_path.parent / record["archive"]
        packed = path.read_bytes()
        payload = inventory_tools.check_record(packed, source, record)
        if len({entry["path"] for entry in payload["entries"]}) != len(payload["entries"]):
            raise ValueError("Duplicate inventory paths")
        trees[source["id"]] = {entry["path"]: entry for entry in payload["entries"]}
        hashes[path] = digest(packed)
    return trees, hashes


def kind_mappings(lock, lock_hash, profile):
    if profile.get("schema_version") != 1 or profile.get("corpus_sha256") != lock_hash:
        raise ValueError("Kind profile must describe this corpus lock")
    expected = {f"{source['id']}/{item['path']}": item for source in lock["sources"] for item in source["documents"]}
    decisions = profile.get("documents", {})
    if decisions.keys() != expected.keys():
        raise ValueError("Kind profile has missing or extra documents")
    for key, document in expected.items():
        decision = decisions[key]
        if decision.get("input_sha256") != document["sha256"] or not decision.get("reason", "").strip():
            raise ValueError(f"Missing source-bound kind review: {key}")
        if decision.get("kind") not in KINDS:
            raise ValueError(f"Invalid reviewed kind: {key}")
    return {"profiles": {source["id"]: {"mappings": [
        {"path": item["path"], "kind": decisions[f"{source['id']}/{item['path']}"]["kind"]}
        for item in source["documents"] if decisions[f"{source['id']}/{item['path']}"]["kind"] != "unknown"]}
        for source in lock["sources"]}}


def validate_sites(lock, lock_hash, profile, trees):
    if profile.get("schema_version") != 1 or profile.get("corpus_sha256") != lock_hash:
        raise ValueError("Site profile must describe this corpus lock")
    profiles = profile.get("profiles", {})
    if profiles.keys() != {source["id"] for source in lock["sources"]}:
        raise ValueError("Every source needs an explicit site review")
    for source_id, review in profiles.items():
        if not isinstance(review.get("sites"), list) or not review.get("reason", "").strip():
            raise ValueError(f"Missing site review reason: {source_id}")
        for site in review["sites"]:
            for key in ("path", "root", "generator", "evidence", "evidence_path", "evidence_git_blob"):
                if not isinstance(site.get(key), str) or not site[key].strip():
                    raise ValueError(f"Missing site configuration evidence: {source_id}/{key}")
            entry = trees[source_id].get(site["evidence_path"])
            if entry is None or entry["type"] != "blob" or entry["mode"] not in {"100644", "100755"} or entry["sha"] != site["evidence_git_blob"]:
                raise ValueError(f"Site evidence is not a pinned configuration blob: {source_id}")
            for key in ("path", "root", "public"):
                if key in site and site[key] != ".":
                    inventory_tools.valid_path(site[key])
                    corpus_tools.valid_path(site[key])
            if "base" in site and not isinstance(site["base"], str):
                raise ValueError("Site base must be a string")


def implementation_fingerprints():
    result = fingerprints()
    for relative in ("scripts/evaluation/fresh_links.py", "corpus/corpus.py", "corpus/inventory.py"):
        result[relative] = source_digest(ROOT / relative)
    return dict(sorted(result.items()))


def build_probe():
    cargo = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo.exe")
    result = subprocess.run([cargo, "build", "--release", "--locked", "--example", "evaluate_m2", "--message-format=json"],
                            cwd=ROOT, capture_output=True, check=True)
    return next(Path(event["executable"]) for line in result.stdout.splitlines()
                if (event := json.loads(line)).get("reason") == "compiler-artifact"
                and event.get("target", {}).get("name") == "evaluate_m2" and event.get("executable"))


def run_probe(binary, inputs, directory, order):
    source, output = directory / f"{order}-input.json", directory / f"{order}.json"
    source.write_bytes(encode(inputs))
    subprocess.run([str(binary), str(source), str(output)], check=True)
    return output.read_bytes()


def validate_files(files, lock):
    expected = {(source["id"], item["path"]): item["sha256"] for source in lock["sources"] for item in source["documents"]}
    actual = {(file["source"], file["path"]): file["sha256"] for file in files}
    if len(actual) != len(files) or actual != expected:
        raise ValueError("Evaluation omitted, duplicated, or changed documents")
    for file in files:
        if not isinstance(file.get("links"), list) or not isinstance(file.get("incomplete_rules"), list):
            raise ValueError("Evaluation omitted link or completeness metadata")
        if file["result"].get("errors") or any(row["code"] not in RULES for row in file["result"]["diagnostics"]):
            raise ValueError("Evaluation contains errors or unselected rules")


def evaluate(lock_path, profile_path, site_path, output):
    preserve(output)
    inventory_path = lock_path.parent / "inventory/inventory.lock.json"
    paths = {"corpus_sha256": lock_path, "kind_profile_sha256": profile_path,
             "site_profile_sha256": site_path, "inventory_sha256": inventory_path}
    contents = {key: path.read_bytes() for key, path in paths.items()}
    hashes = {key: digest(raw) for key, raw in contents.items()}
    lock, profile, sites, inventory = (json.loads(contents[key]) for key in paths)
    validate_cohort(lock)
    trees, archive_hashes = validate_inventory(lock, hashes["corpus_sha256"], inventory_path, inventory)
    profiles = kind_mappings(lock, hashes["corpus_sha256"], profile)
    validate_sites(lock, hashes["corpus_sha256"], sites, trees)
    before = implementation_fingerprints()
    inputs = prepare_inputs(ROOT / "corpus", lock, profiles, inventory, sites,
                            inventory_dir=inventory_path.parent, rules=RULES)
    binary = build_probe()
    probe_hash = digest(binary.read_bytes())
    target = ROOT / "target"
    target.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="fresh-links-", dir=target) as temporary:
        work = Path(temporary)
        raw = run_probe(binary, inputs, work, "forward")
        reverse = run_probe(binary, reverse_inputs(inputs), work, "reverse")
    if raw != reverse:
        raise ValueError("Evaluation output changed during reversed-order verification")
    if before != implementation_fingerprints() or probe_hash != digest(binary.read_bytes()):
        raise ValueError("Implementation changed during evaluation")
    if any(path.read_bytes() != contents[key] for key, path in paths.items()) or any(digest(path.read_bytes()) != sha for path, sha in archive_hashes.items()):
        raise ValueError("Evaluation input bindings changed during the run")
    files = json.loads(raw)
    validate_files(files, lock)
    diagnostics = diagnostic_rows(files, lock)
    metadata = {"schema_version": 1, "evaluation": "fresh_links", "batch": lock["batch"],
                "freeze_commit": lock["freeze_commit"], "spec_sha256": lock["spec_sha256"],
                "pinned_selection_sha256": lock["pinned_selection_sha256"], **hashes,
                "implementation": before, "implementation_hash_format": "sha256-lf",
                "script_sha256": digest(Path(__file__).read_bytes()), "probe_sha256": probe_hash,
                "probe_source_sha256": source_digest(ROOT / "examples/evaluate_m2.rs"),
                "rules": list(RULES), "evaluation_split": "holdout", "reverse_byte_identical": True,
                "reverse_dimensions": ["sources", "documents", "tree_entries"], "raw_result_sha256": digest(raw),
                "link_backend": "case-sensitive pinned Git trees with reviewed sites; anchors only in pinned selected documents"}
    packed = gzip.compress(encode(metadata | {"files": files, "diagnostics": diagnostics}), mtime=0)
    counts = {code: sum(row["code"] == code for row in diagnostics) for code in RULES}
    receipt = metadata | {"report_sha256": digest(packed), "files": len(files), "diagnostics": len(diagnostics),
                          "counts": counts, "incomplete_rule_files": dict(sorted(Counter(code for file in files for code in file["incomplete_rules"]).items())),
                          "annotation_status": "unlabeled; diagnostic counts are not precision measurements"}
    output.mkdir(parents=True)
    write_new(output / "diagnostics.json.gz", packed)
    write_new(output / "run.json", encode(receipt))
    print(json.dumps(counts, sort_keys=True))


def validate_dual_review(label):
    reviews = [label.get(key, {}) for key in ("author_review", "independent_review")]
    for review in reviews:
        if not review.get("reviewer", "").strip() or review.get("label") not in LABELS or review.get("agent_context_reviewed") is not True:
            raise ValueError("Every diagnosis requires two individual agent reviews")
        if any(not review.get(key, "").strip() for key in ("reason", "renderer", "evidence")):
            raise ValueError("Each independent label needs reason, renderer, and evidence")
    if reviews[0]["reviewer"] == reviews[1]["reviewer"]:
        raise ValueError("The implementing and reviewing agents must be distinct")
    agrees = reviews[0]["label"] == reviews[1]["label"]
    if (not agrees or label["label"] != reviews[0]["label"]) and not label.get("disagreement_reason", "").strip():
        raise ValueError("Label disagreements require a resolution reason")
    if any(not label.get(key, "").strip() for key in ("renderer", "evidence")) or label.get("agent_context_reviewed") is not True:
        raise ValueError("Resolved labels need individually reviewed renderer evidence")
    return agrees


def score(rows, decisions):
    counts = Counter(decisions[row["id"]]["label"] for row in rows)
    total = len(rows)
    known = counts["tp"] + counts["fp"]
    agreements = sum(decisions[row["id"]]["author_review"]["label"] == decisions[row["id"]]["independent_review"]["label"] for row in rows)
    return {"samples": total, "tp": counts["tp"], "fp": counts["fp"], "uncertain": counts["uncertain"],
            "precision": counts["tp"] / known if known else None,
            "conservative_precision": counts["tp"] / total if total else None,
            "agreements": agreements, "agreement_rate": agreements / total if total else None}


def summarize(manifest_path, output):
    preserve(output)
    manifest_raw = manifest_path.read_bytes()
    manifest = json.loads(manifest_raw)
    batches = manifest.get("batches", [])
    if manifest.get("schema_version") != 1 or not batches or [batch.get("batch") for batch in batches] != list(range(1, len(batches) + 1)):
        raise ValueError("Manifest must list distinct consecutive batches starting at 1")
    inputs, rows, decisions, by_batch, by_source, repositories = {}, [], {}, {}, {}, set()
    review_provenance = {}
    protocol = None

    def consume(name):
        path = manifest_path.parent / name
        raw = path.read_bytes()
        inputs[name] = digest(raw)
        return raw

    for batch in batches:
        packed = consume(batch["report"])
        report = json.loads(gzip.decompress(packed))
        lock_raw = consume(batch["corpus_lock"])
        lock = json.loads(lock_raw)
        validate_cohort(lock)
        if report.get("evaluation") != "fresh_links" or report.get("corpus_sha256") != digest(lock_raw) or report.get("batch") != batch["batch"] or lock["batch"] != batch["batch"]:
            raise ValueError("Report is not bound to this fresh holdout batch")
        if report.get("evaluation_split") != "holdout" or report.get("reverse_byte_identical") is not True or report.get("rules") != list(RULES):
            raise ValueError("Report did not verify the fresh link evaluation protocol")
        current_protocol = {key: report[key] for key in ("freeze_commit", "spec_sha256", "implementation", "kind_profile_sha256", "site_profile_sha256", "inventory_sha256")}
        for key in ("freeze_commit", "spec_sha256", "pinned_selection_sha256"):
            if report.get(key) != lock.get(key):
                raise ValueError(f"Report selection binding changed: {key}")
        common = {key: current_protocol[key] for key in ("freeze_commit", "spec_sha256", "implementation")}
        if protocol is not None and common != protocol:
            raise ValueError("Batches must share the frozen specification and implementation")
        protocol = common
        validate_files(report["files"], lock)
        if diagnostic_rows(report["files"], lock) != report["diagnostics"]:
            raise ValueError("Report diagnostic identities differ from its file results")
        selected = [row for row in report["diagnostics"] if row["code"] == "LNK002"]
        expected = {row["id"]: row for row in selected}
        bundle = json.loads(consume(batch["labels"]))
        if bundle.get("scope_codes") != ["LNK002"] or any(bundle.get(key) != report[key] for key in ("corpus_sha256", "inventory_sha256")):
            raise ValueError("Label scope or corpus/inventory binding changed")
        if not HEX64.fullmatch(bundle.get("decision_receipt_sha256", "")):
            raise ValueError("Labels must be bound with review_m2_links.py --bind-report")
        reviewed, _ = validate_labels([(batch["labels"], bundle)], expected, digest(packed))
        if type(bundle.get("human_reviewers")) is not int or bundle["human_reviewers"] < 0:
            raise ValueError("Label bundles must state the number of human reviewers")
        review_provenance[str(batch["batch"])] = {"reviewer_kind": bundle["reviewer_kind"], "human_reviewers": bundle["human_reviewers"]}
        for identity, label in reviewed.items():
            for key in ("source", "path", "span", "related_inputs", "split"):
                if label.get(key) != expected[identity][key]:
                    raise ValueError(f"Label diagnostic identity changed: {identity}/{key}")
            validate_dual_review(label)
        if decisions.keys() & reviewed.keys():
            raise ValueError("Duplicate diagnoses across batches")
        decisions.update(reviewed)
        rows.extend(selected)
        by_batch[str(batch["batch"])] = score(selected, reviewed) | {"skipped": lock.get("skipped", [])}
        for source in lock["sources"]:
            if source["repository"].casefold() in repositories or source["id"] in by_source:
                raise ValueError("Repositories and source IDs must be distinct across batches")
            repositories.add(source["repository"].casefold())
            by_source[source["id"]] = {"repository": source["repository"], "batch": batch["batch"],
                                       **score([row for row in selected if row["source"] == source["id"]], reviewed)}
    overall = score(rows, decisions)
    dominant = [source for source, metrics in by_source.items() if metrics["samples"] * 2 > overall["samples"]]
    diversity = manifest.get("source_diversity_review", {})
    diversity_met = diversity.get("reviewed") is True and bool(diversity.get("reason", "").strip()) and (not dominant or diversity.get("owner_accepted") is True)
    numeric = overall["samples"] >= 100 and overall["conservative_precision"] >= 0.95
    result = {"schema_version": 1, "rule": "LNK002", "manifest_sha256": digest(manifest_raw), "inputs": inputs,
              "summary_script_sha256": digest(Path(__file__).read_bytes()), "protocol": protocol,
              "overall": overall, "by_batch": by_batch, "by_source": by_source, "review_provenance": review_provenance,
              "by_language": {language: score([row for row in rows if row["language"] == language], decisions)
                              for language in sorted({"en", "zh", "ja"} | {row["language"] for row in rows})},
              "kind_breakdown": {"status": "unavailable", "reason": "Document responsibilities were not reviewed for this link-rule holdout."},
              "source_diversity_review": diversity, "gate": {"passes": numeric and diversity_met,
                  "minimum_samples": 100, "minimum_conservative_precision": 0.95, "numeric_threshold_met": numeric,
                  "all_diagnoses_reviewed_by_two_agents": True, "source_diversity_review_met": diversity_met,
                  "sources_over_half": dominant, "promotes_rule": False},
              "precision_definition": "tp / (tp + fp); null when there are no determined labels",
              "conservative_precision_definition": "tp / all diagnoses; uncertain counts against the rule"}
    if manifest_path.read_bytes() != manifest_raw:
        raise ValueError("Manifest changed during summary")
    write_new(output, encode(result))
    print(json.dumps({"overall": overall, "gate": result["gate"]}, sort_keys=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    pin_parser = commands.add_parser("pin", help="Resolve a precommitted batch and its exact Git trees")
    pin_parser.add_argument("--spec", type=Path, required=True)
    pin_parser.add_argument("--batch", type=int, required=True)
    fetch_parser = commands.add_parser("fetch", help="Fetch original blobs and write a bound inventory")
    fetch_parser.add_argument("--input", type=Path, required=True)
    evaluate_parser = commands.add_parser("evaluate", help="Run the frozen probe in both input orders")
    evaluate_parser.add_argument("--input", type=Path, required=True)
    evaluate_parser.add_argument("--profile", type=Path, required=True)
    evaluate_parser.add_argument("--sites", type=Path, required=True)
    summary_parser = commands.add_parser("summarize", help="Validate both agents' labels and report the gate")
    summary_parser.add_argument("--manifest", type=Path, required=True)
    for command in (pin_parser, fetch_parser, evaluate_parser, summary_parser):
        command.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "pin":
        pin(args.spec, args.batch, args.output)
    elif args.command == "fetch":
        fetch(args.input, args.output)
    elif args.command == "evaluate":
        evaluate(args.input, args.profile, args.sites, args.output)
    else:
        summarize(args.manifest, args.output)


if __name__ == "__main__":
    main()
