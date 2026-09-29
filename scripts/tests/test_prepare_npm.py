import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile

from scripts.release.prepare_npm import NPM_PLATFORMS, npm_platform, optional_dependencies, prepare
from scripts.release.verify_distributions import verify
from scripts.release.versions import python_version


# Wheel platform tags in NPM_PLATFORMS order.
PLATFORM_TAGS = [
    "macosx_11_0_arm64",
    "macosx_10_12_x86_64",
    "manylinux_2_17_aarch64.manylinux2014_aarch64",
    "musllinux_1_2_aarch64",
    "manylinux_2_17_x86_64.manylinux2014_x86_64",
    "musllinux_1_2_x86_64",
    "win_arm64",
    "win_amd64",
]


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.wheels = self.root / "wheels"
        self.npm = self.root / "npm"
        self.output = self.npm / "seiso"
        self.wheels.mkdir()
        self.output.mkdir(parents=True)
        self.version("0.0.0")
        (self.root / "README.md").write_text("Readme", encoding="utf-8")
        (self.root / "LICENSE").write_text("License", encoding="utf-8")

    def version(self, version, pins=None):
        (self.root / "Cargo.toml").write_text(f'[package]\nname = "seiso"\nversion = "{version}"\n', encoding="utf-8")
        (self.output / "package.json").write_text(json.dumps({
            "name": "@scarletkc/seiso", "version": version, "license": "MIT", "author": "scarletkc",
            "repository": {"type": "git", "url": "git+https://github.com/scarletkc/seiso.git", "directory": "npm/seiso"},
            "homepage": "https://github.com/scarletkc/seiso#readme", "bugs": "https://github.com/scarletkc/seiso/issues",
            "optionalDependencies": optional_dependencies(version) if pins is None else pins,
        }), encoding="utf-8")

    def wheel(self, platform, version="0.0.0"):
        filename = self.wheels / f"seiso-{version}-py3-none-{platform}.whl"
        binary = "seiso.exe" if platform.startswith("win_") else "seiso"
        with zipfile.ZipFile(filename, "w") as archive:
            archive.writestr(f"seiso-{version}.dist-info/METADATA", f"Name: seiso\nVersion: {version}\nLicense-File: LICENSE\n")
            archive.writestr(f"seiso-{version}.dist-info/licenses/LICENSE", "License\n")
            archive.writestr(f"seiso-{version}.data/scripts/{binary}", platform.encode())

    def all_wheels(self, version="0.0.0", skip=()):
        for platform in PLATFORM_TAGS:
            if platform not in skip:
                self.wheel(platform, version)

    def platform_packages(self):
        return sorted(path.name for path in self.npm.glob("seiso-*"))

    def test_wheel_platform_tags_map_to_npm_platforms(self):
        self.assertEqual([npm_platform(Path(f"seiso-0.0.0-py3-none-{tag}.whl")) for tag in PLATFORM_TAGS],
                         list(NPM_PLATFORMS))
        for tag in ["linux_x86_64", "macosx_10_12_universal2", "win32", "any"]:
            self.assertIsNone(npm_platform(Path(f"seiso-0.0.0-py3-none-{tag}.whl")), tag)

    def test_writes_one_package_per_platform_with_its_binary(self):
        self.all_wheels()
        prepare(self.wheels, self.npm, self.root)
        self.assertEqual(self.platform_packages(), sorted(f"seiso-{platform}" for platform in NPM_PLATFORMS))
        for tag, (platform, (system, cpu, libc)) in zip(PLATFORM_TAGS, NPM_PLATFORMS.items()):
            with self.subTest(platform=platform):
                directory = self.npm / f"seiso-{platform}"
                package = json.loads((directory / "package.json").read_text())
                binary = "seiso.exe" if system == "win32" else "seiso"
                self.assertEqual(package["name"], f"@scarletkc/seiso-{platform}")
                self.assertEqual(package["version"], "0.0.0")
                self.assertEqual((package["os"], package["cpu"], package.get("libc")),
                                 ([system], [cpu], [libc] if libc else None))
                self.assertEqual(package["files"], [binary])
                self.assertNotIn("bin", package)
                self.assertEqual(package["repository"],
                                 {"type": "git", "url": "git+https://github.com/scarletkc/seiso.git"})
                self.assertEqual((directory / binary).read_bytes(), tag.encode())
                self.assertEqual((directory / "LICENSE").read_text(), "License")
                self.assertIn("@scarletkc/seiso", (directory / "README.md").read_text())
        self.assertEqual((self.output / "README.md").read_text(), "Readme")
        self.assertEqual((self.output / "LICENSE").read_text(), "License")

    def test_stale_platform_packages_are_replaced(self):
        stale = self.npm / "seiso-linux-ia32"
        stale.mkdir()
        (self.npm / "seiso-linux-x64").mkdir()
        (self.npm / "seiso-linux-x64/old").write_text("old")
        self.all_wheels()
        prepare(self.wheels, self.npm, self.root)
        self.assertFalse(stale.exists())
        self.assertFalse((self.npm / "seiso-linux-x64/old").exists())

    def test_main_package_must_pin_every_platform_package(self):
        self.all_wheels()
        pins = optional_dependencies("0.0.0")
        for changed in [{**pins, "@scarletkc/seiso-win32-arm64": "0.0.1"},
                        {name: version for name, version in pins.items() if not name.endswith("-musl")},
                        {}]:
            with self.subTest(pins=changed):
                self.version("0.0.0", pins=changed)
                with self.assertRaisesRegex(ValueError, "optionalDependencies"):
                    prepare(self.wheels, self.npm, self.root)
                self.assertEqual(self.platform_packages(), [])

    def test_prerelease_wheel_metadata_is_normalized_for_npm(self):
        for stage, suffix in [("alpha", "a"), ("beta", "b"), ("rc", "rc")]:
            with self.subTest(stage=stage):
                cargo_version = f"1.2.3-{stage}.1"
                self.version(cargo_version)
                self.assertEqual(python_version(cargo_version), f"1.2.3{suffix}1")
                for path in self.wheels.glob("*.whl"):
                    path.unlink()
                self.all_wheels(f"1.2.3{suffix}1")
                prepare(self.wheels, self.npm, self.root)
                package = json.loads((self.npm / "seiso-linux-x64-musl/package.json").read_text())
                self.assertEqual(package["version"], cargo_version)

    def test_missing_platform_does_not_write_a_partial_package(self):
        for skipped, platform in [("musllinux_1_2_aarch64", "linux-arm64-musl"), ("win_arm64", "win32-arm64")]:
            with self.subTest(platform=platform):
                for path in self.wheels.glob("*.whl"):
                    path.unlink()
                self.all_wheels(skip=[skipped])
                with self.assertRaisesRegex(ValueError, f"missing {platform}$"):
                    prepare(self.wheels, self.npm, self.root)
                self.assertEqual(self.platform_packages(), [])
                self.assertFalse((self.output / "LICENSE").exists())

    def test_mixed_versions_and_duplicate_platforms_fail(self):
        self.all_wheels(skip=["win_amd64"])
        self.wheel("win_amd64", "0.0.1")
        with self.assertRaisesRegex(ValueError, "does not match"):
            prepare(self.wheels, self.npm, self.root)
        self.wheel("musllinux_1_1_x86_64")
        with self.assertRaisesRegex(ValueError, "More than one wheel supplies linux-x64-musl"):
            prepare(self.wheels, self.npm, self.root)

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
        self.version("1.2.3-rc.1")
        for version in ["1.2.3-rc.1", "1.2.3rc2", "1.2.3b1", "1.2.3"]:
            with self.subTest(version=version):
                for path in self.wheels.iterdir():
                    path.unlink()
                self.all_wheels(version)
                with self.assertRaisesRegex(ValueError, "does not match"):
                    prepare(self.wheels, self.npm, self.root)
                with self.assertRaisesRegex(ValueError, "name/version"):
                    verify(self.wheels, self.root)
                self.assertEqual(self.platform_packages(), [])
                self.assertFalse((self.output / "LICENSE").exists())

    def test_wrong_source_archive_version_is_rejected(self):
        (self.root / "Cargo.toml").write_text('[package]\nversion = "1.2.3-alpha.10"\n')
        self.sdist(include_license=True, version="1.2.3a9")
        with self.assertRaisesRegex(ValueError, "name/version"):
            verify(self.wheels, self.root)


if __name__ == "__main__":
    unittest.main()
