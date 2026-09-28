import unittest

from scripts.release.versions import parse_version, python_version, version_key


class VersionTests(unittest.TestCase):
    def test_python_versions_preserve_numeric_counters(self):
        for stage, suffix in [("alpha", "a"), ("beta", "b"), ("rc", "rc")]:
            for number in [0, 1, 9, 10]:
                with self.subTest(stage=stage, number=number):
                    self.assertEqual(python_version(f"1.2.3-{stage}.{number}"), f"1.2.3{suffix}{number}")
        self.assertEqual(python_version("1.2.3"), "1.2.3")

    def test_semantic_precedence(self):
        ordered = ["1.2.3", "1.2.4-alpha.0", "1.2.4-alpha.9", "1.2.4-alpha.10",
                   "1.2.4-beta.0", "1.2.4-beta.10", "1.2.4-rc.0", "1.2.4-rc.10",
                   "1.2.4", "1.2.10", "1.10.0", "2.0.0-alpha.0", "2.0.0"]
        self.assertEqual(sorted(reversed(ordered), key=version_key), ordered)

    def test_noncanonical_or_unsupported_versions_fail(self):
        for version in ["", "1.2", "01.2.3", "1.02.3", "1.2.03", "1.2.3-alpha",
                        "1.2.3-alpha.01", "1.2.3-beta.-1", "1.2.3-rc.1.2",
                        "1.2.3-RC.1", "1.2.3-preview.1", "1.2.3+build", "1.2.3a1",
                        "v1.2.3", " 1.2.3", "1.2.3\n", "1.2.3-rc.１"]:
            with self.subTest(version=version), self.assertRaises(ValueError):
                parse_version(version)


if __name__ == "__main__":
    unittest.main()
