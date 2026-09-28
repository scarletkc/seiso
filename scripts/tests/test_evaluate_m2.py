import copy
import hashlib
from pathlib import Path
import tempfile
import unittest

from scripts.evaluation import evaluate_m2


class M2EvaluationTests(unittest.TestCase):
    def test_reverses_all_input_dimensions_without_mutating_original(self):
        original = {"sources": [{"id": name, "config": "preview=true", "documents": [1, 2, 3], "entries": [4, 5]} for name in ["first", "second"]]}
        saved = copy.deepcopy(original)
        reversed_batch = evaluate_m2.reverse_inputs(original)
        self.assertEqual(reversed_batch["sources"][0]["id"], "second")
        self.assertEqual(reversed_batch["sources"][0]["documents"], [3, 2, 1])
        self.assertEqual(reversed_batch["sources"][0]["entries"], [5, 4])
        self.assertEqual(original, saved)
        self.assertEqual(evaluate_m2.reverse_inputs(reversed_batch), original)

    def test_verifies_original_byte_size_sha256_and_git_blob(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "blob"
            raw = "# 原文\n".encode()
            path.write_bytes(raw)
            record = {"bytes": len(raw), "sha256": evaluate_m2.digest(raw), "git_blob": hashlib.sha1(f"blob {len(raw)}\0".encode() + raw).hexdigest()}
            evaluate_m2.verify_blob(path, record)
            for field, value in [("bytes", 1), ("sha256", "0" * 64), ("git_blob", "0" * 40)]:
                with self.assertRaises(ValueError):
                    evaluate_m2.verify_blob(path, record | {field: value})
            path.write_bytes(b"changed")
            with self.assertRaises(ValueError):
                evaluate_m2.verify_blob(path, record)

    def test_every_m2_rule_has_a_count_even_when_no_diagnostics_exist(self):
        all_counts, m2_counts = evaluate_m2.counts([{"split": "tuning", "code": "LNK001"}, {"split": "holdout", "code": "OWN002"}])
        self.assertEqual(all_counts["tuning"], {"LNK001": 1})
        for split in ["tuning", "holdout"]:
            self.assertEqual(set(m2_counts[split]), set(evaluate_m2.M2_RULES))
        self.assertEqual(m2_counts["holdout"]["OWN002"], 1)
        self.assertEqual(m2_counts["holdout"]["DUP001"], 0)

    def test_existing_freeze_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "Preserve the existing run"):
                evaluate_m2.run(Path(directory))


if __name__ == "__main__":
    unittest.main()
