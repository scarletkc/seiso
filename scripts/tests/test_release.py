import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch
from urllib.error import HTTPError, URLError
from urllib.parse import unquote

from scripts.release import release
from scripts.release.prepare_npm import NPM_PLATFORMS, optional_dependencies


NPM_PACKAGES = [*(f"@scarletkc/seiso-{platform}" for platform in NPM_PLATFORMS), "@scarletkc/seiso"]


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "npm/seiso").mkdir(parents=True)
        (self.root / "Cargo.toml").write_text('[package]\nname = "seiso"\nversion = "1.2.3"\n')
        (self.root / "Cargo.lock").write_text(
            '[[package]]\nname = "seiso"\nversion = "1.2.3"\n')
        (self.root / "npm/seiso/package.json").write_text(json.dumps(
            {"name": "@scarletkc/seiso", "version": "1.2.3", "optionalDependencies": optional_dependencies("1.2.3")}))
        (self.root / "pyproject.toml").write_text(
            '[project]\nname = "seiso"\ndynamic = ["version"]\n'
            '[tool.maturin]\nmanifest-path = "Cargo.toml"\n')
        self.metadata = {
            "workspace_members": ["cli"],
            "packages": [
                {"id": "cli", "name": "seiso", "version": "1.2.3", "publish": None,
                 "manifest_path": str(self.root / "Cargo.toml"), "dependencies": []},
            ],
        }

    def test_consistent_versions(self):
        version = release.validate(self.root, self.metadata)
        self.assertEqual(version, "1.2.3")

    def test_extra_workspace_packages_do_not_control_seiso_publication(self):
        self.metadata["workspace_members"].append("lib")
        self.metadata["packages"].append({"id": "lib", "name": "internal", "version": "9.0.0", "publish": []})
        self.assertEqual(release.validate(self.root, self.metadata), "1.2.3")

    def test_local_dependency_publishability_is_left_to_cargo_package(self):
        self.metadata["packages"][0]["dependencies"] = [
            {"name": "internal", "req": "^2.0.0", "path": "internal"}]
        self.assertEqual(release.validate(self.root, self.metadata), "1.2.3")

    def test_npm_and_lock_drift_fail(self):
        for path in ["npm/seiso/package.json", "Cargo.lock"]:
            with self.subTest(path=path):
                file = self.root / path
                original = file.read_text()
                file.write_text(original.replace("1.2.3", "1.2.4"))
                with self.assertRaises(ValueError):
                    release.validate(self.root, self.metadata)
                file.write_text(original)

    def test_npm_platform_pin_drift_fails(self):
        file = self.root / "npm/seiso/package.json"
        pins = optional_dependencies("1.2.3")
        for changed in [{**pins, "@scarletkc/seiso-linux-x64-musl": "1.2.2"},
                        {name: version for name, version in pins.items() if "win32" not in name}]:
            with self.subTest(pins=changed):
                file.write_text(json.dumps({"name": "@scarletkc/seiso", "version": "1.2.3", "optionalDependencies": changed}))
                with self.assertRaisesRegex(ValueError, "optionalDependencies"):
                    release.validate(self.root, self.metadata)

    def test_crate_version_drift_fails(self):
        self.metadata["packages"][0]["version"] = "1.2.4"
        with self.assertRaisesRegex(ValueError, "manifest/lock"):
            release.validate(self.root, self.metadata)

    def test_crates_publish_flag_only_applies_to_crates_release(self):
        for publish in [[], ["private"]]:
            with self.subTest(publish=publish):
                self.metadata["packages"][0]["publish"] = publish
                self.assertEqual(release.validate(self.root, self.metadata), "1.2.3")
                with self.assertRaisesRegex(ValueError, "crates.io"):
                    release.validate(self.root, self.metadata, crates=True)

    def test_python_static_version_fails(self):
        file = self.root / "pyproject.toml"
        file.write_text(file.read_text().replace('dynamic = ["version"]', 'version = "1.2.3"'))
        with self.assertRaisesRegex(ValueError, "PyPI"):
            release.validate(self.root, self.metadata)

    def test_prerelease_passes_consistency_validation(self):
        previous = "1.2.3"
        for version in ["1.2.3-alpha.1", "1.2.3-beta.1", "1.2.3-rc.1"]:
            with self.subTest(version=version):
                for name in ["Cargo.toml", "Cargo.lock", "npm/seiso/package.json"]:
                    path = self.root / name
                    path.write_text(path.read_text().replace(previous, version))
                self.metadata["packages"][0]["version"] = version
                self.assertEqual(release.validate(self.root, self.metadata), version)
                previous = version

    @patch("scripts.release.release.run")
    @patch("scripts.release.release.crate_exists")
    def test_default_crates_verifies_archive_without_publishing(self, exists, run):
        release.publish_crates("1.2.3")
        exists.assert_not_called()
        args = run.call_args.args[0]
        self.assertEqual(args[:2], ["cargo", "package"])
        self.assertEqual(args[args.index("--package") + 1], "seiso")
        self.assertIn("--locked", args)
        self.assertNotIn("--no-verify", args)

    @patch("scripts.release.release.run")
    @patch("scripts.release.release.crate_exists", return_value=False)
    def test_missing_crate_version_publishes_only_seiso(self, exists, run):
        release.publish_crates("1.2.3", execute=True)
        args = run.call_args.args[0]
        exists.assert_called_once_with("seiso", "1.2.3")
        self.assertEqual(args[args.index("--package") + 1], "seiso")
        self.assertNotIn("--workspace", args)
        self.assertEqual(args[:2], ["cargo", "publish"])
        self.assertNotIn("--dry-run", args)

    @patch("scripts.release.release.run")
    @patch("scripts.release.release.crate_exists", return_value=True)
    def test_complete_crates_retry_does_not_upload(self, exists, run):
        for version in ["1.2.3", "1.2.3-alpha.1", "1.2.3-beta.1", "1.2.3-rc.1"]:
            release.publish_crates(version, execute=True)
            exists.assert_called_with("seiso", version)
        run.assert_not_called()

    @patch("scripts.release.release.run")
    @patch("scripts.release.release.crate_exists", side_effect=URLError("offline"))
    def test_registry_outage_does_not_start_cargo_publish(self, exists, run):
        with self.assertRaises(URLError):
            release.publish_crates("1.2.3", execute=True)
        run.assert_not_called()

    @patch("scripts.release.release.run", side_effect=subprocess.CalledProcessError(101, "cargo"))
    @patch("scripts.release.release.crate_exists", return_value=False)
    def test_publish_error_is_not_treated_as_duplicate(self, exists, run):
        with self.assertRaises(subprocess.CalledProcessError):
            release.publish_crates("1.2.3", execute=True)

    @patch("scripts.release.release.registry_text")
    def test_sparse_index_exact_versions_and_yanks(self, read):
        read.return_value = '{"vers":"1.2.2","yanked":false}\n'
        self.assertFalse(release.crate_exists("seiso", "1.2.3"))
        read.return_value += '{"vers":"1.2.3","yanked":false}\n'
        self.assertTrue(release.crate_exists("seiso", "1.2.3"))
        read.assert_called_with("https://index.crates.io/se/is/seiso")
        read.return_value = '{"vers":"1.2.3","yanked":true}\n'
        with self.assertRaisesRegex(ValueError, "yanked"):
            release.crate_exists("seiso", "1.2.3")
        read.return_value = ""
        with self.assertRaisesRegex(ValueError, "Empty"):
            release.crate_exists("seiso", "1.2.3")

    @patch("scripts.release.release.urlopen")
    def test_only_http_404_means_absent(self, open_url):
        for status in [401, 403, 429, 500, 503]:
            with self.subTest(status=status):
                open_url.side_effect = HTTPError("https://example.com", status, "error", {}, None)
                self.addCleanup(open_url.side_effect.close)
                with self.assertRaises(HTTPError):
                    release.registry_text("https://example.com")
        open_url.side_effect = HTTPError("https://example.com", 404, "absent", {}, None)
        self.addCleanup(open_url.side_effect.close)
        self.assertIsNone(release.registry_text("https://example.com"))

    def npm_archive(self, name, version="1.2.3", filename=None):
        filename = self.root / (filename or f"{name[1:].replace('/', '-')}.tgz")
        with tarfile.open(filename, "w:gz") as archive:
            data = json.dumps({"name": name, "version": version}).encode()
            member = tarfile.TarInfo("package/package.json")
            member.size = len(data)
            archive.addfile(member, io.BytesIO(data))
        return filename

    def npm_archives(self, version="1.2.3"):
        return [self.npm_archive(name, version) for name in NPM_PACKAGES]

    def published(self, run):
        return [call.args[0][2] for call in run.call_args_list]

    @patch("scripts.release.release.run")
    @patch("scripts.release.release.registry_text")
    def test_npm_default_packs_every_archive_without_registry_or_upload(self, read, run):
        archives = self.npm_archives()
        release.publish_npm("1.2.3", self.root)
        read.assert_not_called()
        self.assertEqual(self.published(run), [str(archive) for archive in archives])
        for call in run.call_args_list:
            self.assertEqual(call.args[0][:2], ["npm", "pack"])
            self.assertIn("--dry-run", call.args[0])

    @patch("scripts.release.release.run")
    @patch("scripts.release.release.registry_text")
    def test_npm_retry_skips_exact_versions(self, read, run):
        def existing(url):
            *_, name, version = url.split("/")
            return json.dumps({"name": unquote(name), "version": version})

        read.side_effect = existing
        for version in ["1.2.3", "1.2.3-alpha.1", "1.2.3-beta.1", "1.2.3-rc.1"]:
            with self.subTest(version=version):
                self.npm_archives(version)
                release.publish_npm(version, self.root, execute=True)
                read.assert_called_with(f"https://registry.npmjs.org/%40scarletkc%2Fseiso/{version}")
        run.assert_not_called()

    @patch("scripts.release.release.run")
    @patch("scripts.release.release.registry_text")
    def test_partial_npm_retry_uploads_only_missing_packages(self, read, run):
        archives = self.npm_archives()
        read.side_effect = lambda url: (json.dumps({"name": unquote(url.split("/")[-2]), "version": "1.2.3"})
                                        if "darwin" in url or "win32" in url else None)
        release.publish_npm("1.2.3", self.root, execute=True)
        missing = [str(archive) for name, archive in zip(NPM_PACKAGES, archives)
                   if "darwin" not in name and "win32" not in name]
        self.assertEqual(self.published(run), missing)

    @patch("scripts.release.release.run")
    @patch("scripts.release.release.registry_text", return_value=None)
    def test_missing_npm_version_publishes_platform_packages_before_the_main_package(self, read, run):
        archives = self.npm_archives()
        release.publish_npm("1.2.3", self.root, execute=True)
        self.assertEqual(self.published(run), [str(archive) for archive in archives])
        self.assertEqual(self.published(run)[-1], str(self.root / "scarletkc-seiso.tgz"))
        for call in run.call_args_list:
            args = call.args[0]
            self.assertEqual(args[:2], ["npm", "publish"])
            self.assertNotIn("--dry-run", args)
            self.assertIn("public", args)
            self.assertEqual(args[args.index("--tag") + 1], "latest")

    @patch("scripts.release.release.run", side_effect=[None, subprocess.CalledProcessError(1, "npm")])
    @patch("scripts.release.release.registry_text", return_value=None)
    def test_failed_platform_upload_stops_before_the_main_package(self, read, run):
        self.npm_archives()
        with self.assertRaises(subprocess.CalledProcessError):
            release.publish_npm("1.2.3", self.root, execute=True)
        self.assertEqual(run.call_count, 2)

    @patch("scripts.release.release.run")
    @patch("scripts.release.release.registry_text", return_value=None)
    def test_prerelease_npm_does_not_replace_latest(self, read, run):
        for stage in ["alpha", "beta", "rc"]:
            with self.subTest(stage=stage):
                version = f"1.2.3-{stage}.1"
                self.npm_archives(version)
                release.publish_npm(version, self.root, execute=True)
                for call in run.call_args_list[-len(NPM_PACKAGES):]:
                    args = call.args[0]
                    self.assertEqual(args[args.index("--tag") + 1], stage)
                    self.assertNotIn("latest", args)

    @patch("scripts.release.release.run")
    @patch("scripts.release.release.registry_text")
    def test_invalid_version_fails_before_registry_access_or_upload(self, read, run):
        for version in ["1.2.3-beta.01", "1.2.3-preview.1", "v1.2.3-alpha.1"]:
            with self.subTest(version=version):
                self.npm_archives(version)
                with self.assertRaises(ValueError):
                    release.publish_npm(version, self.root, execute=True)
                with self.assertRaises(ValueError):
                    release.publish_crates(version, execute=True)
        read.assert_not_called()
        run.assert_not_called()

    @patch("scripts.release.release.run")
    @patch("scripts.release.release.registry_text", return_value=None)
    def test_wrong_npm_archive_version_fails_before_upload(self, read, run):
        self.npm_archives()
        self.npm_archive("@scarletkc/seiso-win32-arm64", "1.2.4")
        with self.assertRaisesRegex(ValueError, "version differs"):
            release.publish_npm("1.2.3", self.root, execute=True)
        run.assert_not_called()

    @patch("scripts.release.release.run")
    @patch("scripts.release.release.registry_text", return_value=None)
    def test_missing_extra_or_duplicate_npm_archives_fail_before_upload(self, read, run):
        cases = [
            ("missing", lambda: (self.root / "scarletkc-seiso-linux-arm64-musl.tgz").unlink(), "exactly"),
            ("extra", lambda: self.npm_archive("@scarletkc/seiso-linux-ia32"), "exactly"),
            ("duplicate", lambda: self.npm_archive("@scarletkc/seiso", filename="old.tgz"), "More than one"),
        ]
        for label, change, message in cases:
            with self.subTest(label):
                for archive in self.root.glob("*.tgz"):
                    archive.unlink()
                self.npm_archives()
                change()
                with self.assertRaisesRegex(ValueError, message):
                    release.publish_npm("1.2.3", self.root, execute=True)
        run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
