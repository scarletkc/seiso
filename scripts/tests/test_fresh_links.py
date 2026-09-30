import contextlib
import copy
import gzip
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from scripts.evaluation import evaluate_m2, fresh_links
from scripts.evaluation.evaluate_m2 import digest, encode, read


class FreshLinkTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.root_patch = patch.object(fresh_links, "ROOT", self.root)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)
        self.cache_patch = patch.object(fresh_links.corpus_tools, "ROOT", self.root / "corpus")
        self.cache_patch.start()
        self.addCleanup(self.cache_patch.stop)
        self.quiet = contextlib.redirect_stdout(io.StringIO())
        self.quiet.__enter__()
        self.addCleanup(self.quiet.__exit__, None, None, None)
        (self.root / "corpus").mkdir()
        (self.root / "corpus/corpus.lock.json").write_bytes(encode({"sources": []}))
        (self.root / "examples").mkdir()
        (self.root / "examples/evaluate_m2.rs").write_bytes(b"fixed probe source\n")
        self.binary = self.root / "probe"
        self.binary.write_bytes(b"fixed executable")
        self.spec_path = self.root / "sources.json"
        self.source = {"id": "project", "repository": "owner/project", "batch": 1,
                       "include": ["README.md", "docs/**"]}
        self.spec = {"schema_version": 1, "freeze_commit": "f" * 40, "sources": [self.source]}
        self.spec_path.write_bytes(encode(self.spec))
        self.blobs = {}
        entries = [{"path": "docs", "type": "tree", "mode": "040000", "sha": "c" * 40}]
        for path, raw in [("README.md", b"# Intro\r\n" + b"[link](docs/guide.md#missing)\r\n" * 100),
                          ("docs/guide.md", "# 文档\r\n\r\nUnchanged bytes.\r\n".encode()),
                          ("LICENSE", b"Permission granted.\n"), ("mkdocs.yml", b"site_name: Sample\n")]:
            blob = hashlib.sha1(f"blob {len(raw)}\0".encode() + raw).hexdigest()
            self.blobs[blob] = raw
            entries.append({"path": path, "type": "blob", "mode": "100644", "sha": blob, "size": len(raw)})
        self.tree = {"sha": "b" * 40, "truncated": False, "tree": entries}
        self.batch = self.root / "batch-1"
        self.selection_path = self.batch / "selection.json"
        self.lock_path = self.batch / "corpus.lock.json"
        self.profile_path = self.batch / "kinds.json"
        self.sites_path = self.batch / "sites.json"

    def github(self, route):
        if route == "repos/owner/project":
            return {"private": False, "full_name": "owner/project", "default_branch": "main", "license": {"spdx_id": "MIT"}}
        if route == "repos/owner/project/commits/main":
            return {"sha": "a" * 40, "commit": {"tree": {"sha": "b" * 40}}}
        if route == "repos/owner/project/git/trees/" + "b" * 40 + "?recursive=1":
            return copy.deepcopy(self.tree)
        raise AssertionError(f"Unexpected GitHub request: {route}")

    def pin(self):
        with patch.object(fresh_links.corpus_tools, "github", side_effect=self.github) as github:
            fresh_links.pin(self.spec_path, 1, self.selection_path)
        return github

    def fetch_blob(self, source, entry):
        raw = self.blobs[entry["git_blob"]]
        path = self.root / "corpus/data/blobs" / entry["git_blob"]
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(raw)
        return digest(raw)

    def cohort(self):
        self.pin()
        with patch.object(fresh_links.corpus_tools, "fetch_blob", side_effect=self.fetch_blob):
            fresh_links.fetch(self.selection_path, self.lock_path)
        lock = read(self.lock_path)
        profile = {"schema_version": 1, "corpus_sha256": digest(self.lock_path.read_bytes()),
                   "documents": {f"{source['id']}/{record['path']}": {"kind": "unknown", "reason": "LNK002 applies to every kind.", "input_sha256": record["sha256"]}
                                 for source in lock["sources"] for record in source["documents"]}}
        sites = {"schema_version": 1, "corpus_sha256": profile["corpus_sha256"],
                 "profiles": {"project": {"sites": [], "reason": "No reviewed site mapping."}}}
        self.profile_path.write_bytes(encode(profile))
        self.sites_path.write_bytes(encode(sites))
        return lock, profile, sites

    def files(self, lock, diagnoses=0):
        files = [{"source": source["id"], "path": item["path"], "sha256": item["sha256"], "language": "en",
                  "links": [], "incomplete_rules": [], "result": {"kind": {"value": None}, "diagnostics": [], "errors": []}}
                 for source in lock["sources"] for item in source["documents"]]
        files[0]["result"]["diagnostics"] = [{"code": "LNK002", "byte_range": {"start": i, "end": i + 1},
                                             "related": [{"filename": "docs/guide.md", "byte_range": {"start": 0, "end": 0}}],
                                             "message": "Missing anchor", "location": {"row": 2}}
                                            for i in range(diagnoses)]
        return files

    def evaluate(self, lock, diagnoses=0, probe=None, fingerprints=None):
        files = self.files(lock, diagnoses)
        with patch.object(fresh_links, "build_probe", return_value=self.binary), \
             patch.object(fresh_links, "implementation_fingerprints", side_effect=fingerprints, return_value={"engine": "fixed"}), \
             patch.object(fresh_links, "run_probe", side_effect=probe, return_value=encode(files)):
            fresh_links.evaluate(self.lock_path, self.profile_path, self.sites_path, self.batch / "run")
        return json.loads(gzip.decompress((self.batch / "run/diagnostics.json.gz").read_bytes()))

    def summary_inputs(self, diagnoses=2):
        lock, _, _ = self.cohort()
        report = self.evaluate(lock, diagnoses)
        labels = []
        for row in report["diagnostics"]:
            review = {"label": "tp", "reason": "Original target lacks the fragment.", "renderer": "GitHub",
                      "evidence": "Pinned target headings and HTML anchors inspected.", "agent_context_reviewed": True}
            labels.append({key: row[key] for key in ("id", "code", "source", "path", "input_sha256", "span", "related_inputs", "split")} |
                          review | {"diagnostic_sha256": digest(encode(row["diagnostic"])),
                                    "author_review": review | {"reviewer": "author-agent"},
                                    "independent_review": review | {"reviewer": "review-agent"}})
        bundle = {"schema_version": 1, "scope_codes": ["LNK002"], "human_reviewers": 0,
                  "reviewer_kind": "agent_consensus", "labels": labels, "decision_receipt_sha256": "d" * 64,
                  "report_sha256": digest((self.batch / "run/diagnostics.json.gz").read_bytes()),
                  "corpus_sha256": report["corpus_sha256"], "inventory_sha256": report["inventory_sha256"]}
        labels_path = self.batch / "labels.json"
        labels_path.write_bytes(encode(bundle))
        manifest = {"schema_version": 1, "batches": [{"batch": 1, "report": "batch-1/run/diagnostics.json.gz",
                    "labels": "batch-1/labels.json", "corpus_lock": "batch-1/corpus.lock.json"}],
                    "source_diversity_review": {"reviewed": True, "reason": "Single source needs owner acceptance."}}
        manifest_path = self.root / "manifest.json"
        manifest_path.write_bytes(encode(manifest))
        return manifest_path, manifest, labels_path, bundle

    def append_summary_batch(self, manifest_path, manifest, diagnoses):
        number = len(manifest["batches"]) + 1
        directory = self.root / f"batch-{number}"
        directory.mkdir()
        lock = copy.deepcopy(read(self.lock_path))
        lock["batch"] = number
        lock["pinned_selection_sha256"] = str(number) * 64
        source = lock["sources"][0]
        source.update(id=f"project-{number}", repository=f"owner/project-{number}", batch=number)
        lock_raw = encode(lock)
        (directory / "corpus.lock.json").write_bytes(lock_raw)
        report = json.loads(gzip.decompress((self.batch / "run/diagnostics.json.gz").read_bytes()))
        report.update(batch=number, corpus_sha256=digest(lock_raw),
                      pinned_selection_sha256=lock["pinned_selection_sha256"], files=self.files(lock, diagnoses))
        report["diagnostics"] = evaluate_m2.diagnostic_rows(report["files"], lock)
        packed = gzip.compress(encode(report), mtime=0)
        (directory / "diagnostics.json.gz").write_bytes(packed)
        reviews = []
        for row in report["diagnostics"]:
            review = {"label": "tp", "reason": "Individually inspected synthetic target lacks fragment.",
                      "renderer": "GitHub", "evidence": "Synthetic target original bytes.", "agent_context_reviewed": True}
            reviews.append({key: row[key] for key in ("id", "code", "source", "path", "span", "related_inputs", "split", "input_sha256")} |
                review | {"diagnostic_sha256": digest(encode(row["diagnostic"])),
                          "author_review": review | {"reviewer": "author-agent"},
                          "independent_review": review | {"reviewer": "review-agent"}})
        bundle = {"schema_version": 1, "scope_codes": ["LNK002"], "human_reviewers": 0,
                  "reviewer_kind": "agent_consensus", "labels": reviews, "decision_receipt_sha256": "d" * 64,
                  "report_sha256": digest(packed), "corpus_sha256": report["corpus_sha256"],
                  "inventory_sha256": report["inventory_sha256"]}
        (directory / "labels.json").write_bytes(encode(bundle))
        manifest["batches"].append({"batch": number, "report": f"batch-{number}/diagnostics.json.gz",
            "labels": f"batch-{number}/labels.json", "corpus_lock": f"batch-{number}/corpus.lock.json"})
        manifest["source_diversity_review"]["owner_accepted"] = True
        manifest_path.write_bytes(encode(manifest))

    def test_specification_rejects_invalid_or_duplicate_sources_before_network(self):
        variants = [self.spec | {"schema_version": 2}, self.spec | {"freeze_commit": "HEAD"},
                    self.spec | {"sources": []}]
        for change in [{"id": "../bad"}, {"repository": "https://github.com/owner/repo"}, {"batch": True},
                       {"include": []}, {"include": ["docs/**", "docs/**"]}, {"include": ["../docs/**"]},
                       {"include": ["/docs/**"]}, {"include": ["docs\\*"]}, {"commit": "a" * 40}, {"split": "tuning"}]:
            variants.append(self.spec | {"sources": [self.source | change]})
        variants.append(self.spec | {"sources": [self.source, self.source | {"id": "second", "batch": 2, "repository": "OWNER/PROJECT"}]})
        for spec in variants:
            with self.subTest(spec=spec), self.assertRaises(ValueError):
                fresh_links.validate_spec(spec, 1)
        with self.assertRaises(ValueError):
            fresh_links.validate_spec(self.spec, 2)

    def test_overlap_rejection_is_case_insensitive_and_includes_all_historical_results(self):
        for relative in ["corpus/corpus.lock.json", "corpus/results/other/nested/corpus.lock.json", "corpus/results/other/selection.json"]:
            with self.subTest(relative=relative):
                path = self.root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(encode({"sources": [{"repository": "OWNER/PROJECT"}]}))
                with patch.object(fresh_links.corpus_tools, "resolve_source") as network, self.assertRaisesRegex(ValueError, "overlap"):
                    fresh_links.pin(self.spec_path, 1, self.selection_path)
                network.assert_not_called()
                path.write_bytes(encode({"sources": []}))

    def test_pin_stores_the_tree_from_the_same_resolution(self):
        network = self.pin()
        self.assertEqual(network.call_count, 3)
        result = read(self.selection_path)
        source = result["sources"][0]
        self.assertEqual(source["inventory"]["tree"], source["tree"])
        self.assertEqual(source["inventory"]["commit"], source["commit"])
        self.assertEqual(result["spec_sha256"], digest(self.spec_path.read_bytes()))
        self.assertEqual({doc["path"] for doc in source["documents"]}, {"README.md", "docs/guide.md"})
        self.assertEqual(source["licenses"][0]["path"], "LICENSE")

    def test_pin_records_mechanical_failures_without_replacement(self):
        original = copy.deepcopy(self.tree)
        for mutation in ["truncated", "unknown_completeness", "no_license", "no_markdown"]:
            with self.subTest(mutation=mutation):
                self.tree = copy.deepcopy(original)
                if mutation == "truncated":
                    self.tree["truncated"] = True
                elif mutation == "unknown_completeness":
                    self.tree.pop("truncated")
                elif mutation == "no_license":
                    self.tree["tree"] = [row for row in self.tree["tree"] if row["path"] != "LICENSE"]
                else:
                    self.tree["tree"] = [row for row in self.tree["tree"] if not row["path"].endswith(".md")]
                self.pin()
                result = read(self.selection_path)
                self.assertEqual(result["sources"], [])
                self.assertEqual(len(result["skipped"]), 1)
                self.assertTrue(result["skipped"][0]["reason"])
                self.selection_path.unlink()

    def test_fetch_binds_archives_and_verifies_original_document_and_license_bytes(self):
        lock, _, _ = self.cohort()
        inventory_path = self.batch / "inventory/inventory.lock.json"
        inventory = read(inventory_path)
        trees, _ = fresh_links.validate_inventory(lock, digest(self.lock_path.read_bytes()), inventory_path, inventory)
        self.assertIn("mkdocs.yml", trees["project"])
        for entry in lock["sources"][0]["documents"] + lock["sources"][0]["licenses"]:
            raw = (self.root / "corpus/data/blobs" / entry["git_blob"]).read_bytes()
            self.assertEqual(raw, self.blobs[entry["git_blob"]])
            self.assertEqual(entry["sha256"], digest(raw))
        self.assertNotIn("inventory", lock["sources"][0])
        self.assertEqual(lock["pinned_selection_sha256"], digest(self.selection_path.read_bytes()))

    def test_fetch_rejects_changed_tree_identity(self):
        self.pin()
        selection = read(self.selection_path)
        selection["sources"][0]["inventory"]["commit"] = "c" * 40
        self.selection_path.write_bytes(encode(selection))
        with patch.object(fresh_links.corpus_tools, "fetch_blob") as network, self.assertRaisesRegex(ValueError, "identity"):
            fresh_links.fetch(self.selection_path, self.lock_path)
        network.assert_not_called()

    def test_empty_mechanically_skipped_batch_fetches_and_evaluates_without_network(self):
        self.tree["truncated"] = True
        self.pin()
        with patch.object(fresh_links.corpus_tools, "fetch_blob") as network:
            fresh_links.fetch(self.selection_path, self.lock_path)
        network.assert_not_called()
        lock = read(self.lock_path)
        self.assertEqual(lock["sources"], [])
        self.assertEqual(len(lock["skipped"]), 1)
        self.profile_path.write_bytes(encode({"schema_version": 1,
            "corpus_sha256": digest(self.lock_path.read_bytes()), "documents": {}}))
        self.sites_path.write_bytes(encode({"schema_version": 1,
            "corpus_sha256": digest(self.lock_path.read_bytes()), "profiles": {}}))
        with patch.object(fresh_links, "build_probe", return_value=self.binary), \
             patch.object(fresh_links, "implementation_fingerprints", return_value={"engine": "fixed"}), \
             patch.object(fresh_links, "run_probe", return_value=b"[]"):
            fresh_links.evaluate(self.lock_path, self.profile_path, self.sites_path, self.batch / "run")
        receipt = read(self.batch / "run/run.json")
        self.assertEqual(receipt["files"], 0)
        self.assertEqual(receipt["counts"], {"LNK001": 0, "LNK002": 0})
        self.assertTrue(receipt["reverse_byte_identical"])
        report = json.loads(gzip.decompress((self.batch / "run/diagnostics.json.gz").read_bytes()))
        (self.batch / "labels.json").write_bytes(encode({"schema_version": 1, "scope_codes": ["LNK002"],
            "human_reviewers": 0, "reviewer_kind": "agent_consensus", "labels": [],
            "decision_receipt_sha256": "d" * 64, "report_sha256": receipt["report_sha256"],
            "corpus_sha256": report["corpus_sha256"], "inventory_sha256": report["inventory_sha256"]}))
        manifest = {"schema_version": 1, "batches": [{"batch": 1,
            "report": "batch-1/run/diagnostics.json.gz", "labels": "batch-1/labels.json",
            "corpus_lock": "batch-1/corpus.lock.json"}],
            "source_diversity_review": {"reviewed": True, "reason": "All sources mechanically skipped."}}
        manifest_path = self.root / "manifest.json"
        manifest_path.write_bytes(encode(manifest))
        output = self.root / "summary.json"
        fresh_links.summarize(manifest_path, output)
        self.assertEqual(read(output)["overall"]["samples"], 0)
        self.assertEqual(read(output)["by_batch"]["1"]["skipped"], lock["skipped"])
        self.assertFalse(read(output)["gate"]["passes"])

    def test_profile_requires_exact_document_coverage_hashes_and_reasons(self):
        lock, profile, _ = self.cohort()
        key = next(iter(profile["documents"]))
        for mutation in ["missing", "extra", "changed", "reason", "kind", "lock"]:
            changed = copy.deepcopy(profile)
            if mutation == "missing":
                changed["documents"].pop(key)
            elif mutation == "extra":
                changed["documents"]["project/extra.md"] = changed["documents"][key]
            elif mutation == "changed":
                changed["documents"][key]["input_sha256"] = "0" * 64
            elif mutation == "reason":
                changed["documents"][key]["reason"] = " "
            elif mutation == "kind":
                changed["documents"][key]["kind"] = "invented"
            else:
                changed["corpus_sha256"] = "0" * 64
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                fresh_links.kind_mappings(lock, digest(self.lock_path.read_bytes()), changed)

    def test_unknown_kinds_and_reviewed_sites_render_only_active_config_fields(self):
        lock, profile, sites = self.cohort()
        entry = next(row for row in self.tree["tree"] if row["path"] == "mkdocs.yml")
        site = {"path": "docs/**", "root": "docs", "public": "docs/public", "base": "/manual/", "generator": "MkDocs",
                "evidence": "Pinned configuration declares the source.", "evidence_path": "mkdocs.yml", "evidence_git_blob": entry["sha"]}
        sites["profiles"]["project"]["sites"] = [site]
        inventory_path = self.batch / "inventory/inventory.lock.json"
        inventory = read(inventory_path)
        trees, _ = fresh_links.validate_inventory(lock, digest(self.lock_path.read_bytes()), inventory_path, inventory)
        fresh_links.validate_sites(lock, digest(self.lock_path.read_bytes()), sites, trees)
        mapped = fresh_links.kind_mappings(lock, digest(self.lock_path.read_bytes()), profile)
        inputs = fresh_links.prepare_inputs(self.root / "corpus", lock, mapped, inventory, sites,
                                            inventory_dir=inventory_path.parent, rules=fresh_links.RULES)
        config = inputs["sources"][0]["config"]
        self.assertNotIn("[[kinds]]", config)
        self.assertIn('[[sites]]\npath="docs/**"\nroot="docs"\npublic="docs/public"\nbase="/manual/"', config)
        self.assertIn('[lint]\nselect=["LNK001", "LNK002"]', config)
        self.assertNotIn("evidence", config)
        self.assertNotIn("generator", config)
        site["evidence_git_blob"] = "0" * 40
        with self.assertRaisesRegex(ValueError, "pinned configuration"):
            fresh_links.validate_sites(lock, digest(self.lock_path.read_bytes()), sites, trees)

    def test_site_reviews_cannot_omit_a_source_or_use_another_lock(self):
        lock, _, sites = self.cohort()
        for change in [{"profiles": {}}, {"corpus_sha256": "0" * 64},
                       {"profiles": {"project": {"sites": []}}}]:
            with self.subTest(change=change), self.assertRaises(ValueError):
                fresh_links.validate_sites(lock, digest(self.lock_path.read_bytes()), sites | change, {})

    def test_default_prepare_inputs_preserves_historical_configuration_and_raw_paths(self):
        lock, _, sites = self.cohort()
        inventory = read(self.batch / "inventory/inventory.lock.json")
        shutil_source = self.batch / "inventory"
        import shutil
        shutil.copytree(shutil_source, self.root / "corpus/inventory")
        profiles = {"profiles": {"project": {"mappings": [{"path": "README.md", "kind": "readme"}]}}}
        result = evaluate_m2.prepare_inputs(self.root / "corpus", lock, profiles, inventory)
        self.assertEqual(result["sources"][0]["config"], 'preview=true\n\n[[kinds]]\npath="README.md"\nkind="readme"\n')
        self.assertNotIn("[lint]", result["sources"][0]["config"])
        self.assertEqual(result, evaluate_m2.prepare_inputs(self.root / "corpus", lock, profiles, inventory, sites))
        for document in result["sources"][0]["documents"]:
            raw = Path(document["blob"]).read_bytes()
            self.assertIn(b"\r\n", raw)

    def test_evaluate_binds_full_results_and_reverses_documents_and_trees(self):
        lock, _, _ = self.cohort()
        calls = []
        files = self.files(lock, 1)
        files[0]["links"] = [{"destination": "docs/guide.md#missing", "span": {"start": 0, "end": 1}}]
        files[0]["incomplete_rules"] = ["LNK002"]
        def probe(binary, inputs, directory, order):
            calls.append(copy.deepcopy(inputs))
            return encode(files)
        report = self.evaluate(lock, probe=probe)
        self.assertEqual(calls[1], evaluate_m2.reverse_inputs(calls[0]))
        self.assertNotEqual(calls[0]["sources"][0]["documents"], calls[1]["sources"][0]["documents"])
        self.assertEqual(report["files"], files)
        self.assertEqual(report["diagnostics"][0]["split"], "holdout")
        self.assertEqual(report["diagnostics"][0]["related_inputs"][0]["sha256"], lock["sources"][0]["documents"][1]["sha256"])
        for key, path in [("corpus_sha256", self.lock_path), ("kind_profile_sha256", self.profile_path),
                          ("site_profile_sha256", self.sites_path), ("inventory_sha256", self.batch / "inventory/inventory.lock.json")]:
            self.assertEqual(report[key], digest(path.read_bytes()))
        self.assertEqual(report["probe_sha256"], digest(self.binary.read_bytes()))
        self.assertEqual(read(self.batch / "run/run.json")["incomplete_rule_files"], {"LNK002": 1})

    def test_evaluate_detects_reversed_order_difference_and_writes_no_run(self):
        lock, _, _ = self.cohort()
        with self.assertRaisesRegex(ValueError, "reversed-order"):
            self.evaluate(lock, probe=[encode(self.files(lock)), b"[]"])
        self.assertFalse((self.batch / "run").exists())

    def test_evaluate_detects_implementation_changes(self):
        lock, _, _ = self.cohort()
        with self.assertRaisesRegex(ValueError, "Implementation changed"):
            self.evaluate(lock, fingerprints=[{"engine": "before"}, {"engine": "after"}])
        self.assertFalse((self.batch / "run").exists())

    def test_evaluate_detects_input_profile_changes_during_the_probe(self):
        lock, _, _ = self.cohort()
        def probe(*args):
            self.profile_path.write_bytes(b"changed")
            return encode(self.files(lock))
        with self.assertRaisesRegex(ValueError, "input bindings changed"):
            self.evaluate(lock, probe=probe)
        self.assertFalse((self.batch / "run").exists())

    def test_probe_result_requires_exact_document_identities_and_metadata(self):
        lock, _, _ = self.cohort()
        original = self.files(lock)
        for mutation in ["missing", "duplicate", "hash", "links", "incomplete", "unselected"]:
            files = copy.deepcopy(original)
            if mutation == "missing":
                files.pop()
            elif mutation == "duplicate":
                files.append(files[0])
            elif mutation == "hash":
                files[0]["sha256"] = "0" * 64
            elif mutation == "links":
                files[0].pop("links")
            elif mutation == "incomplete":
                files[0].pop("incomplete_rules")
            else:
                files[0]["result"]["diagnostics"] = [{"code": "OWN001"}]
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                fresh_links.validate_files(files, lock)

    def test_every_command_refuses_to_overwrite_before_reading_inputs(self):
        occupied = self.root / "occupied"
        occupied.mkdir()
        missing = self.root / "absent.json"
        commands = [lambda: fresh_links.pin(missing, 1, occupied), lambda: fresh_links.fetch(missing, occupied),
                    lambda: fresh_links.evaluate(missing, missing, missing, occupied), lambda: fresh_links.summarize(missing, occupied)]
        for command in commands:
            with self.assertRaisesRegex(ValueError, "Preserve"):
                command()

    def test_fetch_refuses_to_overwrite_existing_inventory(self):
        self.batch.mkdir()
        (self.batch / "inventory").mkdir()
        with self.assertRaisesRegex(ValueError, "Preserve"):
            fresh_links.fetch(self.root / "absent.json", self.lock_path)

    def test_summary_requires_bound_complete_distinct_individual_reviews(self):
        manifest_path, _, labels_path, original = self.summary_inputs()
        for mutation in ["missing", "duplicate", "report", "source", "span", "related", "payload", "receipt", "review", "same_agent", "renderer", "disagreement"]:
            bundle = copy.deepcopy(original)
            if mutation == "missing":
                bundle["labels"].pop()
            elif mutation == "duplicate":
                bundle["labels"].append(bundle["labels"][0])
            elif mutation == "report":
                bundle["report_sha256"] = "0" * 64
            elif mutation == "receipt":
                bundle.pop("decision_receipt_sha256")
            else:
                label = bundle["labels"][0]
                if mutation == "source":
                    label["source"] = "other"
                elif mutation == "span":
                    label["span"]["start"] += 1
                elif mutation == "related":
                    label["related_inputs"] = []
                elif mutation == "payload":
                    label["diagnostic_sha256"] = "0" * 64
                elif mutation == "review":
                    label["independent_review"]["agent_context_reviewed"] = False
                elif mutation == "same_agent":
                    label["independent_review"]["reviewer"] = "author-agent"
                elif mutation == "renderer":
                    label["renderer"] = ""
                else:
                    label["independent_review"]["label"] = "uncertain"
            labels_path.write_bytes(encode(bundle))
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                fresh_links.summarize(manifest_path, self.root / "summary.json")

    def test_summary_counts_uncertain_against_precision_and_reports_agreement(self):
        manifest_path, _, labels_path, bundle = self.summary_inputs()
        label = bundle["labels"][0]
        label["label"] = "uncertain"
        label["independent_review"]["label"] = "uncertain"
        label["disagreement_reason"] = "Renderer evidence does not establish the generated anchor."
        labels_path.write_bytes(encode(bundle))
        output = self.root / "summary.json"
        fresh_links.summarize(manifest_path, output)
        result = read(output)
        self.assertEqual(result["overall"], {"samples": 2, "tp": 1, "fp": 0, "uncertain": 1,
                         "precision": 1.0, "conservative_precision": 0.5, "agreements": 1, "agreement_rate": 0.5})
        self.assertEqual(result["by_source"]["project"]["samples"], 2)
        self.assertEqual(result["by_batch"]["1"]["samples"], 2)
        self.assertEqual(result["by_language"]["en"]["samples"], 2)
        self.assertIsNone(result["by_language"]["zh"]["precision"])
        self.assertFalse(result["gate"]["passes"])

    def test_gate_requires_source_diversity_review_and_owner_acceptance_for_dominance(self):
        manifest_path, manifest, _, _ = self.summary_inputs(100)
        first = self.root / "summary.json"
        fresh_links.summarize(manifest_path, first)
        self.assertTrue(read(first)["gate"]["numeric_threshold_met"])
        self.assertFalse(read(first)["gate"]["passes"])
        manifest["source_diversity_review"]["owner_accepted"] = True
        manifest_path.write_bytes(encode(manifest))
        second = self.root / "accepted.json"
        fresh_links.summarize(manifest_path, second)
        self.assertTrue(read(second)["gate"]["passes"])
        self.assertFalse(read(second)["gate"]["promotes_rule"])

    def test_summary_refuses_extra_batch_after_first_reaches_100_even_if_precision_improves(self):
        manifest_path, manifest, labels_path, bundle = self.summary_inputs(100)
        for label in bundle["labels"][-6:]:
            label["label"] = "fp"
            label["author_review"]["label"] = "fp"
            label["independent_review"]["label"] = "fp"
        labels_path.write_bytes(encode(bundle))
        # A nonexistent second report must not even be opened. More favorable
        # later labels cannot dilute this first batch's six false positives.
        manifest["batches"].append({"batch": 2, "report": "forbidden-extra-report.json.gz",
            "labels": "forbidden-extra-labels.json", "corpus_lock": "forbidden-extra-lock.json"})
        manifest_path.write_bytes(encode(manifest))
        with self.assertRaisesRegex(ValueError, "first batch reaching 100"):
            fresh_links.summarize(manifest_path, self.root / "summary.json")
        self.assertFalse((self.root / "summary.json").exists())

    def test_complete_summary_at_threshold_counts_false_positives_without_adding_batches(self):
        manifest_path, manifest, labels_path, bundle = self.summary_inputs(100)
        for label in bundle["labels"][-6:]:
            label["label"] = "fp"
            label["author_review"]["label"] = "fp"
            label["independent_review"]["label"] = "fp"
        manifest["source_diversity_review"]["owner_accepted"] = True
        manifest_path.write_bytes(encode(manifest))
        labels_path.write_bytes(encode(bundle))
        output = self.root / "summary.json"
        fresh_links.summarize(manifest_path, output)
        result = read(output)
        self.assertEqual(result["overall"]["samples"], 100)
        self.assertEqual(result["overall"]["fp"], 6)
        self.assertEqual(result["overall"]["conservative_precision"], 0.94)
        self.assertFalse(result["gate"]["passes"])

    def test_summary_accepts_batch_that_first_brings_cumulative_count_to_exactly_100(self):
        manifest_path, manifest, _, _ = self.summary_inputs(40)
        self.append_summary_batch(manifest_path, manifest, 60)
        output = self.root / "summary.json"
        fresh_links.summarize(manifest_path, output)
        result = read(output)
        self.assertEqual(result["overall"]["samples"], 100)
        self.assertEqual(result["by_batch"]["1"]["samples"], 40)
        self.assertEqual(result["by_batch"]["2"]["samples"], 60)
        self.assertTrue(result["gate"]["passes"])
        manifest["batches"].append({"batch": 3, "report": "forbidden-third-report.json.gz",
            "labels": "forbidden-third-labels.json", "corpus_lock": "forbidden-third-lock.json"})
        manifest_path.write_bytes(encode(manifest))
        with self.assertRaisesRegex(ValueError, "first batch reaching 100"):
            fresh_links.summarize(manifest_path, self.root / "forbidden-summary.json")

    def test_summary_retains_every_diagnosis_in_batch_that_crosses_100(self):
        manifest_path, manifest, _, _ = self.summary_inputs(99)
        self.append_summary_batch(manifest_path, manifest, 2)
        output = self.root / "summary.json"
        fresh_links.summarize(manifest_path, output)
        self.assertEqual(read(output)["overall"]["samples"], 101)
        self.assertEqual(read(output)["overall"]["agreements"], 101)

    def test_summary_accepts_zero_diagnosis_batch_before_first_threshold_batch(self):
        manifest_path, manifest, _, _ = self.summary_inputs(0)
        self.append_summary_batch(manifest_path, manifest, 100)
        output = self.root / "summary.json"
        fresh_links.summarize(manifest_path, output)
        result = read(output)
        self.assertEqual(result["by_batch"]["1"]["samples"], 0)
        self.assertIsNone(result["by_batch"]["1"]["precision"])
        self.assertEqual(result["overall"]["samples"], 100)
        self.assertTrue(result["gate"]["passes"])

    def test_empty_samples_keep_unavailable_precision(self):
        manifest_path, _, _, _ = self.summary_inputs(0)
        output = self.root / "summary.json"
        fresh_links.summarize(manifest_path, output)
        self.assertIsNone(read(output)["overall"]["precision"])
        self.assertIsNone(read(output)["overall"]["conservative_precision"])
        self.assertFalse(read(output)["gate"]["passes"])


if __name__ == "__main__":
    unittest.main()
