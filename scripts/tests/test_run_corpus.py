import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from scripts.evaluation.run_corpus import load_corpus, run_one
from scripts.evaluation import evaluate_m1
from scripts.evaluation import evaluate_m2
from scripts.evaluation import run_corpus


class CorpusRunnerTests(unittest.TestCase):
    def test_fingerprints_cover_single_package_sources_and_embedded_rules(self):
        for fingerprints in (run_corpus.parser_sources, evaluate_m1.fingerprints, evaluate_m2.fingerprints):
            with self.subTest(tool=fingerprints.__module__):
                recorded = fingerprints()
                self.assertIn("src/lib.rs", recorded)
                self.assertIn("src/md/parser.rs", recorded)
                self.assertIn("docs/rules/KND001.md", recorded)
                self.assertTrue(any(name.startswith("examples/") for name in recorded))
                self.assertFalse(any(name.startswith("crates/") for name in recorded))

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.content = b"# A real document\n"
        git_blob = hashlib.sha1(b"blob " + str(len(self.content)).encode() + b"\0" + self.content).hexdigest()
        self.document = {"path": "docs/readme.md", "bytes": len(self.content), "git_blob": git_blob,
                         "sha256": hashlib.sha256(self.content).hexdigest()}
        self.path = self.root / "data/blobs" / git_blob
        self.path.parent.mkdir(parents=True)
        self.path.write_bytes(self.content)
        self.source = {"id": "sample", "repository": "owner/repository", "commit": "a" * 40,
                       "split": "tuning", "language_focus": "en", "category": "developer-tooling",
                       "documents": [self.document]}

    def lock(self, sources):
        (self.root / "corpus.lock.json").write_text(json.dumps({"schema_version": 1, "sources": sources}), encoding="utf-8")

    def test_empty_sources_unpinned_revisions_and_cross_split_overlap_fail(self):
        for sources in [[], [self.source | {"commit": "main"}],
                        [self.source, self.source | {"id": "other", "split": "holdout"}]]:
            with self.subTest(sources=sources):
                self.lock(sources)
                with self.assertRaises(ValueError):
                    load_corpus(self.root)

    def test_missing_and_tampered_files_do_not_run_the_parser(self):
        with patch("scripts.evaluation.run_corpus.subprocess.run") as process:
            self.path.write_bytes(b"Changed")
            result = run_one(Path("probe"), self.root, "sample", self.document, 1)
            self.assertEqual(result["status"], "input_error")
            self.path.unlink()
            result = run_one(Path("probe"), self.root, "sample", self.document, 1)
            self.assertEqual(result["status"], "input_error")
            process.assert_not_called()

    def test_aborts_and_timeouts_are_failed_results(self):
        with patch("scripts.evaluation.run_corpus.subprocess.run", return_value=subprocess.CompletedProcess([], -6, b"", b"stack overflow")):
            result = run_one(Path("probe"), self.root, "sample", self.document, 1)
            self.assertEqual(result["status"], "process_error")
            self.assertIn("stack overflow", result["error"])
        with patch("scripts.evaluation.run_corpus.subprocess.run", side_effect=subprocess.TimeoutExpired([], 1)):
            result = run_one(Path("probe"), self.root, "sample", self.document, 1)
            self.assertEqual(result["status"], "timeout")

    def test_success_requires_both_flavors_and_the_exact_input_hash(self):
        outcome = {"status": "ok", "flavors": [{"flavor": "gfm", "source_sha256": self.document["sha256"]}], "error": None}
        with patch("scripts.evaluation.run_corpus.subprocess.run", return_value=subprocess.CompletedProcess([], 0, json.dumps(outcome).encode(), b"")):
            result = run_one(Path("probe"), self.root, "sample", self.document, 1)
            self.assertEqual(result["status"], "process_error")
        outcome["flavors"].insert(0, {"flavor": "common_mark", "source_sha256": "f" * 64})
        with patch("scripts.evaluation.run_corpus.subprocess.run", return_value=subprocess.CompletedProcess([], 0, json.dumps(outcome).encode(), b"")):
            result = run_one(Path("probe"), self.root, "sample", self.document, 1)
            self.assertEqual(result["status"], "process_error")


if __name__ == "__main__":
    unittest.main()
