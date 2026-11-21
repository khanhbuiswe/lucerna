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

    def test_fences_and_inline_code_counted(self):
        s = signature("```rust\nlet a = `b`;\n```\nText `c` and `d`.\n")
        self.assertEqual((s["fences"], s["inline_code"]), (2, 2))


if __name__ == "__main__":
    unittest.main()
