"""Tests for scripts/check-english.py. Run: python3 -m unittest scripts/test_check_english.py -v"""

import importlib.util
import pathlib
import unittest

_spec = importlib.util.spec_from_file_location(
    "check_english", pathlib.Path(__file__).with_name("check-english.py")
)
check_english = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(check_english)
has_vietnamese = check_english.has_vietnamese
scan_line = check_english.scan_line
signature = check_english.signature
parse_allow = check_english.parse_allow
is_allowed = check_english.is_allowed
scan = check_english.scan


class CheckEnglishTests(unittest.TestCase):
    def test_has_vietnamese(self):
        self.assertTrue(has_vietnamese("Bản nháp"))
        self.assertFalse(has_vietnamese("Draft, §3 ⌘⇧L"))

    def test_scan_allows_literals_in_crates(self):
        self.assertFalse(scan_line("crates/x/src/a.rs", 'assert_eq!(f("chào"), "ệ");'))
        self.assertTrue(scan_line("crates/x/src/a.rs", "// Nguồn symbol"))

    def test_scan_comment_rules_in_crates(self):
        # one-token test data and shared Latin letters in comments are allowed
        self.assertFalse(scan_line("crates/x/a.rs", '// "aé\\nb": é is 2 bytes'))
        self.assertFalse(scan_line("crates/x/a.rs", "// Byte 6 of `// héllo` follows"))
        self.assertFalse(scan_line("crates/x/a.rs", "// App Exposé lists it"))
        # a quoted Vietnamese phrase in a comment is prose
        self.assertTrue(scan_line("crates/x/a.rs", '//! spec §7.6 "Nguồn symbol"'))
        self.assertTrue(scan_line("crates/x/a.rs", "/// spec's values (§ Bảng màu), VS Code's"))
        self.assertTrue(scan_line("scripts/test_x.py", "# Nguồn symbol"))
        self.assertFalse(scan_line("scripts/test_x.py", 'assert f("xin chào")'))

    def test_scan_flags_backticked_vietnamese_in_docs(self):
        self.assertTrue(scan_line("docs/a.md", "Gõ `chào` rồi Enter"))
        self.assertTrue(scan_line("docs/a.md", "see `chào`"))

    def test_heading_numbers_kept_for_section_and_alnum_forms(self):
        s = signature("# Tiêu đề\n## 3.1. Một\n### §7.6 Hai\n## V2a Ba\n")
        self.assertEqual(s["headings"], [(1, None), (2, "3.1."), (3, "§7.6"), (2, "V2a")])

    def test_structure_detects_table_column_change(self):
        a = signature("| a | `x|y` |\n|---|---|\n")
        b = signature("| a | `x|y` | c |\n|---|---|\n")
        self.assertNotEqual(a["table_rows"], b["table_rows"])
        self.assertEqual(a["table_rows"], [3, 3])

    def test_structure_reports_broken_anchor(self):
        s = signature("## Phần mới\nxem [x](#bổ-sung)\n")
        self.assertEqual(s["broken_anchors"], ["#bổ-sung"])
        ok = signature("## Bổ sung\nxem [x](#bổ-sung)\n")
        self.assertEqual(ok["broken_anchors"], [])

    def test_slug_keeps_underscore_and_strips_markdown(self):
        s = signature("## The `foo_bar` field\n## See [docs](x.md)\n## A *very* __bold__ one\n"
                      "[a](#the-foo_bar-field) [b](#see-docs) [c](#a-very-bold-one)\n")
        self.assertEqual(s["broken_anchors"], [])

    def test_allow_list_snippet_match(self):
        allow = parse_allow("# c\n\ndocs/a.md\t`ch\u00e0o`\n")
        self.assertTrue(is_allowed(allow, "docs/a.md", "type `ch\u00e0o` here"))

    def test_allow_list_no_match(self):
        allow = parse_allow("docs/a.md\t`ch\u00e0o`\n")
        # other file, other snippet, or Vietnamese prose left over after the snippet
        self.assertFalse(is_allowed(allow, "docs/b.md", "type `ch\u00e0o` here"))
        self.assertFalse(is_allowed(allow, "docs/a.md", "type `xin` here"))
        self.assertFalse(is_allowed(allow, "docs/a.md", "G\u00f5 `ch\u00e0o` r\u1ed3i"))

    def test_allow_list_whole_file(self):
        allow = parse_allow("docs/a.md\t*\n")
        self.assertTrue(is_allowed(allow, "docs/a.md", "B\u1ea3n nh\u00e1p"))
        self.assertFalse(is_allowed(allow, "docs/b.md", "B\u1ea3n nh\u00e1p"))

    def test_scan_uses_allow_file_and_reports_stale(self):
        import contextlib, io, os, subprocess, tempfile
        with tempfile.TemporaryDirectory() as d:
            subprocess.run(["git", "init", "-q", d], check=True)
            pathlib.Path(d, "a.md").write_text("one `ch\u00e0o`\ntwo B\u1ea3n\n", encoding="utf-8")
            allow = pathlib.Path(d, "allow.txt")
            allow.write_text("a.md\t`ch\u00e0o`\na.md\t`stale`\n", encoding="utf-8")
            subprocess.run(["git", "-C", d, "add", "a.md"], check=True)
            old = os.getcwd()
            os.chdir(d)
            try:
                out = io.StringIO()
                with contextlib.redirect_stdout(out):
                    rc = scan([], allow_file=str(allow))
            finally:
                os.chdir(old)
        self.assertEqual(rc, 1)
        self.assertIn("a.md:2:", out.getvalue())
        self.assertNotIn("a.md:1:", out.getvalue())
        self.assertIn("unused allow entry", out.getvalue())

    def _repo(self, d, text, allow_text):
        import subprocess
        subprocess.run(["git", "init", "-q", d], check=True)
        pathlib.Path(d, "a.md").write_text(text, encoding="utf-8")
        allow = pathlib.Path(d, "allow.txt")
        allow.write_text(allow_text, encoding="utf-8")
        subprocess.run(["git", "-C", d, "add", "a.md"], check=True)
        return allow

    def _run_scan(self, d, paths, allow):
        import contextlib, io, os
        old = os.getcwd()
        os.chdir(d)
        try:
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = scan(paths, allow_file=str(allow))
        finally:
            os.chdir(old)
        return rc, out.getvalue()

    def test_full_scan_fails_on_unused_allow_entry_only(self):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            allow = self._repo(d, "clean\n", "a.md\t`stale`\n")
            rc, out = self._run_scan(d, [], allow)
            self.assertEqual(rc, 1)
            self.assertIn("unused allow entry", out)
            rc, out = self._run_scan(d, ["a.md"], allow)
            self.assertEqual(rc, 0)

    def test_tracked_failure_exits_2(self):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            allow = pathlib.Path(d, "allow.txt")
            allow.write_text("", encoding="utf-8")
            with self.assertRaises(SystemExit) as cm:
                self._run_scan(d, [], allow)
            self.assertEqual(cm.exception.code, 2)

    def test_decomposed_vietnamese_is_flagged(self):
        import unicodedata
        nfd = unicodedata.normalize("NFD", "ch\u00e0o")
        self.assertNotEqual(nfd, "ch\u00e0o")
        self.assertTrue(has_vietnamese(nfd))
        self.assertTrue(scan_line("docs/a.md", "say " + nfd))

    def test_checker_does_not_flag_itself(self):
        here = pathlib.Path(__file__).parent
        for name in ("check-english.py", "test_check_english.py"):
            for n, line in enumerate((here / name).read_text(encoding="utf-8").splitlines(), 1):
                self.assertFalse(scan_line(f"scripts/{name}", line), f"{name}:{n}: {line}")
        # the checker itself has no literal allowance, so it must hold no Vietnamese at all
        text = (here / "check-english.py").read_text(encoding="utf-8")
        self.assertFalse(has_vietnamese(text))

    def test_fences_and_inline_code_counted(self):
        s = signature("```rust\nlet a = `b`;\n```\nText `c` and `d`.\n")
        self.assertEqual((s["fences"], s["inline_code"]), (2, 2))


if __name__ == "__main__":
    unittest.main()
