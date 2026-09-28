import copy
import unittest

from scripts.evaluation.ecosystem_diff import compare, markdown


def item(code="DUP001", start=1, message="Repeated definition"):
    return {"source": "source", "path": "page.md", "code": code,
            "span": {"start": start, "end": start + 1}, "input_sha256": "source-hash",
            "diagnostic": {"code": code, "message": message, "related": [],
                           "location": {"row": 1, "column": 2}}}


def report(items):
    return {"corpus_sha256": "corpus", "kind_profile_sha256": "kinds", "inventory_sha256": "tree",
            "diagnostics": items}


class ComparisonTests(unittest.TestCase):
    def test_add_remove_changed_and_input_order(self):
        old = report([item(), item("LNK002")])
        new = report([item("OWN002"), item(message="A better explanation")])
        diff = compare(old, new)
        self.assertEqual(diff["counts"], {"added": {"OWN002": 1}, "removed": {"LNK002": 1}, "changed": {"DUP001": 1}})
        new["diagnostics"].reverse()
        self.assertEqual(diff, compare(old, new))

    def test_related_changes_are_visible(self):
        old = report([item()])
        new = copy.deepcopy(old)
        new["diagnostics"][0]["diagnostic"]["related"].append({"filename": "owner.md"})
        self.assertEqual(len(compare(old, new)["changed"]), 1)

    def test_refuses_incompatible_inputs_and_duplicates(self):
        old = report([item()])
        for field in ["corpus_sha256", "kind_profile_sha256", "inventory_sha256"]:
            new = copy.deepcopy(old)
            new[field] = "different"
            with self.assertRaises(ValueError):
                compare(old, new)
        with self.assertRaises(ValueError):
            compare(old, report([item(), item()]))
        new = copy.deepcopy(old)
        new["diagnostics"][0]["input_sha256"] = "different"
        with self.assertRaises(ValueError):
            compare(old, new)

    def test_moved_diagnostic_cannot_hide_changed_source_bytes(self):
        old = report([item()])
        moved = item(code="OWN002", start=20)
        moved["input_sha256"] = "different"
        with self.assertRaises(ValueError):
            compare(old, report([moved]))

    def test_markdown_escapes_untrusted_source_and_caps_details(self):
        diff = compare(report([]), report([item(start=n, message="<img src=x>\n## forged") for n in range(3)]))
        text = markdown(diff, limit=1)
        self.assertNotIn("<img", text)
        self.assertIn("&lt;img", text)
        self.assertIn("Showing 1 of 3", text)


if __name__ == "__main__":
    unittest.main()
