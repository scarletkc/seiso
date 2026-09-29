from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from scripts.release import verify_installation


class InstallationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def manifest(self, version):
        (self.root / "Cargo.toml").write_text(f'[package]\nversion = "{version}"\n')

    @patch("scripts.release.verify_installation.installed_version")
    @patch("scripts.release.verify_installation.subprocess.check_output")
    def test_python_metadata_is_normalized_but_cli_stays_semver(self, output, metadata):
        for version, normalized in [("1.2.3-alpha.10", "1.2.3a10"), ("1.2.3-beta.1", "1.2.3b1"),
                                    ("1.2.3-rc.1", "1.2.3rc1"), ("1.2.3", "1.2.3")]:
            with self.subTest(version=version):
                self.manifest(version)
                output.return_value = f"seiso {version}\n"
                metadata.return_value = normalized
                verify_installation.verify(self.root, ["seiso"], python_package=True)
                metadata.assert_called_with("seiso")
                self.assertEqual(output.call_args.args[0][-1], "--version")

    @patch("scripts.release.verify_installation.installed_version")
    @patch("scripts.release.verify_installation.subprocess.check_output", return_value="seiso 1.2.3a1\n")
    def test_cli_rejects_normalized_or_stale_version(self, output, metadata):
        self.manifest("1.2.3-alpha.1")
        for actual in ["seiso 1.2.3a1\n", "seiso 1.2.2\n"]:
            output.return_value = actual
            with self.assertRaisesRegex(ValueError, "CLI version mismatch"):
                verify_installation.verify(self.root, ["seiso"], python_package=True)
        metadata.assert_not_called()

    @patch("scripts.release.verify_installation.installed_version", return_value="1.2.3-alpha.1")
    @patch("scripts.release.verify_installation.subprocess.check_output", return_value="seiso 1.2.3-alpha.1\n")
    def test_python_metadata_must_match_normalized_version(self, output, metadata):
        self.manifest("1.2.3-alpha.1")
        with self.assertRaisesRegex(ValueError, "Python version mismatch"):
            verify_installation.verify(self.root, ["seiso"], python_package=True)

    @patch("scripts.release.verify_installation.installed_version")
    @patch("scripts.release.verify_installation.subprocess.check_output", return_value="seiso 1.2.3-rc.1\n")
    def test_npm_command_does_not_require_python_installation(self, output, metadata):
        self.manifest("1.2.3-rc.1")
        command = ["npm", "exec", "--yes", "--package", "./seiso.tgz", "--", "seiso"]
        verify_installation.verify(self.root, command)
        self.assertEqual(output.call_args.args[0][1:], [*command[1:], "--version"])
        metadata.assert_not_called()

    @patch("scripts.release.verify_installation.subprocess.check_output", side_effect=subprocess.CalledProcessError(1, "seiso"))
    def test_failed_cli_is_not_accepted(self, output):
        self.manifest("1.2.3")
        with self.assertRaises(subprocess.CalledProcessError):
            verify_installation.verify(self.root, ["seiso"])

    def cli(self, check="[]", parse='{"files": [{}], "errors": []}'):
        outputs = {"rule": "# KND001: Missing document kind\n", "check": check, "parse": parse}
        return lambda command, **kwargs: outputs[command[command.index("seiso") + 1]]

    def test_exercise_runs_rule_check_and_parse_through_the_command(self):
        command = ["npm", "exec", "--yes", "--package", "@scarletkc/seiso@1.2.3", "--", "seiso"]
        with patch("scripts.release.verify_installation.subprocess.check_output", side_effect=self.cli()) as output:
            verify_installation.exercise(self.root, command)
        self.assertEqual([call.args[0][len(command):] for call in output.call_args_list], [
            ["rule", "KND001"],
            ["check", "README.md", "--select", "KND", "--output-format", "json"],
            ["parse", "README.md", "--output-format", "json"],
        ])
        self.assertTrue(all(call.kwargs["cwd"] == self.root for call in output.call_args_list))

    def test_exercise_rejects_diagnostics_and_parse_errors(self):
        for outputs, message in [
            ({"check": '[{"code": "KND001"}]'}, "reported diagnostics"),
            ({"parse": '{"files": [{}], "errors": [{"path": "README.md"}]}'}, "did not parse"),
            ({"parse": '{"files": [], "errors": []}'}, "did not parse"),
        ]:
            with self.subTest(outputs=outputs), \
                    patch("scripts.release.verify_installation.subprocess.check_output", side_effect=self.cli(**outputs)), \
                    self.assertRaisesRegex(ValueError, message):
                verify_installation.exercise(self.root, ["seiso"])


if __name__ == "__main__":
    unittest.main()
