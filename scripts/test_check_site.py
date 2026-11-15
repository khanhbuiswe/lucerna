"""Tests for scripts/check-site.py. Run: python3 -m unittest scripts/test_check_site.py -v"""

import importlib.util
import pathlib
import tempfile
import unittest

_spec = importlib.util.spec_from_file_location(
    "check_site", pathlib.Path(__file__).with_name("check-site.py")
)
check_site_mod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(check_site_mod)
check_site = check_site_mod.check_site

VALID_HTML = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Lucerna</title>
<meta name="description" content="A fast, native code viewer.">
<meta property="og:title" content="Lucerna">
<meta property="og:description" content="A fast, native code viewer.">
<meta property="og:image" content="https://lucerna.dev/og.png">
<meta property="og:url" content="https://lucerna.dev/">
<meta name="twitter:card" content="summary_large_image">
<meta name="theme-color" content="#080b12">
<link rel="canonical" href="https://lucerna.dev/">
<link rel="icon" type="image/svg+xml" href="favicon.svg">
</head>
<body>
<header id="top"><a href="#top">Lucerna</a></header>
<div class="v">169<small>MB</small></div>
<div class="v">1.40<small>%</small></div>
<div class="v">50<small>MB</small></div>
</body>
</html>
"""


class CheckSiteTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def write(self, html, files=None):
        (self.root / "index.html").write_text(html, encoding="utf-8")
        (self.root / "favicon.svg").write_text("<svg/>", encoding="utf-8")
        (self.root / "og.png").write_bytes(b"png")
        for name, text in (files or {}).items():
            (self.root / name).write_text(text, encoding="utf-8")
        return self.root

    def assertFailureMentions(self, failures, needle):
        self.assertTrue(
            any(needle in f for f in failures),
            f"expected a failure mentioning {needle!r}, got {failures!r}",
        )

    def test_valid_site_passes(self):
        self.assertEqual(check_site(self.write(VALID_HTML)), [])

    def test_missing_index_fails(self):
        self.assertFailureMentions(check_site(self.root), "index.html")

    def test_missing_theme_color_fails(self):
        html = VALID_HTML.replace('<meta name="theme-color" content="#080b12">', "")
        self.assertFailureMentions(check_site(self.write(html)), "theme-color")

    def test_dead_anchor_fails(self):
        html = VALID_HTML.replace("</body>", '<a href="#nope">x</a></body>')
        self.assertFailureMentions(check_site(self.write(html)), "#nope")

    def test_root_relative_paths_fail(self):
        html = VALID_HTML.replace(
            "</head>", '<link rel="stylesheet" href="/styles.css"></head>'
        )
        root = self.write(html, {"styles.css": ""})
        self.assertFailureMentions(check_site(root), "/styles.css")

    def test_missing_local_file_fails(self):
        html = VALID_HTML.replace("</head>", '<script src="main.js"></script></head>')
        self.assertFailureMentions(check_site(self.write(html)), "main.js")

    def test_external_stylesheet_fails(self):
        html = VALID_HTML.replace(
            "</head>", '<link rel="stylesheet" href="styles.css"></head>'
        )
        root = self.write(
            html, {"styles.css": "@import url(https://fonts.googleapis.com/x);"}
        )
        self.assertFailureMentions(check_site(root), "fonts.googleapis.com")

    def test_foreign_link_fails(self):
        html = VALID_HTML.replace("</body>", '<a href="https://example.com">x</a></body>')
        self.assertFailureMentions(check_site(self.write(html)), "example.com")

    def test_github_link_passes(self):
        html = VALID_HTML.replace(
            "</body>",
            '<a href="https://github.com/khanhbuiswe/lucerna/issues">x</a></body>',
        )
        self.assertEqual(check_site(self.write(html)), [])

    def test_inline_style_import_fails(self):
        html = VALID_HTML.replace(
            "</head>", "<style>@import url(https://fonts.googleapis.com/css2);</style></head>"
        )
        self.assertFailureMentions(check_site(self.write(html)), "fonts.googleapis.com")

    def test_style_attribute_url_fails(self):
        html = VALID_HTML.replace(
            "</body>", '<div style="background:url(https://cdn.example/a.png)"></div></body>'
        )
        self.assertFailureMentions(check_site(self.write(html)), "cdn.example")

    def test_srcset_external_fails(self):
        html = VALID_HTML.replace(
            "</body>", '<img src="og.png" srcset="og.png 1x, https://cdn.example/b.png 2x" alt=""></body>'
        )
        self.assertFailureMentions(check_site(self.write(html)), "cdn.example")

    def test_poster_external_fails(self):
        html = VALID_HTML.replace(
            "</body>", '<video poster="https://cdn.example/p.png"></video></body>'
        )
        self.assertFailureMentions(check_site(self.write(html)), "cdn.example")

    def test_inline_script_fails(self):
        html = VALID_HTML.replace(
            "</body>", '<script>import("https://cdn.example/x.js")</script></body>'
        )
        self.assertFailureMentions(check_site(self.write(html)), "inline <script>")

    def test_aria_label_on_plain_div_fails(self):
        html = VALID_HTML.replace("</body>", '<div aria-label="Shortcuts"></div></body>')
        self.assertFailureMentions(check_site(self.write(html)), "aria-label")

    def test_aria_label_on_div_with_role_passes(self):
        html = VALID_HTML.replace(
            "</body>", '<div role="group" aria-label="Shortcuts"></div></body>'
        )
        self.assertEqual(check_site(self.write(html)), [])

    def test_missing_metric_fails(self):
        html = VALID_HTML.replace("1.40<small>", "1.04<small>")
        self.assertFailureMentions(check_site(self.write(html)), "1.40%")


if __name__ == "__main__":
    unittest.main()
