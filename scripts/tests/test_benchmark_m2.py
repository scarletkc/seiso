import unittest
from pathlib import Path
import subprocess
import sys
import tempfile

from scripts.evaluation.benchmark_m2 import BYTES_PER_FILE, comparison, document_bytes, summarize_samples, timed_run


class PerformanceFixtureTests(unittest.TestCase):
    def test_workload_retains_exact_templates_nearby_variants_and_byte_budget(self):
        for group in range(100):
            template = document_bytes(group, 0)
            for variant in range(10):
                content = document_bytes(group, variant)
                self.assertEqual(len(content), BYTES_PER_FILE)
                self.assertEqual(content.count(b"## Procedure"), 15)
                if variant < 5:
                    self.assertEqual(content, template)
                else:
                    self.assertNotEqual(content, template)
            self.assertNotIn(b"```", template)

    def test_timing_cannot_hide_nondeterministic_diagnostics_or_slow_medians(self):
        rows = [{"seconds": value, "output_sha256": "same", "output_bytes": 42} for value in [0.1, 0.2, 0.9]]
        self.assertTrue(summarize_samples(rows, 0.5)["target_met"])
        self.assertFalse(summarize_samples(rows, 0.2)["target_met"])
        rows[-1]["output_sha256"] = "different"
        with self.assertRaises(ValueError):
            summarize_samples(rows)

    def test_baseline_comparison_reports_each_required_mode(self):
        baseline = {"modes": {name: {"median_seconds": 1.0} for name in ["cold", "warm", "hook_warm"]}}
        current = {"modes": {name: {"median_seconds": value} for name, value in [("cold", 1.05), ("warm", 1.11), ("hook_warm", 0.1)]}}
        result = comparison(current, baseline)
        self.assertAlmostEqual(result["cold"]["change_percent"], 5.0)
        self.assertAlmostEqual(result["warm"]["change_seconds"], 0.11)
        self.assertAlmostEqual(result["hook_warm"]["change_percent"], -90.0)

    def test_watchdog_keeps_real_output_and_terminates_timed_out_process(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            result = timed_run([sys.executable, "-c", "print('diagnostic')"], root, root / "fast.txt")
            self.assertEqual(result["exit_code"], 0)
            self.assertEqual((root / "fast.txt").read_text(), "diagnostic\n")
            with self.assertRaises(subprocess.TimeoutExpired):
                timed_run([sys.executable, "-c", "import time; time.sleep(10)"], root, root / "slow.txt", timeout=0.05)


if __name__ == "__main__":
    unittest.main()
