import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from scripts.ci import ci_scope


class DocumentationPathsTests(unittest.TestCase):
    def test_only_prose_paths_are_documentation(self):
        for path in ["README.md", "CONTRIBUTING.md", "LICENSE", "docs/README.md", "docs/guides/checking.md",
                     "docs/design/roadmap.md", "docs/日本語 guide.md", "corpus/README.md",
                     "corpus/docs/evaluation.md", "spec/convention.md", "spec/adopting.md",
                     "site/package.json", "site/.vitepress/config.mts"]:
            with self.subTest(path=path):
                self.assertTrue(ci_scope.is_documentation(path))
        for path in ["docs/rules/KND001.md", "docs/design/history.md", "spec/CHANGELOG.md",
                     "spec/notes.txt", "tests/fixtures/input.md",
                     "corpus/data/example.md", "src/main.rs", "Cargo.toml", "Cargo.lock",
                     "seiso.toml", "docs/seiso.toml", ".github/workflows/ci.yml",
                     "scripts/ci/ci_scope.py", "docs/image.png", "unknown.md", "unknown/file.md",
                     "docs/../src/input.md", "docs//guide.md", "/docs/guide.md", "docs\\guide.md", ""]:
            with self.subTest(path=path):
                self.assertFalse(ci_scope.is_documentation(path))

    def test_unavailable_or_malformed_diffs_select_full_ci(self):
        event = {"before": "a" * 40, "after": "b" * 40}
        for output in [b"", b"README.md", b"README.md\0\0", b"docs/\xff.md\0"]:
            with self.subTest(output=output), patch("scripts.ci.ci_scope.subprocess.run") as run:
                run.return_value.stdout = output
                self.assertFalse(ci_scope.docs_only("push", event))
        with patch("scripts.ci.ci_scope.subprocess.run", side_effect=OSError("git unavailable")):
            self.assertFalse(ci_scope.docs_only("push", event))

    def test_manual_and_invalid_events_select_full_ci_without_diffing(self):
        cases = [("workflow_dispatch", {}), ("unknown", {}), ("push", {}), ("push", None),
                 ("push", {"before": "0" * 40, "after": "a" * 40}),
                 ("push", {"before": "--invalid", "after": "a" * 40}),
                 ("push", {"before": "a" * 40, "after": None}),
                 ("pull_request", {"pull_request": {"base": None}})]
        with patch("scripts.ci.ci_scope.subprocess.run") as run:
            for name, event in cases:
                with self.subTest(name=name, event=event):
                    self.assertFalse(ci_scope.docs_only(name, event))
            run.assert_not_called()

    def test_missing_and_malformed_event_files_write_full_ci_output(self):
        with tempfile.TemporaryDirectory() as directory:
            event_path = Path(directory) / "event.json"
            output = Path(directory) / "output"
            for content in [None, "{broken", "null", "[]"]:
                if content is not None:
                    event_path.write_text(content, encoding="utf-8")
                with patch.dict(os.environ, {"GITHUB_EVENT_NAME": "push",
                                             "GITHUB_EVENT_PATH": str(event_path),
                                             "GITHUB_OUTPUT": str(output)}):
                    with contextlib.redirect_stdout(io.StringIO()):
                        ci_scope.main()
            self.assertEqual(output.read_text(encoding="utf-8"), "docs_only=false\n" * 4)


class GitChangeScopeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.name", "CI scope test")
        self.git("config", "user.email", "ci-scope@example.invalid")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "core.hooksPath", str(self.root / "no-hooks"))
        self.base = self.commit("README.md", "# Project\n")

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.root, check=True,
                              capture_output=True, text=True).stdout.strip()

    def commit(self, path=None, content="# Guide\n"):
        if path is not None:
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(content, encoding="utf-8")
        self.git("add", "-A")
        self.git("commit", "-qm", "Update files")
        return self.git("rev-parse", "HEAD")

    def push(self, base, head):
        return ci_scope.docs_only("push", {"before": base, "after": head}, self.root)

    def test_docs_push_and_github_output(self):
        head = self.commit("docs/日本語 guide.md")
        self.assertTrue(self.push(self.base, head))
        event_path = self.root / "event.json"
        event_path.write_text(json.dumps({"before": self.base, "after": head}), encoding="utf-8")
        output = self.root / "output"
        classify = ci_scope.docs_only
        with patch.dict(os.environ, {"GITHUB_EVENT_NAME": "push", "GITHUB_EVENT_PATH": str(event_path),
                                     "GITHUB_OUTPUT": str(output)}):
            with patch("scripts.ci.ci_scope.docs_only", side_effect=lambda name, event: classify(name, event, self.root)):
                with contextlib.redirect_stdout(io.StringIO()):
                    ci_scope.main()
        self.assertEqual(output.read_text(encoding="utf-8"), "docs_only=true\n")

    def test_mixed_multi_commit_push_is_full_even_when_last_commit_is_docs(self):
        self.commit("src/lib.rs", "fn main() {}\n")
        head = self.commit("docs/guide.md")
        self.assertFalse(self.push(self.base, head))

    def test_pull_request_uses_merge_base_and_keeps_prior_code_changes(self):
        self.git("checkout", "-qb", "topic")
        docs_head = self.commit("docs/guide.md")
        self.git("checkout", "main")
        main_head = self.commit("src/unrelated.rs", "// Base branch change\n")
        event = {"pull_request": {"base": {"sha": main_head}, "head": {"sha": docs_head}}}
        self.assertTrue(ci_scope.docs_only("pull_request", event, self.root))
        self.git("checkout", "topic")
        self.commit("src/lib.rs", "// PR code change\n")
        event["pull_request"]["head"]["sha"] = self.commit("docs/guide.md", "# Updated\n")
        self.assertFalse(ci_scope.docs_only("pull_request", event, self.root))

    def test_deletions_and_renames_classify_both_paths(self):
        docs = self.commit("docs/guide.md")
        (self.root / "docs/guide.md").unlink()
        deleted = self.commit()
        self.assertTrue(self.push(docs, deleted))
        embedded = self.commit("docs/rules/KND001.md")
        self.git("mv", "docs/rules/KND001.md", "docs/guide.md")
        renamed = self.commit()
        self.assertFalse(self.push(embedded, renamed))
        self.assertFalse(self.push(renamed, embedded))
        self.git("mv", "docs/guide.md", "docs/renamed.md")
        self.assertTrue(self.push(renamed, self.commit()))

    def test_empty_diff_and_unavailable_history_select_full_ci(self):
        self.assertFalse(self.push(self.base, self.base))
        self.assertFalse(self.push("a" * 40, self.base))


if __name__ == "__main__":
    unittest.main()
