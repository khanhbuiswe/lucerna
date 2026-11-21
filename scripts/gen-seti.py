#!/usr/bin/env python3
"""Generate Lucerna's Seti file icon data from VS Code and seti-ui sources.

Dev-only; the build never runs this and needs no network. Run it only to
update Seti. The outputs are committed:

  crates/lucerna-theme/src/seti_data.rs      definitions and match tables
  crates/lucerna-workbench/src/seti_svgs.rs  include_bytes! table of the SVGs
  crates/lucerna-workbench/assets/icons/seti/*.svg, LICENSE-*, README.md

Setup (once, outside the repo; downloads go in their own empty folders):
    python3 -m venv <venv> && <venv>/bin/pip install svgelements
    gh api repos/jesseweed/seti-ui/tarball/<sha> > seti.tgz   # unpack to <seti-ui>
    # vscode: a sparse checkout of /LICENSE.txt, /extensions/*/package.json
    # and /extensions/theme-seti/ is enough.

Usage:
    <venv>/bin/python -I scripts/gen-seti.py --seti <seti-ui> --vscode <vscode> \\
        [--vscode-commit <sha>]

<seti-ui> must be at the commit that vs-seti-icon-theme.json names in its
"version" field. Without --vscode-commit the vscode commit is read with
`git rev-parse HEAD` in <vscode>.

What it does (spec: docs/superpowers/specs/2026-11-15-file-icons-design.md,
"Seti data"):
  - Reads extensions/theme-seti/icons/vs-seti-icon-theme.json. Each
    definition name is its SVG name: drop the leading "_", then a trailing
    "_<n>" ("_typescript_1" -> typescript.svg).
  - Dark colour is the definition's fontColor; light colour is the fontColor
    of "<def>_light" (VS Code's dark x 0.9), darkened in HSL lightness, 0.01
    at a time and keeping hue and saturation, until it reaches 3:1 against
    #ffffff. A definition without fontColor (_todo) takes _default's colours.
  - Checks the theme's "light" block is exactly the main tables with every
    definition that has a _light variant renamed to it.
  - Maps the theme's languageIds to file names, name patterns and extensions
    through contributes.languages in extensions/*/package.json, plus the
    nonBuiltInLanguages list of extensions/theme-seti/build/update-icon-theme.js
    (included because Lucerna has no VS Code language extensions, so these
    give the icons a VS Code user with those extensions would see)
    (its inheritIconFromLanguage list is already folded into languageIds, and
    ignoreExtAssociation only edits fileExtensions, which we read as is).
    Name patterns "X.*" become prefixes ("x."), "*.x" suffixes (".x"), and
    other patterns whose only wildcard is "*" (no "**") go to LANG_NAME_GLOBS
    as is ("*docker*compose*.yml"); a pattern without wildcards is an exact
    file name. Patterns with "?", "[", "{", "/" or "**" are skipped with a
    warning. Keys are lowercase; on a duplicate key within
    one table the first entry wins, in extension folder name order.
  - Rewrites each used SVG's root viewBox to a square around the real drawing
    bounds (svgelements, transforms and strokes included), padded by PAD on
    each side, and drops width/height. The drawing itself is not changed.
"""

import argparse
import colorsys
import io
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

import svgelements

ROOT = Path(__file__).resolve().parent.parent
DATA_RS = ROOT / "crates/lucerna-theme/src/seti_data.rs"
SVGS_RS = ROOT / "crates/lucerna-workbench/src/seti_svgs.rs"
SVG_DIR = ROOT / "crates/lucerna-workbench/assets/icons/seti"
THEME_JSON = "extensions/theme-seti/icons/vs-seti-icon-theme.json"
UPDATE_JS = "extensions/theme-seti/build/update-icon-theme.js"

PAD = 0.0625
MIN_CONTRAST = 3.0
TABLES = ("fileExtensions", "fileNames", "languageIds")


def die(msg):
    sys.exit(f"gen-seti: error: {msg}")


def warn(msg):
    print(f"gen-seti: warning: {msg}", file=sys.stderr)


# ---------------------------------------------------------------- colours


def parse_hex(s):
    if not re.fullmatch(r"#[0-9a-fA-F]{6}", s):
        die(f"unexpected colour {s!r}")
    return int(s[1:], 16)


def channels(c):
    return (c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF


def darken_like_vscode(c):
    """VS Code's darkenColor: each channel x 0.9, rounded half up."""
    r, g, b = (int(v * 0.9 + 0.5) for v in channels(c))
    return (r << 16) | (g << 8) | b


def luminance(c):
    def lin(v):
        v /= 255
        return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4

    r, g, b = (lin(v) for v in channels(c))
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast_on_white(c):
    return 1.05 / (luminance(c) + 0.05)


def ensure_contrast(c):
    """Lower HSL lightness by 0.01 steps, hue and saturation fixed, to the
    first value with contrast >= MIN_CONTRAST against white."""
    if contrast_on_white(c) >= MIN_CONTRAST:
        return c
    r, g, b = (v / 255 for v in channels(c))
    h, l0, s = colorsys.rgb_to_hls(r, g, b)
    k = 1
    while True:
        l = max(l0 - k * 0.01, 0.0)
        rr, gg, bb = (round(v * 255) for v in colorsys.hls_to_rgb(h, l, s))
        out = (rr << 16) | (gg << 8) | bb
        if contrast_on_white(out) >= MIN_CONTRAST or l == 0.0:
            return out
        k += 1


# ------------------------------------------------------------- definitions


def glyph_of(defname):
    name = defname[1:] if defname.startswith("_") else defname
    return re.sub(r"_\d+$", "", name)


def build_defs(theme):
    icon_defs = theme["iconDefinitions"]
    default = icon_defs.get("_default", {}).get("fontColor")
    if default is None:
        die("_default has no fontColor")
    defs = {}
    for key in sorted(icon_defs):
        if key.endswith("_light"):
            if key[: -len("_light")] not in icon_defs:
                die(f"{key} has no main definition")
            continue
        base = key if "fontColor" in icon_defs[key] else "_default"
        dark = parse_hex(icon_defs[base]["fontColor"])
        light_def = icon_defs.get(base + "_light")
        vs_light = (
            parse_hex(light_def["fontColor"])
            if light_def and "fontColor" in light_def
            else darken_like_vscode(dark)
        )
        defs[key] = {
            "glyph": glyph_of(key),
            "dark": dark,
            "vs_light": vs_light,
            "light": ensure_contrast(vs_light),
        }
    return defs


def check_light_block(theme):
    icon_defs = theme["iconDefinitions"]
    light = theme.get("light")
    if not isinstance(light, dict):
        die("theme has no light block")
    if light.get("file") != theme.get("file", "") + "_light":
        die(f"light.file is {light.get('file')!r}, expected {theme.get('file')!r} + '_light'")
    if theme.get("file") != "_default":
        die(f"theme file default is {theme.get('file')!r}, expected '_default'")
    for table in TABLES:
        expected = {
            k: v + "_light" for k, v in theme[table].items() if v + "_light" in icon_defs
        }
        got = light.get(table, {})
        if got != expected:
            extra = sorted(set(got.items()) - set(expected.items()))[:5]
            missing = sorted(set(expected.items()) - set(got.items()))[:5]
            die(f"light.{table} does not mirror {table}: extra {extra}, missing {missing}")


# ---------------------------------------------------------------- languages


def non_builtin_languages(update_js):
    """Parse the nonBuiltInLanguages object literal of update-icon-theme.js."""
    m = re.search(r"const nonBuiltInLanguages\s*=\s*\{(.*?)\n\};", update_js, re.S)
    if not m:
        die(f"nonBuiltInLanguages not found in {UPDATE_JS}")
    out = []
    for lang, body in re.findall(r'"([\w-]+)"\s*:\s*\{([^}]*)\}', m.group(1)):
        entry = {"id": lang}
        for field, items in re.findall(r"(extensions|fileNames)\s*:\s*\[([^\]]*)\]", body):
            values = re.findall(r"'([^']*)'", items)
            if field == "extensions":
                entry["extensions"] = ["." + v for v in values]
            else:
                entry["filenames"] = values
        out.append(entry)
    if not out:
        die("nonBuiltInLanguages parsed empty")
    return out


def language_contributions(vscode):
    """(source, language) pairs from every extension, folder name order,
    then the theme's hard-coded non-built-in languages."""
    out = []
    for pkg in sorted((vscode / "extensions").glob("*/package.json")):
        data = json.loads(pkg.read_text(encoding="utf-8"))
        langs = (data.get("contributes") or {}).get("languages") or []
        if isinstance(langs, list):
            for lang in langs:
                if isinstance(lang, dict) and lang.get("id"):
                    out.append((pkg.parent.name, lang))
    update_js = (vscode / UPDATE_JS).read_text(encoding="utf-8")
    for lang in non_builtin_languages(update_js):
        out.append(("update-icon-theme.js nonBuiltInLanguages", lang))
    return out


GLOB_CHARS = set("*?[]{}/")


def build_lang_tables(theme, vscode, stats):
    lang_defs = theme["languageIds"]
    tables = {"names": {}, "prefixes": {}, "suffixes": {}, "globs": {}, "exts": {}}
    seen_langs = set()

    def put(table, key, d, where):
        t = tables[table]
        if key in t:
            if t[key] != d:
                stats["conflicts"].append(f"{table} {key!r}: kept {t[key]}, dropped {d} ({where})")
            return
        t[key] = d

    for src, lang in language_contributions(vscode):
        d = lang_defs.get(lang["id"])
        if d is None:
            continue
        seen_langs.add(lang["id"])
        where = f"{src}:{lang['id']}"
        for ext in lang.get("extensions") or []:
            if not ext.startswith(".") or GLOB_CHARS & set(ext):
                warn(f"skipped extension {ext!r} ({where})")
                continue
            put("exts", ext[1:].lower(), d, where)
        for name in lang.get("filenames") or []:
            put("names", name.lower(), d, where)
        for pat in lang.get("filenamePatterns") or []:
            p = pat.lower()
            if p.endswith(".*") and not GLOB_CHARS & set(p[:-2]):
                put("prefixes", p[:-1], d, where)
            elif p.startswith("*.") and not GLOB_CHARS & set(p[2:]):
                put("suffixes", p[1:], d, where)
            elif not GLOB_CHARS & set(p):
                put("names", p, d, where)  # a pattern with no wildcard is a file name
            elif not (GLOB_CHARS - {"*"}) & set(p) and "**" not in p:
                put("globs", p, d, where)  # "*" wildcards only
            else:
                warn(f"skipped filename pattern {pat!r} ({where})")
    stats["langs_without_files"] = sorted(set(lang_defs) - seen_langs)
    return tables


# --------------------------------------------------------------------- SVGs

ROOT_TAG = re.compile(r"<svg\b[^>]*>", re.S)
SIZE_ATTR = re.compile(r"""\s(?:width|height|viewBox)\s*=\s*(?:"[^"]*"|'[^']*')""")


def fmt_num(v):
    s = f"{v:.3f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def drawing_bbox(text, name):
    """Bounds of everything painted, in the SVG's user units."""
    m = ROOT_TAG.search(text)
    bare = text[: m.start()] + SIZE_ATTR.sub("", m.group(0)) + text[m.end():]
    svg = svgelements.SVG.parse(io.StringIO(bare), reify=True)
    box = None
    for el in svg.elements():
        if not isinstance(el, svgelements.Shape):
            continue
        fill = el.fill is not None and el.fill.value is not None and el.fill.alpha != 0
        stroke = (
            el.stroke is not None
            and el.stroke.value is not None
            and el.stroke.alpha != 0
            and (el.stroke_width or 0) > 0
        )
        if not (fill or stroke):
            continue
        b = el.bbox(transformed=True, with_stroke=stroke)
        if b is None:
            continue
        box = b if box is None else (
            min(box[0], b[0]), min(box[1], b[1]), max(box[2], b[2]), max(box[3], b[3])
        )
    if box is None:
        die(f"{name}.svg draws nothing")
    return box


def normalise_svg(text, name):
    m = ROOT_TAG.search(text)
    if not m:
        die(f"{name}.svg has no <svg> root")
    x0, y0, x1, y1 = drawing_bbox(text, name)
    side = max(x1 - x0, y1 - y0) * (1 + 2 * PAD)
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    vb = " ".join(fmt_num(v) for v in (cx - side / 2, cy - side / 2, side, side))
    tag = SIZE_ATTR.sub("", m.group(0))
    tag = f'<svg viewBox="{vb}"' + tag[len("<svg"):]
    return text[: m.start()] + tag + text[m.end():]


# ------------------------------------------------------------------ output


def rust_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def write_rust(path, text):
    path.write_text(text, encoding="utf-8")
    rustfmt = shutil.which("rustfmt")
    if rustfmt:
        subprocess.run([rustfmt, "--edition", "2024", str(path)], check=True)
    else:
        warn(f"rustfmt not on PATH; {path.name} left unformatted")


def pairs_const(name, doc, mapping):
    lines = [f"/// {doc}", f"pub(crate) const {name}: &[(&str, &str)] = &["]
    lines += [f"    ({rust_str(k)}, {rust_str(v)})," for k, v in sorted(mapping.items())]
    lines += ["];", ""]
    return lines


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--seti", required=True, type=Path, help="seti-ui checkout")
    ap.add_argument("--vscode", required=True, type=Path, help="vscode checkout")
    ap.add_argument("--vscode-commit", help="vscode commit sha (default: git HEAD)")
    args = ap.parse_args()

    theme = json.loads((args.vscode / THEME_JSON).read_text(encoding="utf-8"))
    m = re.search(r"seti-ui/commit/([0-9a-f]{40})", theme.get("version", ""))
    if not m:
        die("theme version names no seti-ui commit")
    seti_commit = m.group(1)
    vscode_commit = args.vscode_commit or subprocess.run(
        ["git", "-C", str(args.vscode), "rev-parse", "HEAD"],
        check=True, capture_output=True, text=True,
    ).stdout.strip()
    if not re.fullmatch(r"[0-9a-f]{40}", vscode_commit):
        die(f"bad vscode commit {vscode_commit!r}")

    check_light_block(theme)
    defs = build_defs(theme)
    for table in TABLES:
        for k, v in theme[table].items():
            if v not in defs:
                die(f"{table}[{k!r}] -> {v}, which is not a main definition")

    stats = {"conflicts": []}
    lang = build_lang_tables(theme, args.vscode, stats)

    glyphs = sorted({d["glyph"] for d in defs.values()})
    missing = [g for g in glyphs if not (args.seti / "icons" / f"{g}.svg").is_file()]
    if missing:
        die(f"glyphs referenced by DEFS without an SVG in seti-ui: {missing}")
    # Everything that can fail runs before anything is deleted or written.
    svgs = {
        g: normalise_svg((args.seti / "icons" / f"{g}.svg").read_text(encoding="utf-8"), g)
        for g in glyphs
    }
    for src in (args.seti / "LICENSE.md", args.vscode / "LICENSE.txt"):
        if not src.is_file():
            die(f"{src} not found")

    # SVGs, licenses, README
    SVG_DIR.mkdir(parents=True, exist_ok=True)
    for old in SVG_DIR.glob("*.svg"):
        old.unlink()
    for g, out in svgs.items():
        (SVG_DIR / f"{g}.svg").write_text(out, encoding="utf-8")
    shutil.copyfile(args.seti / "LICENSE.md", SVG_DIR / "LICENSE-seti-ui")
    shutil.copyfile(args.vscode / "LICENSE.txt", SVG_DIR / "LICENSE-vscode")
    (SVG_DIR / "README.md").write_text(
        "# Seti file icons\n\n"
        "Generated by `scripts/gen-seti.py`; do not edit by hand.\n\n"
        f"- SVGs: [jesseweed/seti-ui](https://github.com/jesseweed/seti-ui) at "
        f"`{seti_commit}`, `icons/`. MIT, see `LICENSE-seti-ui`.\n"
        f"- Icon definitions, colours and file associations: "
        f"[microsoft/vscode](https://github.com/microsoft/vscode) at `{vscode_commit}`, "
        "`extensions/theme-seti` and the `contributes.languages` of its built-in "
        "extensions. MIT, see `LICENSE-vscode`.\n\n"
        "Each SVG's viewBox has been reset to a square around its drawing, so every "
        "icon has the same size and is centred; width and height were removed. "
        "The drawing paths are unchanged.\n\n"
        "VS Code's `nonBuiltInLanguages` are included because Lucerna has no VS Code "
        "language extensions, so these give the icons a VS Code user with those "
        "extensions would see.\n",
        encoding="utf-8",
    )

    header = (
        f"// @generated by scripts/gen-seti.py — seti-ui {seti_commit}, "
        f"vscode {vscode_commit}. Do not edit.\n"
    )

    # seti_data.rs
    lines = [
        header,
        "/// A Seti icon definition: the glyph (SVG name) and its colours, 0xRRGGBB.",
        "pub(crate) struct SetiDef {",
        "    pub glyph: &'static str,",
        "    pub dark: u32,",
        "    pub light: u32,",
        "}",
        "",
        "/// The definition for files nothing else matches.",
        'pub(crate) const DEFAULT_DEF: &str = "_default";',
        "",
        "/// Every definition, sorted by key.",
        "pub(crate) const DEFS: &[(&str, SetiDef)] = &[",
    ]
    for k, d in defs.items():
        lines.append(
            f"    ({rust_str(k)}, SetiDef {{ glyph: {rust_str(d['glyph'])}, "
            f"dark: 0x{d['dark']:06x}, light: 0x{d['light']:06x} }}),"
        )
    lines += ["];", ""]
    seti_ext = {k.lower(): v for k, v in theme["fileExtensions"].items()}
    seti_names = {k.lower(): v for k, v in theme["fileNames"].items()}
    lines += pairs_const("FILE_NAMES", "Seti file names, lowercase.", seti_names)
    lines += pairs_const(
        "FILE_EXTENSIONS", "Seti file extensions, lowercase, no leading dot.", seti_ext
    )
    lines += pairs_const(
        "LANG_FILE_NAMES", "File names of languages with a Seti icon, lowercase.", lang["names"]
    )
    lines += pairs_const(
        "LANG_NAME_PREFIXES", 'Language file name patterns "X.*", as "x.".', lang["prefixes"]
    )
    lines += pairs_const(
        "LANG_NAME_SUFFIXES", 'Language file name patterns "*.x", as ".x".', lang["suffixes"]
    )
    lines += pairs_const(
        "LANG_NAME_GLOBS",
        'Other language file name patterns with only "*" wildcards, lowercase.',
        lang["globs"],
    )
    lines += pairs_const(
        "LANG_EXTENSIONS", "Language file extensions, lowercase, no leading dot.", lang["exts"]
    )
    write_rust(DATA_RS, "\n".join(lines).rstrip("\n") + "\n")

    # seti_svgs.rs
    lines = [
        header,
        "macro_rules! seti {",
        "    ($name:literal) => {",
        "        include_bytes!(concat!(",
        '            env!("CARGO_MANIFEST_DIR"),',
        '            "/assets/icons/seti/",',
        "            $name,",
        '            ".svg"',
        "        ))",
        "    };",
        "}",
        "",
        "/// The Seti SVGs (MIT, see `assets/icons/seti/`), sorted by glyph name.",
        "pub(crate) const SETI: &[(&str, &[u8])] = &[",
    ]
    lines += [f"    ({rust_str(g)}, seti!({rust_str(g)}))," for g in glyphs]
    lines += ["];"]
    write_rust(SVGS_RS, "\n".join(lines) + "\n")

    # stats
    print(f"seti-ui {seti_commit}, vscode {vscode_commit}")
    print(f"definitions: {len(defs)} (of {len(theme['iconDefinitions'])} incl. _light)")
    print(f"glyphs: {len(glyphs)}")
    print(
        f"tables: FILE_NAMES {len(seti_names)}, FILE_EXTENSIONS {len(seti_ext)}, "
        f"LANG_FILE_NAMES {len(lang['names'])}, LANG_NAME_PREFIXES {len(lang['prefixes'])}, "
        f"LANG_NAME_SUFFIXES {len(lang['suffixes'])}, LANG_NAME_GLOBS {len(lang['globs'])}, "
        f"LANG_EXTENSIONS {len(lang['exts'])}"
    )
    print(f"languageIds with no file associations: {stats['langs_without_files']}")
    for c in stats["conflicts"]:
        print(f"duplicate key: {c}")
    adjusted = [(k, d) for k, d in defs.items() if d["light"] != d["vs_light"]]
    print(f"light colours adjusted to >= {MIN_CONTRAST}:1 on white: {len(adjusted)}")
    for k, d in adjusted:
        print(
            f"  {k}: #{d['vs_light']:06x} ({contrast_on_white(d['vs_light']):.2f}) -> "
            f"#{d['light']:06x} ({contrast_on_white(d['light']):.2f})"
        )


if __name__ == "__main__":
    main()
