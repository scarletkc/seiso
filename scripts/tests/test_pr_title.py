import contextlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from scripts.ci import pr_title


class TitleTests(unittest.TestCase):
    def test_conventional_headers_pass(self):
        for title in ["feat: add a rule for unlabeled code blocks",
                      "fix(rules): LNK002 misses anchors from HTML id attributes",
                      "feat(config)!: rename the include key",
                      "docs: describe `seiso check` exit codes",
                      "build(release-scripts): bump maturin to 1.15",
                      "revert: restore the previous cache format",
                      "ci: " + "x" * (pr_title.MAX_LENGTH - 4)]:
            with self.subTest(title=title):
                self.assertEqual(pr_title.problems(title), [])

    def test_each_violation_is_reported(self):
        cases = {
            "Add a rule": "Use the format",
            "feat:add a rule": "Use the format",
            "feat!(rules): add a rule": "Use the format",
            "feat: add\n::error::injected": "Use the format",
            "Feat: add a rule": "Replace the type 'Feat'",
            "feature: add a rule": "Replace the type 'feature'",
            "feat(Rules): add a rule": "Write the scope",
            "feat(): add a rule": "Write the scope",
            "feat(rules--cache): add a rule": "Write the scope",
            "feat: Add a rule": "Start the description with a lowercase word",
            "feat:  add a rule": "Put exactly one space",
            "feat: ": "Put exactly one space",
            "feat: add a rule.": "Remove the trailing period",
            "ci: " + "x" * (pr_title.MAX_LENGTH - 3): "Shorten the title",
        }
        for title, message in cases.items():
            with self.subTest(title=title):
                errors = pr_title.problems(title)
                self.assertEqual(len(errors), 1, errors)
                self.assertIn(message, errors[0])

    def test_independent_violations_are_reported_together(self):
        errors = pr_title.problems("Feat(Rules): Add a rule.")
        self.assertEqual(len(errors), 4, errors)


class MainTests(unittest.TestCase):
    def run_main(self, argv, env):
        output = io.StringIO()
        with patch.dict(os.environ, env, clear=True), contextlib.redirect_stdout(output):
            code = pr_title.main(argv)
        return code, output.getvalue()

    def test_reads_the_pull_request_event_and_annotates_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            event = Path(directory) / "event.json"
            event.write_text(json.dumps({"pull_request": {"title": "Add a rule"}}), encoding="utf-8")
            code, output = self.run_main([], {"GITHUB_EVENT_PATH": str(event), "GITHUB_ACTIONS": "true"})
        self.assertEqual(code, 1)
        self.assertIn("::error title=PR title::Use the format", output)
        self.assertIn(pr_title.GUIDE, output)

    def test_title_argument_reports_plain_errors_and_success(self):
        code, output = self.run_main(["feat: Add a rule"], {})
        self.assertEqual(code, 1)
        self.assertIn("error: Start the description", output)
        self.assertNotIn("::error", output)
        code, output = self.run_main(["feat: add a rule"], {})
        self.assertEqual((code, output), (0, "Title follows Conventional Commits: feat: add a rule\n"))

    def test_printed_title_cannot_start_a_workflow_command(self):
        code, output = self.run_main(["feat: add\n::add-mask::secret"], {"GITHUB_ACTIONS": "true"})
        self.assertEqual(code, 1)
        self.assertFalse(any(line.startswith("::add-mask") for line in output.splitlines()))

    def test_missing_title_source_is_a_usage_error(self):
        output = io.StringIO()
        with patch.dict(os.environ, {}, clear=True), contextlib.redirect_stderr(output):
            self.assertEqual(pr_title.main([]), 2)
        self.assertIn("usage:", output.getvalue())


if __name__ == "__main__":
    unittest.main()
