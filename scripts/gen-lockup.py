#!/usr/bin/env python3
"""Generate a Lucerna lockup SVG: the J2 mark plus "Lucerna" outlined as paths.

Dev-only; CI never runs this. The output SVGs in assets/brand/ are committed.

Setup (once, outside the repo):
    python3 -m venv /tmp/lucerna-brand
    /tmp/lucerna-brand/bin/pip install fonttools brotli uharfbuzz

Usage:
    /tmp/lucerna-brand/bin/python scripts/gen-lockup.py --ink '#e6eaf2' --out assets/brand/lockup-dark.svg
    /tmp/lucerna-brand/bin/python scripts/gen-lockup.py --ink '#080b12' --out assets/brand/lockup-light.svg

Spec: docs/superpowers/specs/2026-11-09-logo-design.md, section 4.
"""

import argparse
import io
import math
import pathlib

import uharfbuzz as hb
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = pathlib.Path(__file__).resolve().parent.parent
FONT = ROOT / "site" / "fonts" / "BricolageGrotesque-latin-wght.woff2"
TEXT = "Lucerna"
WGHT = 650
TRACKING = -0.02  # em
EM = 100  # output units per em
ACCENT = "#4f8cff"
# Tight crop of the 64-unit mark: x 1.5..62.5, y 18..47; line bottom at y 46.5.
CROP_X0, CROP_X1, CROP_Y0, CROP_Y1, LINE_BOTTOM = 1.5, 62.5, 18, 47, 46.5
CLEAR = 7  # grid units of clear space


def num(v):
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ink", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()

    font = TTFont(FONT)
    upm = font["head"].unitsPerEm
    sc = EM / upm

    # Shape with HarfBuzz (kerning) at wght 650. HarfBuzz cannot read woff2, so hand it sfnt bytes.
    buf = io.BytesIO()
    font.flavor = None
    font.save(buf)
    hb_font = hb.Font(hb.Face(buf.getvalue()))
    hb_font.set_variations({"wght": WGHT})
    hb_buf = hb.Buffer()
    hb_buf.add_str(TEXT)
    hb_buf.guess_segment_properties()
    hb.shape(hb_font, hb_buf, {})

    inst = instancer.instantiateVariableFont(TTFont(FONT), {"wght": WGHT})
    order = inst.getGlyphOrder()
    gs = inst.getGlyphSet()

    cap = inst["OS/2"].sCapHeight * sc
    s = round(1.25 * cap / (CROP_Y1 - CROP_Y0), 4)
    tx = round(-CROP_X0 * s, 2)
    ty = round(-LINE_BOTTOM * s, 2)  # line bottom on the baseline (y = 0)
    text_x = (CROP_X1 - CROP_X0) * s + 0.3 * (CROP_Y1 - CROP_Y0) * s

    glyphs, x = [], text_x
    tx0 = ty0 = math.inf
    tx1 = ty1 = -math.inf
    infos, poss = hb_buf.glyph_infos, hb_buf.glyph_positions
    for i, (info, pos) in enumerate(zip(infos, poss)):
        name = order[info.codepoint]
        gx, gy = x + pos.x_offset * sc, -pos.y_offset * sc
        t = (sc, 0, 0, -sc, gx, gy)
        pen = SVGPathPen(gs, ntos=num)
        gs[name].draw(TransformPen(pen, t))
        glyphs.append(pen.getCommands())
        bp = BoundsPen(gs)
        gs[name].draw(TransformPen(bp, t))
        if bp.bounds:
            a, b, c, d = bp.bounds
            tx0, ty0, tx1, ty1 = min(tx0, a), min(ty0, b), max(tx1, c), max(ty1, d)
        x += pos.x_advance * sc
        if i < len(infos) - 1:
            x += TRACKING * EM

    pad = CLEAR * s
    mx0, my0 = tx + CROP_X0 * s, ty + CROP_Y0 * s
    mx1, my1 = tx + CROP_X1 * s, ty + CROP_Y1 * s
    vx = math.floor((min(mx0, tx0) - pad) * 100) / 100
    vy = math.floor((min(my0, ty0) - pad) * 100) / 100
    vw = math.ceil((max(mx1, tx1) + pad - vx) * 100) / 100
    vh = math.ceil((max(my1, ty1) + pad - vy) * 100) / 100

    ink = args.ink
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{num(vx)} {num(vy)} {num(vw)} {num(vh)}">',
        f'  <g transform="translate({num(tx)} {num(ty)}) scale({s})">',
        f'    <path d="M10 42a22 22 0 0 1 44 0Z" fill="{ACCENT}" opacity=".18"/>',
        f'    <path d="M18 42a14 14 0 0 1 28 0Z" fill="{ACCENT}" opacity=".45"/>',
        f'    <path d="M25 42a7 7 0 0 1 14 0Z" fill="{ACCENT}"/>',
        f'    <path d="M4 44H60" stroke="{ink}" stroke-width="5" stroke-linecap="round"/>',
        "  </g>",
        *[f'  <path d="{d}" fill="{ink}"/>' for d in glyphs],
        "</svg>",
        "",
    ]
    pathlib.Path(args.out).write_text("\n".join(out))


if __name__ == "__main__":
    main()
