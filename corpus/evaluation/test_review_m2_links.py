import unittest

from review_m2_links import ExplicitAnchors


class ExplicitAnchorTests(unittest.TestCase):
    def test_names_on_headings_and_other_elements_follow_github_viewer(self):
        parser = ExplicitAnchors()
        parser.feed('<h3 name="config">Configuration</h3>'
                    '<SPAN NAME="two&amp;three"></SPAN><a name=legacy></a>'
                    '<div id=custom></div><custom-element name=安装 />')
        self.assertEqual(parser.anchors, {'config', 'two&three', 'legacy', 'custom', '安装'})

    def test_comments_and_raw_text_do_not_supply_nested_fake_names(self):
        parser = ExplicitAnchors()
        parser.feed('<!-- <h3 name=comment> -->'
                    '<script name=script>"<h3 name=fake>"</script>'
                    '<style>/* <span name=style-fake> */</style>')
        self.assertEqual(parser.anchors, {'script'})


if __name__ == '__main__':
    unittest.main()
