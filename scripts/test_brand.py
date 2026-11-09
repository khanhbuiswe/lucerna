"""Tests for the J2 logo files. Run: python3 -m unittest scripts/test_brand.py -v

Spec: docs/superpowers/specs/2026-11-09-logo-design.md
"""

import pathlib
import unittest
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent
BRAND = ROOT / "assets" / "brand"

MARK_D = ["M10 42a22 22 0 0 1 44 0Z", "M18 42a14 14 0 0 1 28 0Z", "M25 42a7 7 0 0 1 14 0Z", "M4 44H60"]
SMALL_D = ["M10 42a22 22 0 0 1 44 0Z", "M22 42a10 10 0 0 1 20 0Z", "M4 44H60"]
MONO_D = ["M17 42a15 15 0 0 1 30 0", "M24 42a8 8 0 0 1 16 0Z", "M4 44H60"]


def _strip(el):
    for e in el.iter():
        if isinstance(e.tag, str) and "}" in e.tag:
            e.tag = e.tag.split("}", 1)[1]
    return el


def load(svg):
    """Parse an SVG file or SVG markup string; return the namespace-stripped root."""
    if isinstance(svg, pathlib.Path):
        return _strip(ET.parse(svg).getroot())
    return _strip(ET.fromstring(svg))


def paths(svg):
    """Every <path> in document order, namespace stripped."""
    root = svg if isinstance(svg, ET.Element) else load(svg)
    return list(root.iter("path"))


def ds(svg):
    return [p.get("d") for p in paths(svg)]


class BrandTest(unittest.TestCase):
    def test_mark_full(self):
        root = load(BRAND / "mark.svg")
        self.assertEqual(root.get("viewBox"), "0 0 64 64")
        ps = paths(root)
        self.assertEqual([p.get("d") for p in ps], MARK_D)
        self.assertEqual([p.get("opacity") for p in ps[:3]], [".18", ".45", None])
        self.assertEqual({p.get("fill") for p in ps[:3]}, {"#4f8cff"})
        line = ps[3]
        self.assertEqual(
            (line.get("stroke"), line.get("stroke-width"), line.get("stroke-linecap")),
            ("#e6eaf2", "5", "round"),
        )

    def test_mark_small(self):
        root = load(BRAND / "mark-small.svg")
        self.assertEqual(root.get("viewBox"), "0 0 64 64")
        ps = paths(root)
        self.assertEqual([p.get("d") for p in ps], SMALL_D)
        self.assertEqual([p.get("opacity") for p in ps[:2]], [".3", None])
        self.assertEqual(ps[2].get("stroke-width"), "6")

    def test_mark_mono_is_single_colour(self):
        root = load(BRAND / "mark-mono.svg")
        self.assertEqual(ds(root), MONO_D)
        for el in root.iter():
            for attr in ("fill", "stroke"):
                if el.get(attr) is not None:
                    self.assertIn(el.get(attr), ("none", "currentColor"), f"{el.tag} {attr}")
            self.assertIsNone(el.get("opacity"), el.tag)

    def test_app_icon(self):
        root = load(BRAND / "app-icon.svg")
        self.assertEqual(root.get("viewBox"), "0 0 1024 1024")
        rects = list(root.iter("rect"))
        self.assertEqual(len(rects), 1)
        want = {"x": "7.5", "y": "7.5", "width": "1009", "height": "1009", "rx": "226",
                "fill": "#0f1420", "stroke": "#1b2232", "stroke-width": "15"}
        self.assertEqual({k: rects[0].get(k) for k in want}, want)
        groups = [g for g in root.iter("g") if g.get("transform") == "translate(192 179.5) scale(10)"]
        self.assertEqual(len(groups), 1)
        self.assertEqual(ds(groups[0]), MARK_D)

    # Task 2: lockups

    LOCKUPS = {"lockup-dark.svg": "#e6eaf2", "lockup-light.svg": "#080b12"}

    def _mark_group(self, root):
        groups = [g for g in root.iter("g") if (g.get("transform") or "").startswith("translate(")]
        self.assertEqual(len(groups), 1)
        return groups[0]

    def test_lockups_have_no_text(self):
        for name in self.LOCKUPS:
            root = load(BRAND / name)
            self.assertEqual(list(root.iter("text")) + list(root.iter("tspan")), [], name)

    def test_lockup_contains_mark(self):
        for name in self.LOCKUPS:
            g = self._mark_group(load(BRAND / name))
            self.assertEqual(ds(g), MARK_D, name)

    def test_lockup_colours(self):
        for name, ink in self.LOCKUPS.items():
            root = load(BRAND / name)
            g = self._mark_group(root)
            mark = paths(g)
            self.assertEqual({p.get("fill") for p in mark[:3]}, {"#4f8cff"}, name)
            self.assertEqual(mark[3].get("stroke"), ink, name)
            glyphs = [p for p in paths(root) if p not in mark]
            self.assertEqual(len(glyphs), len("Lucerna"), name)
            self.assertEqual({p.get("fill") for p in glyphs}, {ink}, name)

    def test_lockup_viewbox_contains_mark(self):
        import re

        for name in self.LOCKUPS:
            root = load(BRAND / name)
            vx, vy, vw, vh = map(float, root.get("viewBox").split())
            self.assertGreater(vw, 0)
            self.assertGreater(vh, 0)
            m = re.fullmatch(r"translate\(([-\d.]+) ([-\d.]+)\) scale\(([\d.]+)\)",
                             self._mark_group(root).get("transform"))
            self.assertIsNotNone(m, name)
            tx, ty, s = map(float, m.groups())
            margin = 7 * s - 0.01
            self.assertGreaterEqual(tx + 1.5 * s - vx, margin, name)
            self.assertGreaterEqual(ty + 18 * s - vy, margin, name)
            self.assertGreaterEqual(vy + vh - (ty + 47 * s), margin, name)
            self.assertGreaterEqual(vx + vw - (tx + 62.5 * s), margin, name)

    def test_lockup_gap_is_0_3_of_mark_height(self):
        import re

        for name in self.LOCKUPS:
            root = load(BRAND / name)
            g = self._mark_group(root)
            tx, ty, s = map(float, re.findall(r"[-\d.]+", g.get("transform")))
            mark = paths(g)
            first = [p for p in paths(root) if p not in mark][0]
            xs = []  # SVGPathPen emits absolute M/L/Q/C pairs, H (x only), V (y only), Z
            for cmd, args in re.findall(r"([MLHVQCZ])([^MLHVQCZ]*)", first.get("d")):
                nums = [float(n) for n in re.findall(r"-?\d+(?:\.\d+)?", args)]
                xs += nums if cmd == "H" else [] if cmd in "VZ" else nums[0::2]
            ink_left = min(xs)
            gap = ink_left - (tx + 62.5 * s)
            self.assertAlmostEqual(gap, 0.3 * 29 * s, delta=0.05, msg=name)

    # Task 3: the site uses the new logo

    SITE = ROOT / "site"

    def test_favicon_adapts_to_scheme(self):
        f = self.SITE / "favicon.svg"
        root = load(f)
        ps = paths(root)
        self.assertEqual([p.get("d") for p in ps], SMALL_D)
        self.assertEqual((ps[2].get("class"), ps[2].get("stroke")), ("line", "#080b12"))
        text = "".join(f.read_text().split())
        self.assertIn("prefers-color-scheme:dark", text)
        self.assertIn(".line{stroke:#e6eaf2}", text)

    def test_nav_logo(self):
        import re

        html = (self.SITE / "index.html").read_text()
        m = re.search(r'class="brand"[^>]*>\s*(<svg\b.*?</svg>)', html, re.S)
        self.assertIsNotNone(m)
        root = load(m.group(1).replace("<svg ", '<svg xmlns="http://www.w3.org/2000/svg" ', 1)
                    if "xmlns" not in m.group(1) else m.group(1))
        self.assertEqual(root.get("viewBox"), "1 18 62 29")
        ps = paths(root)
        self.assertEqual([p.get("d") for p in ps], SMALL_D)
        self.assertEqual({p.get("fill") for p in ps[:2]}, {"var(--accent)"})
        self.assertEqual(ps[2].get("stroke"), "var(--ink)")

    def test_no_old_lamp_in_site(self):
        for f in self.SITE.rglob("*"):
            if f.suffix in (".html", ".css", ".js", ".svg"):
                self.assertNotIn("M12 2.5c2.6 3", f.read_text(), str(f))


if __name__ == "__main__":
    unittest.main()
