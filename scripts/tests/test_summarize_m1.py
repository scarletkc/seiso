import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from scripts.evaluation.summarize_m1 import PROTOCOL, sha, summarize, verify_protocol


class AcceptanceTests(unittest.TestCase):
    def fixture(self, count=100, false=0):
        diagnoses = [{"id": str(i), "code": "KND001", "split": "holdout", "language": "en", "kind": "unknown"} for i in range(count)]
        labels = [{"id": str(i), "label": "fp" if i < false else "tp", "evidence": "Independently checked declaration."} for i in range(count)]
        return {"diagnostics": diagnoses}, {"report_sha256": "run", "labels": labels, "manually_reviewed_sample_ids": [str(i) for i in range(count)]}

    def test_threshold_is_per_rule_and_small_or_tuning_only_samples_cannot_pass(self):
        for count, false, expected in [(100, 5, True), (100, 6, False), (99, 0, False)]:
            report, annotations = self.fixture(count, false)
            result = summarize(report, "run", [annotations])
            self.assertEqual(result["rules"]["KND001"]["eligible_for_stable"], expected)
            self.assertFalse(result["m1_exit_passed"])
        report, annotations = self.fixture()
        for item in report["diagnostics"]:
            item["split"] = "tuning"
        self.assertFalse(summarize(report, "run", [annotations])["rules"]["KND001"]["eligible_for_stable"])

    def test_empty_protocol_samples_remain_unavailable_even_with_exception(self):
        result = summarize({"diagnostics": []}, "run", [], True)
        self.assertTrue(result["rules"]["SUP001"]["eligible_for_stable"])
        self.assertIsNone(result["rules"]["SUP001"]["holdout"]["precision"])
        self.assertFalse(result["m1_exit_passed"])

    def test_missing_duplicate_or_unbound_labels_fail_closed(self):
        report, annotations = self.fixture()
        for invalid in [annotations | {"labels": annotations["labels"][:-1]}, annotations | {"report_sha256": "other"},
                        annotations | {"labels": annotations["labels"] * 2}]:
            with self.assertRaises(ValueError):
                summarize(report, "run", [invalid])

    def test_uncertainty_and_unreviewed_oracle_labels_cannot_inflate_acceptance(self):
        report, annotations = self.fixture()
        annotations["manually_reviewed_sample_ids"] = []
        self.assertFalse(summarize(report, "run", [annotations])["rules"]["KND001"]["eligible_for_stable"])
        annotations["manually_reviewed_sample_ids"] = [str(i) for i in range(100)]
        for label in annotations["labels"][:6]:
            label["label"] = "uncertain"
        result = summarize(report, "run", [annotations])["rules"]["KND001"]
        self.assertEqual(result["holdout"]["precision"], 1)
        self.assertFalse(result["eligible_for_stable"])


class ProtocolReplayTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.git("init", "--quiet")
        self.git("config", "user.name", "Protocol Tests")
        self.git("config", "user.email", "tests@example.com")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "core.autocrlf", "false")
        self.source = self.root / "old-tests.rs"
        self.source.write_bytes(b"accepted tests\n")
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "Accepted protocol")
        self.revision = self.git("rev-parse", "HEAD").strip()
        (self.root / "syntax-audit.json").write_bytes(b"audit\n")
        self.path = self.root / "protocol.json"
        self.protocol = {
            "syntax_audit_sha256": sha(b"audit\n"),
            "test_sources": {"old-tests.rs": sha(b"accepted tests\n")},
            "status": "accepted", "owner_approved": True, "conformance_passed": True,
            "approved_rules": sorted(PROTOCOL),
        }
        self.write_receipt()

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, text=True, encoding="utf-8")

    def write_receipt(self):
        self.path.write_text(json.dumps(self.protocol), encoding="utf-8")

    def test_historical_replay_survives_source_moves_and_edits(self):
        self.source.rename(self.root / "new-tests.rs")
        (self.root / "new-tests.rs").write_bytes(b"current tests\n")
        self.assertEqual(verify_protocol(self.path, self.root, self.revision), (True, self.revision))

    def test_new_receipts_bind_their_source_revision(self):
        self.protocol["source_revision"] = self.revision
        self.write_receipt()
        self.assertEqual(verify_protocol(self.path, self.root), (True, self.revision))
        self.source.write_bytes(b"changed tests\n")
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "Changed protocol")
        with self.assertRaisesRegex(ValueError, "differs from the receipt"):
            verify_protocol(self.path, self.root, "HEAD")

    def test_old_receipt_requires_an_explicit_source_ref(self):
        with self.assertRaisesRegex(ValueError, "--protocol-source-ref"):
            verify_protocol(self.path, self.root)

    def test_wrong_revision_or_source_hash_cannot_replay_acceptance(self):
        self.source.write_bytes(b"changed tests\n")
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "Changed protocol")
        with self.assertRaisesRegex(ValueError, "Contract test differs"):
            verify_protocol(self.path, self.root, "HEAD")
        self.protocol["test_sources"]["old-tests.rs"] = sha(b"wrong source\n")
        self.write_receipt()
        with self.assertRaisesRegex(ValueError, "Contract test differs"):
            verify_protocol(self.path, self.root, self.revision)

    def test_audit_hash_and_acceptance_status_still_apply(self):
        self.protocol["owner_approved"] = False
        self.write_receipt()
        self.assertFalse(verify_protocol(self.path, self.root, self.revision)[0])
        (self.root / "syntax-audit.json").write_bytes(b"changed audit\n")
        with self.assertRaisesRegex(ValueError, "Protocol audit differs"):
            verify_protocol(self.path, self.root, self.revision)


if __name__ == "__main__":
    unittest.main()
