import unittest
from pathlib import Path
import tempfile

from scripts.evaluation import summarize_m2 as summary


def row(identity="one", code="OWN002", language="en", kind="howto"):
    return {"id": identity, "code": code, "input_sha256": "source-hash", "diagnostic": {"code": code, "related": []},
            "split": "holdout", "language": language, "kind": kind}


def label(item, decision="fp"):
    return {"id": item["id"], "code": item["code"], "input_sha256": item["input_sha256"],
            "diagnostic_sha256": summary.digest(summary.encode(item["diagnostic"])), "label": decision, "reason": "Reviewed original context."}


def bundle(labels):
    return {"schema_version": 1, "report_sha256": "report-hash", "reviewer_kind": "agent", "labels": labels}


class M2SummaryTests(unittest.TestCase):
    def test_rejects_missing_duplicate_unknown_and_stale_annotations(self):
        item = row()
        rows = {item["id"]: item}
        cases = [bundle([]), bundle([label(item), label(item)]), bundle([label(row("other"))]),
                 bundle([label(item)]) | {"report_sha256": "different-report"},
                 bundle([label(item) | {"input_sha256": "different-source"}]),
                 bundle([label(item) | {"diagnostic_sha256": "different-diagnostic"}]),
                 bundle([label(item) | {"label": "maybe"}]),
                 bundle([label(item) | {"reason": ""}])]
        for candidate in cases:
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                summary.validate_labels([("labels.json", candidate)], rows, "report-hash")

    def test_precision_distinguishes_unknown_and_absent_evidence(self):
        items = [row("tp"), row("fp"), row("uncertain")]
        decisions = {item["id"]: label(item, item["id"]) for item in items}
        actual = summary.precision(items, decisions)
        self.assertEqual(actual["precision"], 0.5)
        self.assertEqual(actual["conservative_precision"], 1 / 3)
        self.assertIsNone(summary.precision([], {})["precision"])
        self.assertIsNone(summary.precision([], {})["conservative_precision"])
        unknown = summary.precision([items[2]], decisions)
        self.assertIsNone(unknown["precision"])
        self.assertEqual(unknown["conservative_precision"], 0)

    def test_reports_every_rule_zero_sample_splits_and_original_language_kind(self):
        item = row(language="ja", kind="reference")
        rows = {item["id"]: item}
        decisions, provenance = summary.validate_labels([("labels.json", bundle([label(item)]))], rows, "report-hash")
        report = summary.summarize(rows, decisions, provenance)
        self.assertEqual(set(report), set(summary.RULES))
        for code in summary.RULES:
            self.assertEqual(report[code]["status"], "preview")
            self.assertFalse(report[code]["promotion"])
            self.assertFalse(report[code]["precision_validated"])
            self.assertIsNone(report[code]["splits"]["tuning"]["precision"])
        self.assertEqual(report["DUP001"]["natural_sample_status"], "no_natural_samples")
        self.assertEqual(report["OWN002"]["splits"]["holdout"]["by_language"]["ja"]["fp"], 1)
        self.assertEqual(report["OWN002"]["splits"]["holdout"]["by_kind"]["reference"]["fp"], 1)

    def test_partial_independent_review_is_valid_but_not_double_counted(self):
        first, second = row("one"), row("two")
        rows = {item["id"]: item for item in [first, second]}
        checked, _ = summary.validate_labels([("review.json", bundle([label(first)]))], rows, "report-hash", complete=False)
        self.assertEqual(len(checked), 1)
        with self.assertRaisesRegex(ValueError, "Missing 1"):
            summary.validate_labels([("review.json", bundle([label(first)]))], rows, "report-hash")

    def test_source_verification_catches_tampered_primary_and_related_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            corpus = Path(directory)
            blobs = corpus / "data/blobs"
            blobs.mkdir(parents=True)
            primary = b"primary original"
            related = b"related original"
            (blobs / "primary").write_bytes(primary)
            (blobs / "related").write_bytes(related)
            lock = {"sources": [{"id": "source", "documents": [
                {"path": "primary.md", "git_blob": "primary", "sha256": summary.digest(primary), "bytes": len(primary)},
                {"path": "related.md", "git_blob": "related", "sha256": summary.digest(related), "bytes": len(related)},
            ]}]}
            encoded = summary.encode(lock)
            (corpus / "corpus.lock.json").write_bytes(encoded)
            item = row() | {"source": "source", "path": "primary.md", "git_blob": "primary", "input_sha256": summary.digest(primary),
                            "related_inputs": [{"path": "related.md", "git_blob": "related", "sha256": summary.digest(related)}]}
            rows = {item["id"]: item}
            summary.verify_sources(rows, corpus, summary.digest(encoded))
            for blob in ["primary", "related"]:
                original = (blobs / blob).read_bytes()
                (blobs / blob).write_bytes(b"changed")
                with self.assertRaisesRegex(ValueError, "Original source hash mismatch"):
                    summary.verify_sources(rows, corpus, summary.digest(encoded))
                (blobs / blob).write_bytes(original)


if __name__ == "__main__":
    unittest.main()
