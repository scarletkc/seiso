import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import unittest

from scripts.release.prepare_npm import NPM_PLATFORMS, executable


LAUNCHER = Path(__file__).resolve().parents[2] / "npm/seiso/bin/seiso.cjs"
NODE = shutil.which("node")
# Loaded before the launcher: fakes the host it inspects and records the binary it would start.
PRELOAD = """
const fs = require("node:fs");
const childProcess = require("node:child_process");
const host = JSON.parse(process.env.SEISO_TEST_HOST);
Object.defineProperty(process, "platform", { value: host.platform });
Object.defineProperty(process, "arch", { value: host.arch });
process.report.getReport = () => ({ header: host.glibc ? { glibcVersionRuntime: host.glibc } : {} });
childProcess.spawnSync = (file, args) => {
  fs.writeSync(1, JSON.stringify({ file, args }));
  return host.spawn;
};
"""


# CI must run these tests; elsewhere they need Node.js on PATH.
@unittest.skipUnless(NODE or os.environ.get("CI"), "Node.js is not installed")
class LauncherTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.modules = self.root / "node_modules/@scarletkc"
        self.launcher = self.modules / "seiso/bin/seiso.cjs"
        self.launcher.parent.mkdir(parents=True)
        shutil.copyfile(LAUNCHER, self.launcher)
        (self.root / "preload.cjs").write_text(PRELOAD, encoding="utf-8")

    def install(self, *platforms):
        for platform in platforms:
            package = self.modules / f"seiso-{platform}"
            package.mkdir()
            (package / "package.json").write_text(json.dumps({"name": f"@scarletkc/seiso-{platform}"}))
            (package / executable(platform)).write_bytes(b"")

    def run_launcher(self, platform, arch, glibc=None, spawn=None, args=("check", "--select", "KND")):
        host = {"platform": platform, "arch": arch, "glibc": glibc, "spawn": spawn or {"status": 0}}
        return subprocess.run(
            [NODE, "--require", str(self.root / "preload.cjs"), str(self.launcher), *args],
            env={**os.environ, "SEISO_TEST_HOST": json.dumps(host)},
            capture_output=True, text=True, encoding="utf-8", timeout=30,
        )

    def test_each_host_starts_its_platform_binary(self):
        self.install(*NPM_PLATFORMS)
        for platform, (system, cpu, libc) in NPM_PLATFORMS.items():
            with self.subTest(platform=platform):
                result = self.run_launcher(system, cpu, glibc="2.39" if libc == "glibc" else None)
                self.assertEqual(result.returncode, 0, result.stderr)
                started = json.loads(result.stdout)
                self.assertTrue(os.path.samefile(
                    started["file"], self.modules / f"seiso-{platform}" / executable(platform)))
                self.assertEqual(started["args"], ["check", "--select", "KND"])

    def test_missing_platform_package_names_it_and_how_to_reinstall(self):
        # Each Linux host gets only the package for the other libc, as after copying node_modules.
        for glibc, installed, missing in [("2.39", "linux-x64-musl", "linux-x64"),
                                          (None, "linux-x64", "linux-x64-musl")]:
            with self.subTest(missing=missing):
                shutil.rmtree(self.modules / f"seiso-{installed}", ignore_errors=True)
                self.install(installed)
                result = self.run_launcher("linux", "x64", glibc=glibc)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, "")
                message = " ".join(result.stderr.split())
                self.assertIn(f"@scarletkc/seiso-{missing} is not installed", message)
                self.assertIn("--omit=optional", message)
                self.assertIn("cargo install seiso", message)
                shutil.rmtree(self.modules / f"seiso-{installed}")

    def test_unsupported_host_points_to_cargo(self):
        self.install(*NPM_PLATFORMS)
        for platform, arch in [("linux", "ia32"), ("freebsd", "x64"), ("win32", "ia32")]:
            with self.subTest(host=f"{platform}-{arch}"):
                result = self.run_launcher(platform, arch)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, "")
                self.assertIn(f"no prebuilt binary for {platform}-{arch}", result.stderr)
                self.assertIn("cargo install seiso", result.stderr)

    def test_exit_status_and_start_failures(self):
        self.install("linux-x64")
        for spawn, code in [({"status": 3}, 3), ({"status": None}, 2),
                            ({"error": {"message": "spawn EACCES"}}, 2)]:
            with self.subTest(spawn=spawn):
                result = self.run_launcher("linux", "x64", glibc="2.39", spawn=spawn)
                self.assertEqual(result.returncode, code, result.stderr)
        self.assertIn("cannot start", result.stderr)
        self.assertIn("spawn EACCES", result.stderr)

    @unittest.skipIf(os.name == "nt", "Windows has no POSIX signal exit status")
    def test_binary_signal_is_raised_again(self):
        self.install("linux-x64")
        result = self.run_launcher("linux", "x64", glibc="2.39", spawn={"status": None, "signal": "SIGTERM"})
        self.assertEqual(result.returncode, -signal.SIGTERM)


if __name__ == "__main__":
    unittest.main()
