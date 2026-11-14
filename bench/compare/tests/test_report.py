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
        self.assertIn("rust-analyzer · lạnh (chỉ khởi động)", md)

    def test_meta_lists_binary_display_and_window(self):
        r = load()
        r["meta"].update({
            "lucerna_binary": {"path": "/x/luc", "mtime": "2026-11-14T01:02:03+00:00",
                               "version": "luc 0.1.0"},
            "display": {"width": 1470, "height": 956, "scale": 2.0, "refresh_hz": 120},
            "window_frame": {"x": 0, "y": 25, "w": 1280, "h": 800}})
        md = report.render_markdown(r)
        self.assertIn("luc 0.1.0, dựng lúc 2026-11-14T01:02:03+00:00", md)
        self.assertIn("1470×956 pt, scale 2.0, 120 Hz", md)
        self.assertIn("1280×800 pt", md)

    def test_idle_cpu_two_decimals(self):
        self.assertEqual(report.cell([0.04, 0.05, 0.03], "%", decimals=2),
                         "0,04 % (p90 0,05)")
        md = report.render_markdown(load())
        row = next(l for l in md.splitlines() if l.startswith("| CPU khi rỗi"))
        self.assertIn("0,40 % (p90 0,50)", row)
        # scroll % keeps one decimal
        row = next(l for l in md.splitlines() if l.startswith("| Khung rớt"))
        self.assertIn("0,2 % (p90 0,5)", row)

    def test_cpu_ratio_hidden_below_noise_floor(self):
        r = load()
        cfg = r["configs"][0]
        cfg["metrics"]["idle_cpu_pct"] = {"lucerna": [0.04, 0.03, 0.05],
                                          "zed": [0.8, 0.79, 0.81], "vscode": [2.0, 2.1, 2.2]}
        self.assertEqual(report.ratio_cells(cfg, "idle_cpu_pct"), ["—", "—", "—"])
        md = report.render_markdown(r)
        lines = md.splitlines()
        i = next(n for n, l in enumerate(lines) if l.startswith("| CPU khi rỗi"))
        self.assertEqual(lines[i + 1], "| ↳ so với Lucerna | — | — | — |")
        note = ("Hệ số không tính khi giá trị của Lucerna dưới ngưỡng đo được "
                "(CPU < 0,1 %).")
        self.assertEqual(md.count(note), 1)   # only the config below the floor
        self.assertNotIn("×", lines[i + 1])

    def test_cpu_ratio_shown_at_or_above_floor(self):
        r = load()
        cfg = r["configs"][0]
        cfg["metrics"]["idle_cpu_pct"] = {"lucerna": [0.1, 0.1, 0.1],
                                          "zed": [0.2, 0.2, 0.2], "vscode": [0.3, 0.3, 0.3]}
        self.assertEqual(report.ratio_cells(cfg, "idle_cpu_pct"), ["—", "2,0×", "3,0×"])
        self.assertNotIn("ngưỡng đo được", report.render_markdown(r).split("## ")[1])

    def test_ratio_format(self):
        self.assertEqual(report.ratio(140, 100), "1,4×")
        self.assertEqual(report.ratio(210, 100), "2,1×")

    def test_failed_rounds_reported(self):
        md = report.render_markdown(load())
        self.assertIn("1/3 lượt hỏng", md)       # zed scroll
        self.assertIn("3/3 lượt hỏng", md)       # vscode quick open all failed
        row = next(l for l in md.splitlines() if "Quick Open" in l)
        self.assertIn("— · 3/3 lượt hỏng", row)

    def test_trial_warning_only_with_preflight_ignored(self):
        r = load()
        self.assertNotIn("Kết quả chạy thử", report.render_markdown(r))
        r["meta"]["preflight_ignored"] = [{"round": 1, "reasons": ["x"]}]
        md = report.render_markdown(r)
        self.assertIn("Kết quả chạy thử: máy không yên khi đo (preflight bị bỏ qua), "
                      "không dùng để công bố.", md.splitlines()[0])

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
