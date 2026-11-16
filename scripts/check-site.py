#!/usr/bin/env python3
"""Static checks for the landing page in site/.

Usage: python3 scripts/check-site.py [site_dir]   (default: site)
Prints one line per failure and exits 1 if there are any.
"""

import pathlib
import re
import sys
from html.parser import HTMLParser

ORIGIN = "https://lucerna.dev/"
GITHUB = "https://github.com/khanhbuiswe/lucerna"
METRICS = ["169MB", "1.40%", "50MB"]
DMG_URL = re.compile(
    re.escape(GITHUB) + r"/releases/download/v(?P<v>[^/]+)/Lucerna-(?P=v)-macos-aarch64\.dmg"
)
CSS_URL = re.compile(r"""url\(\s*['"]?([^'")]+)""")
CSS_IMPORT = re.compile(r"""@import\s+['"]([^'"]+)""")


class _Page(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.lang = None
        self.title = ""
        self.meta = {}  # name or property -> content
        self.links = []  # (rel, href)
        self.ids = set()
        self.anchors = []  # every <a href>
        self.downloads = []  # (href, data-dmg) of <a data-dmg> buttons
        self.srcs = []  # src attributes
        self.metrics = []  # text of class="v" elements
        self.inline_css = []  # <style> blocks and style="" attributes
        self.inline_scripts = 0  # <script> elements without src
        self.unlabelled_roles = []  # div/span with aria-label but no role
        self._in_title = False
        self._in_style = False
        self._in_script = False
        self._metric_depth = 0
        self._metric_text = ""

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if "id" in a:
            self.ids.add(a["id"])
        if tag == "html":
            self.lang = a.get("lang")
        elif tag == "title":
            self._in_title = True
        elif tag == "meta":
            key = a.get("name") or a.get("property")
            if key:
                self.meta[key] = a.get("content", "")
        elif tag == "link":
            self.links.append((a.get("rel", ""), a.get("href", "")))
        elif tag == "a" and "href" in a:
            self.anchors.append(a["href"])
            if "data-dmg" in a:
                self.downloads.append((a["href"], a["data-dmg"]))
        for attr in ("src", "poster", "data"):
            if a.get(attr):
                self.srcs.append(a[attr])
        for attr in ("srcset", "imagesrcset"):
            for candidate in (a.get(attr) or "").split(","):
                if candidate.strip():
                    self.srcs.append(candidate.split()[0])
        if a.get("style"):
            self.inline_css.append(a["style"])
        if tag in ("div", "span") and "aria-label" in a and not a.get("role"):
            self.unlabelled_roles.append(f'<{tag} aria-label="{a["aria-label"]}">')
        if tag == "style":
            self._in_style = True
        elif tag == "script" and not a.get("src"):
            self._in_script = True
        if self._metric_depth:
            self._metric_depth += 1
        elif "v" in a.get("class", "").split():
            self._metric_depth = 1
            self._metric_text = ""

    def handle_endtag(self, tag):
        if tag == "title":
            self._in_title = False
        elif tag == "style":
            self._in_style = False
        elif tag == "script":
            self._in_script = False
        if self._metric_depth:
            self._metric_depth -= 1
            if not self._metric_depth:
                self.metrics.append(re.sub(r"\s+", "", self._metric_text))

    def handle_data(self, data):
        if self._in_title:
            self.title += data
        if self._in_style:
            self.inline_css.append(data)
        if self._in_script and data.strip():
            self.inline_scripts += 1
        if self._metric_depth:
            self._metric_text += data


def _is_external(url):
    return url.startswith(("http://", "https://", "//"))


def _check_local(root, base, url, where, failures):
    """A local reference must be relative and point at an existing file."""
    if url.startswith(("data:", "#")):
        return
    if _is_external(url):
        failures.append(f"{where}: external URL not allowed: {url}")
        return
    if url.startswith("/"):
        failures.append(f"{where}: root-relative path breaks under a sub-path: {url}")
        return
    path = url.split("#")[0].split("?")[0]
    if not (base / path).resolve().is_file() or root.resolve() not in (base / path).resolve().parents:
        failures.append(f"{where}: missing local file: {url}")


def check_site(root):
    root = pathlib.Path(root)
    failures = []
    index = root / "index.html"
    if not index.is_file():
        return [f"{index}: index.html is missing"]
    html = index.read_text(encoding="utf-8")
    if not html.lstrip().lower().startswith("<!doctype html>"):
        failures.append("index.html: must begin with <!doctype html>")

    page = _Page()
    page.feed(html)

    if page.lang != "en":
        failures.append('index.html: <html lang="en"> is required')
    if not page.title.strip():
        failures.append("index.html: <title> is empty or missing")
    for key in ("description", "og:title", "og:description", "og:image", "og:url", "viewport"):
        if not page.meta.get(key):
            failures.append(f"index.html: missing meta {key}")
    if page.meta.get("twitter:card") != "summary_large_image":
        failures.append("index.html: meta twitter:card must be summary_large_image")
    if page.meta.get("theme-color") != "#080b12":
        failures.append("index.html: meta theme-color must be #080b12")

    rels = {}
    for rel, href in page.links:
        for r in rel.split():
            rels.setdefault(r, href)
    if not rels.get("canonical"):
        failures.append("index.html: missing link rel=canonical")
    if rels.get("icon") != "favicon.svg":
        failures.append("index.html: link rel=icon must point to favicon.svg")
    if not (root / "og.png").is_file():
        failures.append("og.png: missing (og:image points at it)")

    for label, value in (
        ("og:image", page.meta.get("og:image", "")),
        ("og:url", page.meta.get("og:url", "")),
        ("canonical", rels.get("canonical", "")),
    ):
        if value and not value.startswith(ORIGIN):
            failures.append(f"index.html: {label} must start with {ORIGIN}: {value}")

    for href in page.anchors:
        if href.startswith("#"):
            if href[1:] not in page.ids:
                failures.append(f"index.html: dead anchor {href}")
        elif _is_external(href):
            if not href.startswith(GITHUB):
                failures.append(f"index.html: external link not allowed: {href}")
        elif href.startswith(("mailto:", "tel:")):
            failures.append(f"index.html: link scheme not allowed: {href}")
        else:
            _check_local(root, root, href, "index.html <a>", failures)

    # A download button links to the release page and carries the disk image
    # in data-dmg; main.js swaps it in only on an Apple silicon Mac.
    for href in page.anchors:
        if href.startswith(GITHUB + "/releases/download/"):
            failures.append(f"index.html: put release files in data-dmg, not href: {href}")
    for href, dmg in page.downloads:
        m = DMG_URL.fullmatch(dmg)
        if not m:
            failures.append(
                f"index.html: data-dmg must be {GITHUB}/releases/download/vX/Lucerna-X-macos-aarch64.dmg: {dmg}"
            )
            continue
        page_url = f"{GITHUB}/releases/tag/v{m['v']}"
        if href != page_url:
            failures.append(f"index.html: download href must be the release page {page_url}: {href}")

    for rel, href in page.links:
        if "canonical" in rel.split():
            continue
        _check_local(root, root, href, f"index.html <link rel={rel}>", failures)
    for src in page.srcs:
        _check_local(root, root, src, "index.html src", failures)

    for el in page.unlabelled_roles:
        failures.append(f"index.html: aria-label needs a role on a plain element: {el}")
    if page.inline_scripts:
        failures.append("index.html: inline <script> not allowed; put code in main.js")
    for text in page.inline_css:
        for url in CSS_URL.findall(text) + CSS_IMPORT.findall(text):
            _check_local(root, root, url.strip(), "index.html inline style", failures)

    for css in sorted(root.rglob("*.css")):
        text = css.read_text(encoding="utf-8")
        where = str(css.relative_to(root))
        for url in CSS_URL.findall(text) + CSS_IMPORT.findall(text):
            _check_local(root, css.parent, url.strip(), where, failures)

    if page.metrics != METRICS:
        failures.append(
            f"index.html: metric values must be {', '.join(METRICS)}; found {', '.join(page.metrics) or 'none'}"
        )
    return failures


def main(argv):
    root = pathlib.Path(argv[1] if len(argv) > 1 else "site")
    failures = check_site(root)
    for f in failures:
        print(f)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
