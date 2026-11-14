import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import report  # noqa: E402

HERE = Path(__file__).resolve().parent
FIXTURE = HERE / "fixtures" / "result.json"


def load():
    return json.loads(FIXTURE.read_text())


class ReportTests(unittest.TestCase):
    def test_table_has_three_editor_columns(self):
        md = report.render_markdown(load())
        self.assertIn("| Chỉ số | Lucerna | Zed | VS Code |", md.splitlines())
        self.assertEqual(md.count("| Chỉ số | Lucerna | Zed | VS Code |"), 2)
        self.assertIn("Không LSP · ấm", md)
        self.assertIn("rust-analyzer · lạnh", md)

    def test_ratio_format(self):
        self.assertEqual(report.ratio(140, 100), "1,4×")
        self.assertEqual(report.ratio(210, 100), "2,1×")

    def test_failed_rounds_reported(self):
        md = report.render_markdown(load())
        self.assertIn("1/3 lượt hỏng", md)       # zed scroll
        self.assertIn("3/3 lượt hỏng", md)       # vscode quick open all failed
        row = next(l for l in md.splitlines() if "Quick Open" in l)
        self.assertIn("— · 3/3 lượt hỏng", row)

    def test_svg_has_three_bars(self):
        cfg = load()["configs"][0]
        svg = report.render_svg("startup_ms", cfg)
        self.assertEqual(svg.count("<rect"), 3)
        self.assertTrue(svg.startswith("<svg"))

    def test_svg_missing_value_still_three_rects(self):
        cfg = load()["configs"][0]
        svg = report.render_svg("quick_open_ms", cfg)
        self.assertEqual(svg.count("<rect"), 3)
        self.assertIn("—", svg)

    def test_missing_lucerna_median_gives_dash(self):
        md = report.render_markdown(load())
        self.assertIn("| ↳ so với Lucerna | — | — | — |", md.splitlines())

    def test_ratio_row_per_column_and_charts_list(self):
        md = report.render_markdown(load())
        self.assertIn("| ↳ so với Lucerna | — | 1,5× | 3,2× |", md.splitlines())
        self.assertIn("(startup_ms-nolsp-warm.svg)", md)
        self.assertIn("Biểu đồ:", md)
        self.assertNotIn("(ms) |", md)

    def test_ratio_half_up(self):
        self.assertEqual(report.ratio(125, 100), "1,3×")
        self.assertEqual(report.ratio(1.25, 1), "1,3×")

    def test_svg_dark_mode_style(self):
        svg = report.render_svg("startup_ms", load()["configs"][0])
        self.assertIn("prefers-color-scheme:dark", svg)
        self.assertNotIn("currentColor", svg)

    def test_ratio_zero_or_none(self):
        self.assertEqual(report.ratio(5, 0), "—")
        self.assertEqual(report.ratio(None, 5), "—")
        self.assertEqual(report.ratio(5, None), "—")

    def test_decimal_comma(self):
        self.assertEqual(report.fmt(123.456, "ms"), "123 ms")
        self.assertEqual(report.fmt(12.34, "ms"), "12,3 ms")
        self.assertEqual(report.fmt(1234.5, "MB"), "1234 MB")
        self.assertEqual(report.fmt(0.46, "%"), "0,5 %")
        self.assertNotIn(".", report.fmt(12.34, "ms"))

    def test_cell_formats(self):
        c = report.cell([100.0, 120.0, 110.0], "ms")
        self.assertEqual(c, "110 ms (p90 120)")
        self.assertEqual(report.cell([38.4], "MB", single=True), "38 MB")
        self.assertEqual(report.cell([None, None], "ms"), "— · 2/2 lượt hỏng")

    def test_metric_order_and_skip(self):
        md = report.render_markdown(load())
        self.assertLess(md.index("Khởi động"), md.index("RAM khi rỗi"))
        second = md.split("rust-analyzer · lạnh", 1)[1]
        self.assertNotIn("Dung lượng app", second)

    def test_meta_block(self):
        md = report.render_markdown(load())
        for s in ("2026-11-14", "0068887", "0.170.2", "1.95.3", "Apple M2 Pro",
                  "16 GB", "26.1", "rust-analyzer 1.91.0"):
            self.assertIn(s, md)

    def test_cli_smoke(self):
        with tempfile.TemporaryDirectory() as t:
            r = subprocess.run([sys.executable, "-I", str(HERE.parent / "report.py"),
                                str(FIXTURE), "--out", t],
                               capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertTrue((Path(t) / "results.md").exists())
            self.assertTrue((Path(t) / "startup_ms-nolsp-warm.svg").exists())
            self.assertTrue((Path(t) / "idle_ram_mb-lsp-cold.svg").exists())


if __name__ == "__main__":
    unittest.main()
