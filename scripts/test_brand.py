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


if __name__ == "__main__":
    unittest.main()
