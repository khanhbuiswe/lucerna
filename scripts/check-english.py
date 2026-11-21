#!/usr/bin/env python3
"""English-only checks for the repository.

Usage:
  python3 scripts/check-english.py scan [PATH...]
      Print `path:line: text` for every line containing Vietnamese (default:
      all tracked files). Inside crates/**, bench/** and scripts/test_*.py,
      Vietnamese within "...", '...' or `...` is test data and allowed.
  python3 scripts/check-english.py structure --base REV FILE...
      Compare the Markdown structure of each FILE at REV with the working
      tree (heading levels and numbers, fences, table columns, inline code,
      links) and report broken #anchors in the working tree.
Both exit 1 if anything was printed.
"""

import argparse
import pathlib
import re
import subprocess
import sys

# Written as escapes so this file itself never contains Vietnamese letters.
VIETNAMESE = (
    "\u00e0\u00e1\u1ea1\u1ea3\u00e3\u00e2\u1ea7\u1ea5\u1ead\u1ea9\u1eab"
    "\u0103\u1eb1\u1eaf\u1eb7\u1eb3\u1eb5"
    "\u00e8\u00e9\u1eb9\u1ebb\u1ebd\u00ea\u1ec1\u1ebf\u1ec7\u1ec3\u1ec5"
    "\u00ec\u00ed\u1ecb\u1ec9\u0129"
    "\u00f2\u00f3\u1ecd\u1ecf\u00f5\u00f4\u1ed3\u1ed1\u1ed9\u1ed5\u1ed7"
    "\u01a1\u1edd\u1edb\u1ee3\u1edf\u1ee1"
    "\u00f9\u00fa\u1ee5\u1ee7\u0169\u01b0\u1eeb\u1ee9\u1ef1\u1eed\u1eef"
    "\u1ef3\u00fd\u1ef5\u1ef7\u1ef9\u0111"
)
VIETNAMESE_RE = re.compile("[" + VIETNAMESE + "]", re.IGNORECASE)

# A quoted span closed on the same line.
LITERAL_RE = re.compile(r'"(?:[^"\\]|\\.)*"' + r"|'(?:[^'\\]|\\.)*'" + r"|`[^`]*`")
# Words shared with other languages that are not Vietnamese (App Expos\u00e9).
NOT_VIETNAMESE_RE = re.compile("expos\u00e9", re.IGNORECASE)
# A lone letter such as "\u00e9 is 2 bytes": a code comment naming a character.
LONE_LETTER_RE = re.compile(r"(?<![^\W\d_])[^\W\d_](?![^\W\d_])")
# Letters Vietnamese shares with French, Spanish, Portuguese and others.
SHARED_LETTERS = "\u00e0\u00e1\u00e2\u00e3\u00e8\u00e9\u00ea\u00ec\u00ed\u00f2\u00f3\u00f4\u00f5\u00f9\u00fa\u00fd"
UNIQUE_RE = re.compile(
    "[" + "".join(c for c in VIETNAMESE if c not in SHARED_LETTERS) + "]", re.IGNORECASE
)
LITERAL_ALLOWED = re.compile(r"^(crates/|bench/|scripts/test_[^/]*\.py$)")

HEADING_RE = re.compile(r"^(#{1,6})[ \t]+(.*?)[ \t#]*$")
NUMBER_RE = re.compile(r"§?[0-9A-Za-z]+(\.[0-9A-Za-z]+)*\.?")
CODE_SPAN_RE = re.compile(r"(?<!`)(`+)(?!`)(.+?)(?<!`)\1(?!`)")
ANCHOR_RE = re.compile(r"\]\((#[^)\s]*)\)")


def has_vietnamese(text):
    return VIETNAMESE_RE.search(text) is not None


def _comment_literal(span):
    """In a comment a quoted span is allowed if it is one token (test data) or
    has no Vietnamese-only letters; a quoted Vietnamese phrase is prose."""
    if not re.search(r"\s", span) or not UNIQUE_RE.search(span):
        return ""
    return span


def scan_line(path, line):
    line = NOT_VIETNAMESE_RE.sub("", line)
    if LITERAL_ALLOWED.match(path):
        marker = "#" if path.endswith(".py") else "//"
        pattern = re.compile(LITERAL_RE.pattern + "|" + re.escape(marker) + ".*$")

        def strip(m):
            text = m.group(0)
            if text.startswith(marker):
                # Prose in a comment is checked; a quoted single token such as
                # "ab" is a test-data mention and stays allowed.
                return LITERAL_RE.sub(lambda q: _comment_literal(q.group(0)), text)
            return ""

        line = LONE_LETTER_RE.sub("", pattern.sub(strip, line))
    return has_vietnamese(line)


def _slug(title):
    """GitHub heading slug of the rendered text of a heading."""
    t = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", title)  # [text](url) -> text
    t = re.sub(r"(`+)(.+?)\1", r"\2", t)  # `code` -> code
    t = re.sub(r"(?<![\w*])(\*{1,3}|_{1,3})(?=\S)(.+?)(?<=\S)\1(?![\w*])", r"\2", t)
    kept = "".join(c for c in t.lower() if c.isalnum() or c in " -_")
    return kept.replace(" ", "-")


def _strip_code(line):
    """Replace inline code spans with a placeholder."""
    return CODE_SPAN_RE.sub("\0", line)


def _count_pipes(line):
    return _strip_code(line).count("|")


def signature(markdown):
    headings, table_rows, titles = [], [], []
    fences = inline_code = links = 0
    anchors = []
    in_fence = False
    for line in markdown.splitlines():
        if line.lstrip().startswith("```"):
            fences += 1
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        m = HEADING_RE.match(line)
        if m:
            title = m.group(2)
            token = title.split(None, 1)[0] if title.split() else ""
            number = token if NUMBER_RE.fullmatch(token) and any(c.isdigit() for c in token) else None
            headings.append((len(m.group(1)), number))
            titles.append(title)
        if line.lstrip().startswith("|"):
            table_rows.append(_count_pipes(line.lstrip()))
        inline_code += len(CODE_SPAN_RE.findall(line))
        links += line.count("](")
        anchors.extend(ANCHOR_RE.findall(line))

    slugs, seen = set(), {}
    for title in titles:
        base = _slug(title)
        n = seen.get(base, 0)
        slugs.add(base if n == 0 else f"{base}-{n}")
        seen[base] = n + 1
    broken = [a for a in anchors if a[1:] not in slugs]
    return {
        "headings": headings,
        "fences": fences,
        "table_rows": table_rows,
        "inline_code": inline_code,
        "links": links,
        "broken_anchors": broken,
    }


def _git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, check=False)


def _tracked(paths):
    out = _git("ls-files", "-z", "--", *paths).stdout
    return [p for p in out.split("\0") if p]


def scan(paths):
    flagged = 0
    for path in _tracked(paths):
        p = pathlib.Path(path)
        if not p.is_file():
            continue
        try:
            text = p.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        for n, line in enumerate(text.splitlines(), 1):
            if scan_line(path, line):
                print(f"{path}:{n}: {line}")
                flagged += 1
    return 1 if flagged else 0


def structure(base, files):
    printed = 0
    for path in files:
        old = _git("show", f"{base}:{path}")
        if old.returncode != 0:
            print(f"{path}: not present at {base}, skipped", file=sys.stderr)
            continue
        try:
            new_text = pathlib.Path(path).read_text(encoding="utf-8")
        except OSError as e:
            print(f"{path}: cannot read working tree file: {e}")
            printed += 1
            continue
        a, b = signature(old.stdout), signature(new_text)
        for key in ("headings", "fences", "table_rows", "inline_code", "links"):
            if a[key] != b[key]:
                print(f"{path}: {key} differ")
                if isinstance(a[key], list):
                    for i in range(max(len(a[key]), len(b[key]))):
                        x = a[key][i] if i < len(a[key]) else None
                        y = b[key][i] if i < len(b[key]) else None
                        if x != y:
                            print(f"  #{i + 1}: {x!r} -> {y!r}")
                else:
                    print(f"  {a[key]} -> {b[key]}")
                printed += 1
        for anchor in b["broken_anchors"]:
            print(f"{path}: broken anchor {anchor}")
            printed += 1
    return 1 if printed else 0


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)
    p_scan = sub.add_parser("scan")
    p_scan.add_argument("paths", nargs="*")
    p_struct = sub.add_parser("structure")
    p_struct.add_argument("--base", required=True)
    p_struct.add_argument("files", nargs="+")
    args = parser.parse_args(argv[1:])
    if args.cmd == "scan":
        return scan(args.paths)
    return structure(args.base, args.files)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
