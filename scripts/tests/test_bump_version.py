import json
from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest.mock import patch

from scripts.release import bump_version
from scripts.release.bump_version import next_version, plan_bump
from scripts.release.prepare_npm import optional_dependencies


class BumpVersionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "npm/seiso").mkdir(parents=True)
        (self.root / "Cargo.toml").write_text(
            '[package]\nname = "seiso"\nversion = "1.2.3"\n'
            '[dependencies]\nexternal = "1.2.3"\n')
        (self.root / "Cargo.lock").write_text(
            'version = 4\n\n[[package]]\nname = "seiso"\nversion = "1.2.3"\n'
            'dependencies = ["external 1.2.3"]\n'
            '[[package]]\nname = "external"\nversion = "1.2.3"\nsource = "registry+https://example.com"\n')
        (self.root / "npm/seiso/package.json").write_text(json.dumps(
            {"name": "@scarletkc/seiso", "version": "1.2.3", "custom": "1.2.3",
             "optionalDependencies": optional_dependencies("1.2.3")}, indent=2) + "\n")

    def test_version_modes_and_optional_v_prefix(self):
        for requested, expected in [("patch", "1.2.4"), ("minor", "1.3.0"),
                                    ("major", "2.0.0"), ("v2.3.4", "2.3.4")]:
            self.assertEqual(next_version("1.2.3", requested), expected)

    def test_invalid_equal_and_lower_versions_fail(self):
        for requested in ["1.2.3", "0.2.3", "01.3.0", "2.0", "2.0.0-rc.01", "2.0.0+build"]:
            with self.subTest(version=requested), self.assertRaises(ValueError):
                next_version("1.2.3", requested)

    def test_prerelease_ordering_and_promotion(self):
        for current, requested, expected in [
            ("1.2.3", "1.2.4-alpha.0", "1.2.4-alpha.0"),
            ("1.2.4-alpha.9", "v1.2.4-alpha.10", "1.2.4-alpha.10"),
            ("1.2.4-alpha.9", "1.2.4-beta.1", "1.2.4-beta.1"),
            ("1.2.4-beta.1", "v1.2.4-rc.1", "1.2.4-rc.1"),
            ("1.2.4-rc.2", "1.2.4-rc.10", "1.2.4-rc.10"),
            ("1.2.4-rc.1", "patch", "1.2.4"),
            ("1.2.4-alpha.10", "1.2.4", "1.2.4"),
            ("1.2.4-beta.1", "patch", "1.2.4"),
        ]:
            with self.subTest(current=current, requested=requested):
                self.assertEqual(next_version(current, requested), expected)
        for current, requested in [("1.2.3", "1.2.3-rc.1"),
                                   ("1.2.4-rc.1", "1.2.4-beta.9"),
                                   ("1.2.4-alpha.10", "1.2.4-alpha.9"),
                                   ("1.2.4-alpha.2", "1.2.4-alpha.1")]:
            with self.subTest(current=current, requested=requested), self.assertRaisesRegex(ValueError, "greater"):
                next_version(current, requested)

    def test_plan_updates_package_without_touching_external_versions(self):
        changes = plan_bump(self.root, "1.2.3", "1.2.4", "Fix links")
        manifest = tomllib.loads(changes[self.root / "Cargo.toml"])
        self.assertEqual(manifest["package"]["version"], "1.2.4")
        self.assertEqual(manifest["dependencies"]["external"], "1.2.3")
        lock = tomllib.loads(changes[self.root / "Cargo.lock"])["package"]
        self.assertEqual([p["version"] for p in lock], ["1.2.4", "1.2.3"])
        self.assertEqual(lock[0]["dependencies"], ["external 1.2.3"])
        npm = json.loads(changes[self.root / "npm/seiso/package.json"])
        self.assertEqual(npm["custom"], "1.2.3")
        self.assertEqual(npm["optionalDependencies"], optional_dependencies("1.2.4"))
        self.assertEqual(changes[self.root / "docs/release-notes/1.2.4.md"], "## Fix links\n\n")
        self.assertIn('version = "1.2.3"', (self.root / "Cargo.toml").read_text())

    def test_existing_note_is_preserved_and_no_versions_are_written(self):
        note = self.root / "docs/release-notes/1.2.4.md"
        note.parent.mkdir(parents=True)
        note.write_text("## Existing\n\nKeep this.\n")
        before = {p: p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
        with self.assertRaisesRegex(ValueError, "already exists"):
            plan_bump(self.root, "1.2.3", "1.2.4", "New title")
        self.assertEqual(before, {p: p.read_bytes() for p in before})

    def test_bad_note_title_fails_before_any_write(self):
        for note in ["", "   ", "Title\nSecond line"]:
            with self.subTest(note=note), self.assertRaisesRegex(ValueError, "single-line"):
                plan_bump(self.root, "1.2.3", "1.2.4", note)

    def test_missing_lock_package_is_rejected(self):
        (self.root / "Cargo.lock").write_text('version = 4\n')
        with self.assertRaisesRegex(ValueError, "must contain"):
            plan_bump(self.root, "1.2.3", "1.2.4")

    def test_prerelease_bump_plans_canonical_versions_and_note_filename(self):
        for stage in ["alpha", "beta", "rc"]:
            with self.subTest(stage=stage):
                version = next_version("1.2.3", f"v1.2.4-{stage}.1")
                changes = plan_bump(self.root, "1.2.3", version, "Preview")
                self.assertEqual(tomllib.loads(changes[self.root / "Cargo.toml"])["package"]["version"], version)
                self.assertEqual(tomllib.loads(changes[self.root / "Cargo.lock"])["package"][0]["version"], version)
                self.assertIn(f'"version": "{version}"', changes[self.root / "npm/seiso/package.json"])
                self.assertEqual(changes[self.root / f"docs/release-notes/{version}.md"], "## Preview\n\n")

    def test_platform_pin_drift_fails_before_any_write(self):
        path = self.root / "npm/seiso/package.json"
        package = json.loads(path.read_text())
        package["optionalDependencies"]["@scarletkc/seiso-linux-x64-musl"] = "1.2.2"
        path.write_text(json.dumps(package))
        with self.assertRaisesRegex(ValueError, "optionalDependencies"):
            plan_bump(self.root, "1.2.3", "1.2.4", "Fix links")
        self.assertFalse((self.root / "docs/release-notes").exists())

    @patch("scripts.release.bump_version.release_metadata")
    def test_invalid_or_lower_request_does_not_write_any_file(self, metadata):
        metadata.return_value = "1.2.3"
        before = {p: p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
        for version in ["1.2.3-alpha.10", "1.2.2", "1.2.3", "1.3.0-beta.01", "1.3.0-dev.1"]:
            with self.subTest(version=version), patch("scripts.release.bump_version.ROOT", self.root), \
                    patch("sys.argv", ["scripts.release.bump_version.py", version]), self.assertRaises(ValueError):
                bump_version.main()
        self.assertEqual(before, {p: p.read_bytes() for p in before})

    def test_version_drift_leaves_every_file_unchanged(self):
        for name in ["Cargo.toml", "Cargo.lock", "npm/seiso/package.json"]:
            with self.subTest(name=name):
                path = self.root / name
                original = path.read_text()
                path.write_text(original.replace("1.2.3", "1.2.4-rc.1"))
                before = {p: p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
                with self.assertRaises(ValueError):
                    plan_bump(self.root, "1.2.3", "1.3.0-alpha.1", "Preview")
                self.assertEqual(before, {p: p.read_bytes() for p in before})
                self.assertFalse((self.root / "docs/release-notes").exists())
                path.write_text(original)


if __name__ == "__main__":
    unittest.main()
