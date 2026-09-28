from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from scripts.release import github_release


class GitHubReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.git("init", "--quiet")
        self.git("config", "user.name", "Release Tests")
        self.git("config", "user.email", "tests@example.com")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "tag.gpgsign", "false")
        self.commit("Old release")
        self.git("tag", "v1.0.0")
        self.commit("Fix links")
        self.notes = self.root / "notes.md"
        self.notes.write_text("## Changes\n\nFix links.\n")
        self.assets = self.root / "dist"
        self.assets.mkdir()
        (self.assets / "seiso.tgz").write_bytes(b"test archive")

    def git(self, *args):
        return github_release.git(*args, root=self.root)

    def commit(self, message):
        (self.root / "source.txt").write_text(message)
        self.git("add", "source.txt")
        self.git("commit", "--quiet", "-m", message)

    def test_notes_combine_manual_text_and_exact_commit_range(self):
        note = self.root / "docs/release-notes/1.1.0.md"
        note.parent.mkdir(parents=True)
        note.write_text("## Better links\n\nSupports anchors.\n")
        body = github_release.compose_notes("1.1.0", "scarletkc/seiso", self.root)
        self.assertTrue(body.startswith("## Better links\n\nSupports anchors."))
        self.assertIn("Fix links", body)
        self.assertNotIn("Old release (", body)
        self.assertIn("compare/v1.0.0...v1.1.0", body)

    def test_first_release_lists_all_history(self):
        self.git("tag", "-d", "v1.0.0")
        body = github_release.compose_notes("1.1.0", "scarletkc/seiso", self.root)
        self.assertIn("Old release", body)
        self.assertIn("Fix links", body)
        self.assertNotIn("Full diff", body)

    def test_stable_notes_include_the_whole_prerelease_cycle(self):
        self.git("tag", "v1.1.0-rc.1")
        self.commit("Finish release")
        body = github_release.compose_notes("1.1.0", "scarletkc/seiso", self.root)
        self.assertIn("Changes since v1.0.0", body)
        self.assertIn("Finish release", body)
        self.assertIn("Fix links (", body)

    def test_first_prerelease_uses_stable_base_and_semver_note_filename(self):
        note = self.root / "docs/release-notes/1.1.0-alpha.1.md"
        note.parent.mkdir(parents=True)
        note.write_text("## Preview links\n\nTry anchors.\n")
        body = github_release.compose_notes("1.1.0-alpha.1", "owner/repo", self.root)
        self.assertTrue(body.startswith("## Preview links\n\nTry anchors."))
        self.assertIn("Changes since v1.0.0", body)
        self.assertIn("compare/v1.0.0...v1.1.0-alpha.1", body)

    def test_successive_prereleases_use_highest_semantic_predecessor(self):
        self.git("tag", "-a", "v1.1.0-alpha.9", "-m", "Alpha nine")
        self.git("tag", "v1.1.0-alpha.10")
        self.commit("Beta changes")
        for version, previous in [("1.1.0-alpha.11", "v1.1.0-alpha.10"),
                                  ("1.1.0-beta.1", "v1.1.0-alpha.11"),
                                  ("1.1.0-rc.1", "v1.1.0-beta.1")]:
            with self.subTest(version=version):
                self.assertEqual(github_release.comparison_base(version, self.root), previous)
                self.git("tag", f"v{version}")
        body = github_release.compose_notes("1.1.0-rc.2", "owner/repo", self.root)
        self.assertIn("Changes since v1.1.0-rc.1", body)
        self.assertNotIn("Fix links (", body)

    def test_unreachable_future_and_invalid_tags_are_excluded(self):
        self.git("branch", "side")
        self.commit("Main changes")
        head = self.git("rev-parse", "HEAD")
        self.git("checkout", "--quiet", "side")
        self.commit("Unreachable release")
        self.git("tag", "v1.1.0-alpha.10")
        self.git("checkout", "--quiet", head)
        for tag in ["v1.1.0-alpha.01", "v1.1.0-preview.1", "v2.0.0", "v1.1.0-alpha.9"]:
            self.git("tag", tag)
        body = github_release.compose_notes("1.1.0-alpha.9", "owner/repo", self.root)
        self.assertIn("Changes since v1.0.0", body)
        self.assertIn("Main changes", body)
        self.assertNotIn("Unreachable release", body)

    def test_first_prerelease_and_first_stable_without_stable_base_use_all_history(self):
        self.git("tag", "-d", "v1.0.0")
        self.git("tag", "v1.1.0-alpha.1")
        for version in ["1.1.0-alpha.1", "1.1.0"]:
            with self.subTest(version=version):
                body = github_release.compose_notes(version, "owner/repo", self.root)
                self.assertIn("Old release", body)
                self.assertIn("Fix links", body)
                self.assertNotIn("Full diff", body)

    def test_prerelease_retry_has_the_same_commit_range(self):
        before = github_release.compose_notes("1.1.0-beta.1", "owner/repo", self.root)
        self.git("tag", "v1.1.0-beta.1")
        self.assertEqual(github_release.compose_notes("1.1.0-beta.1", "owner/repo", self.root), before)

    def test_rerun_excludes_current_tag_from_previous_selection(self):
        self.git("tag", "v1.1.0")
        body = github_release.compose_notes("1.1.0", "scarletkc/seiso", self.root)
        self.assertIn("Changes since v1.0.0", body)
        self.assertIn("Fix links", body)

    def test_conflicting_tag_fails(self):
        with self.assertRaisesRegex(ValueError, "another commit"):
            github_release.compose_notes("1.0.0", "scarletkc/seiso", self.root)

    def test_note_requires_heading_and_body(self):
        for contents in ["", "# Title\n\nBody", "## Title\n\n", "##  \nBody"]:
            with self.subTest(contents=contents), self.assertRaises(ValueError):
                self.notes.write_text(contents)
                github_release.handwritten_note(self.notes)

    @patch("scripts.release.github_release.run")
    @patch("scripts.release.github_release.get_release")
    def test_default_never_queries_or_writes_github(self, lookup, run):
        github_release.publish_github("1.1.0", "owner/repo", self.notes, self.assets, root=self.root)
        lookup.assert_not_called()
        run.assert_not_called()

    @patch("scripts.release.github_release.run")
    @patch("scripts.release.github_release.get_release", return_value=None)
    def test_create_pins_commit_and_uses_notes_file(self, lookup, run):
        github_release.publish_github("1.1.0", "owner/repo", self.notes, self.assets,
                                      execute=True, root=self.root)
        args = run.call_args_list[0].args[0]
        self.assertEqual(args[:4], ["gh", "release", "create", "v1.1.0"])
        self.assertEqual(args[args.index("--target") + 1], self.git("rev-parse", "HEAD"))
        self.assertEqual(args[args.index("--notes-file") + 1], str(self.notes))
        self.assertNotIn("--prerelease", args)
        self.assertNotIn("--latest=false", args)
        self.assertEqual(run.call_args_list[1].args[0][:3], ["gh", "release", "upload"])

    @patch("scripts.release.github_release.run")
    @patch("scripts.release.github_release.get_release", return_value=None)
    def test_prerelease_is_marked_on_github(self, lookup, run):
        for stage in ["alpha", "beta", "rc"]:
            with self.subTest(stage=stage):
                run.reset_mock()
                github_release.publish_github(f"1.1.0-{stage}.1", "owner/repo", self.notes, self.assets,
                                              execute=True, root=self.root)
                self.assertIn("--prerelease", run.call_args_list[0].args[0])
                self.assertIn("--latest=false", run.call_args_list[0].args[0])

    @patch("scripts.release.github_release.run")
    @patch("scripts.release.github_release.get_release", return_value={"draft": False, "prerelease": False, "assets": [{"name": "seiso.tgz"}]})
    def test_retry_only_uploads_missing_assets(self, lookup, run):
        (self.assets / "seiso.crate").write_bytes(b"crate")
        github_release.publish_github("1.1.0", "owner/repo", self.notes, self.assets,
                                      execute=True, root=self.root)
        run.assert_called_once()
        args = run.call_args.args[0]
        self.assertEqual(args[-1], str(self.assets / "seiso.crate"))
        self.assertNotIn(str(self.assets / "seiso.tgz"), args)

    @patch("scripts.release.github_release.run")
    @patch("scripts.release.github_release.get_release", return_value={"draft": False, "prerelease": False, "assets": [{"name": "seiso.tgz"}]})
    def test_complete_retry_leaves_release_unchanged(self, lookup, run):
        github_release.publish_github("1.1.0", "owner/repo", self.notes, self.assets,
                                      execute=True, root=self.root)
        run.assert_not_called()

    @patch("scripts.release.github_release.run")
    @patch("scripts.release.github_release.get_release")
    def test_prerelease_retry_uploads_missing_attachments_then_skips_complete_release(self, lookup, run):
        lookup.return_value = {"draft": False, "prerelease": True, "assets": [{"name": "seiso.tgz"}]}
        (self.assets / "seiso.crate").write_bytes(b"crate")
        for stage in ["alpha", "beta", "rc"]:
            with self.subTest(stage=stage):
                run.reset_mock()
                lookup.return_value["assets"] = [{"name": "seiso.tgz"}]
                github_release.publish_github(f"1.1.0-{stage}.1", "owner/repo", self.notes, self.assets,
                                              execute=True, root=self.root)
                run.assert_called_once()
                self.assertEqual(run.call_args.args[0][:3], ["gh", "release", "upload"])
                self.assertEqual(run.call_args.args[0][-1], str(self.assets / "seiso.crate"))
                lookup.return_value["assets"].append({"name": "seiso.crate"})
                run.reset_mock()
                github_release.publish_github(f"1.1.0-{stage}.1", "owner/repo", self.notes, self.assets,
                                              execute=True, root=self.root)
                run.assert_not_called()

    @patch("scripts.release.github_release.run")
    @patch("scripts.release.github_release.get_release")
    def test_inconsistent_release_kind_fails_before_upload(self, lookup, run):
        for version, prerelease in [("1.1.0", True), ("1.1.0-rc.1", False)]:
            with self.subTest(version=version), self.assertRaisesRegex(ValueError, "prerelease flag"):
                lookup.return_value = {"draft": False, "prerelease": prerelease, "assets": []}
                github_release.publish_github(version, "owner/repo", self.notes, self.assets,
                                              execute=True, root=self.root)
        run.assert_not_called()

    @patch("scripts.release.github_release.run")
    @patch("scripts.release.github_release.get_release", side_effect=RuntimeError("API unavailable"))
    def test_lookup_error_never_creates_release(self, lookup, run):
        with self.assertRaises(RuntimeError):
            github_release.publish_github("1.1.0", "owner/repo", self.notes, self.assets,
                                          execute=True, root=self.root)
        run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
