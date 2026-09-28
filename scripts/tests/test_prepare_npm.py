import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile

from scripts.release.prepare_npm import NPM_PLATFORMS, npm_platform, prepare
from scripts.release.verify_distributions import verify
from scripts.release.versions import python_version


PLATFORM_TAGS = [
    "manylinux_2_17_x86_64.manylinux2014_x86_64",
    "manylinux_2_17_aarch64.manylinux2014_aarch64",
    "macosx_10_12_x86_64",
    "macosx_11_0_arm64",
    "win_amd64",
]


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.wheels = self.root / "wheels"
        self.output = self.root / "npm"
        self.wheels.mkdir()
        self.output.mkdir()
        (self.root / "Cargo.toml").write_text('[package]\nname = "seiso"\nversion = "0.0.0"\n', encoding="utf-8")
        (self.output / "package.json").write_text('{"name":"@scarletkc/seiso","version":"0.0.0"}', encoding="utf-8")
        (self.root / "README.md").write_text("Readme", encoding="utf-8")
        (self.root / "LICENSE").write_text("License", encoding="utf-8")

    def wheel(self, platform, version="0.0.0"):
        filename = self.wheels / f"seiso-{version}-py3-none-{platform}.whl"
        binary = "seiso.exe" if platform == "win_amd64" else "seiso"
        with zipfile.ZipFile(filename, "w") as archive:
            archive.writestr(f"seiso-{version}.dist-info/METADATA", f"Name: seiso\nVersion: {version}\nLicense-File: LICENSE\n")
            archive.writestr(f"seiso-{version}.dist-info/licenses/LICENSE", "License\n")
            archive.writestr(f"seiso-{version}.data/scripts/{binary}", platform.encode())

    def all_wheels(self, version="0.0.0", skip=()):
        for platform in PLATFORM_TAGS:
            if platform not in skip:
                self.wheel(platform, version)

    def test_wheel_platform_tags_map_to_npm_platforms(self):
        self.assertEqual([npm_platform(Path(f"seiso-0.0.0-py3-none-{tag}.whl")) for tag in PLATFORM_TAGS],
                         list(NPM_PLATFORMS))
        for tag in ["musllinux_1_2_x86_64", "macosx_10_12_universal2", "win_arm64", "any"]:
            self.assertIsNone(npm_platform(Path(f"seiso-0.0.0-py3-none-{tag}.whl")), tag)

    def test_packages_every_binary_and_its_checksum(self):
        self.all_wheels()
        prepare(self.wheels, self.output, self.root)
        manifest = json.loads((self.output / "native/manifest.json").read_text())
        self.assertEqual(manifest["version"], "0.0.0")
        self.assertEqual(sorted(manifest["sha256"]), sorted([
            "darwin-arm64/seiso", "darwin-x64/seiso", "linux-arm64/seiso", "linux-x64/seiso", "win32-x64/seiso.exe",
        ]))
        for name, checksum in manifest["sha256"].items():
            self.assertEqual(checksum, hashlib.sha256((self.output / "native" / name).read_bytes()).hexdigest())
        self.assertEqual((self.output / "README.md").read_text(), "Readme")

    def test_prerelease_wheel_metadata_is_normalized_for_npm(self):
        for stage, suffix in [("alpha", "a"), ("beta", "b"), ("rc", "rc")]:
            with self.subTest(stage=stage):
                cargo_version = f"1.2.3-{stage}.1"
                (self.root / "Cargo.toml").write_text(f'[package]\nversion = "{cargo_version}"\n')
                (self.output / "package.json").write_text(json.dumps({"name": "@scarletkc/seiso", "version": cargo_version}))
                self.assertEqual(python_version(cargo_version), f"1.2.3{suffix}1")
                for path in self.wheels.glob("*.whl"):
                    path.unlink()
                self.all_wheels(f"1.2.3{suffix}1")
                prepare(self.wheels, self.output, self.root)
                manifest = json.loads((self.output / "native/manifest.json").read_text())
                self.assertEqual(manifest["version"], cargo_version)

    def test_missing_platform_does_not_write_a_partial_package(self):
        self.all_wheels(skip=["macosx_11_0_arm64"])
        with self.assertRaisesRegex(ValueError, "missing darwin-arm64$"):
            prepare(self.wheels, self.output, self.root)
        self.assertFalse((self.output / "native").exists())

    def test_mixed_versions_and_duplicate_platforms_fail(self):
        self.all_wheels(skip=["win_amd64"])
        self.wheel("win_amd64", "0.0.1")
        with self.assertRaisesRegex(ValueError, "does not match"):
            prepare(self.wheels, self.output, self.root)
        self.wheel("manylinux_2_28_x86_64")
        with self.assertRaisesRegex(ValueError, "More than one wheel"):
            prepare(self.wheels, self.output, self.root)

    def sdist(self, include_license, version="0.0.0"):
        with tarfile.open(self.wheels / f"seiso-{version}.tar.gz", "w:gz") as archive:
            files = {"PKG-INFO": f"Name: seiso\nVersion: {version}\nLicense-File: LICENSE\n".encode()}
            if include_license:
                files["LICENSE"] = b"License\r\n"
            for name, data in files.items():
                member = tarfile.TarInfo(f"seiso-{version}/{name}")
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))

    def test_source_archive_requires_its_declared_license(self):
        self.sdist(include_license=False)
        with self.assertRaisesRegex(KeyError, "LICENSE"):
            verify(self.wheels, self.root)

    def test_source_archive_accepts_license_with_windows_newlines(self):
        self.sdist(include_license=True)
        verify(self.wheels, self.root)

    def test_wheel_and_source_archive_use_python_prerelease_version(self):
        for stage, suffix in [("alpha", "a"), ("beta", "b"), ("rc", "rc")]:
            with self.subTest(stage=stage):
                for path in self.wheels.iterdir():
                    path.unlink()
                (self.root / "Cargo.toml").write_text(f'[package]\nversion = "1.2.3-{stage}.10"\n')
                self.sdist(include_license=True, version=f"1.2.3{suffix}10")
                self.wheel("win_amd64", f"1.2.3{suffix}10")
                verify(self.wheels, self.root)

    def test_wrong_prerelease_metadata_fails_before_npm_files_are_written(self):
        (self.root / "Cargo.toml").write_text('[package]\nversion = "1.2.3-rc.1"\n')
        (self.output / "package.json").write_text('{"name":"@scarletkc/seiso","version":"1.2.3-rc.1"}')
        for version in ["1.2.3-rc.1", "1.2.3rc2", "1.2.3b1", "1.2.3"]:
            with self.subTest(version=version):
                for path in self.wheels.iterdir():
                    path.unlink()
                self.all_wheels(version)
                with self.assertRaisesRegex(ValueError, "does not match"):
                    prepare(self.wheels, self.output, self.root)
                with self.assertRaisesRegex(ValueError, "name/version"):
                    verify(self.wheels, self.root)
                self.assertFalse((self.output / "native").exists())
                self.assertFalse((self.output / "LICENSE").exists())

    def test_wrong_source_archive_version_is_rejected(self):
        (self.root / "Cargo.toml").write_text('[package]\nversion = "1.2.3-alpha.10"\n')
        self.sdist(include_license=True, version="1.2.3a9")
        with self.assertRaisesRegex(ValueError, "name/version"):
            verify(self.wheels, self.root)


if __name__ == "__main__":
    unittest.main()
