import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import multiwindow  # noqa: E402


class SummarizeTest(unittest.TestCase):
    def test_median_min_max_per_metric(self):
        rounds = [
            {"second_window_ms": 300.0, "drop_pct_while_busy": 2.0},
            {"second_window_ms": 100.0, "drop_pct_while_busy": 0.0},
            {"second_window_ms": 200.0, "drop_pct_while_busy": 4.0},
        ]
        s = multiwindow.summarize(rounds)
        self.assertEqual(s["second_window_ms"],
                         {"median": 200.0, "min": 100.0, "max": 300.0, "n": 3, "failed": 0})
        self.assertEqual(s["drop_pct_while_busy"]["median"], 2.0)

    def test_nested_ram_is_flattened_per_window_count(self):
        rounds = [{"ram_mb": {"1": 100.0, "2": 190.0}},
                  {"ram_mb": {"1": 120.0, "2": 210.0}}]
        s = multiwindow.summarize(rounds)
        self.assertEqual(s["ram_mb[1]"]["median"], 110.0)
        self.assertEqual(s["ram_mb[2]"]["max"], 210.0)

    def test_none_rounds_are_counted_as_failed(self):
        rounds = [{"second_window_ms": None}, {"second_window_ms": 50.0},
                  {"second_window_ms": None}]
        s = multiwindow.summarize(rounds)["second_window_ms"]
        self.assertEqual((s["median"], s["n"], s["failed"]), (50.0, 1, 2))

    def test_all_failed_has_null_stats(self):
        s = multiwindow.summarize([{"x": None}])["x"]
        self.assertEqual(s, {"median": None, "min": None, "max": None, "n": 0, "failed": 1})

    def test_empty(self):
        self.assertEqual(multiwindow.summarize([]), {})


class LucCommandsTest(unittest.TestCase):
    def test_before_is_one_process_per_window(self):
        self.assertEqual(multiwindow.luc_commands("before", ["/r1", "/r2"], "luc"),
                         [["luc", "/r1"], ["luc", "/r2"]])

    def test_after_first_is_foreground_then_clients(self):
        self.assertEqual(multiwindow.luc_commands("after", ["/r1", "/r2", "/r3"], "luc"),
                         [["luc", "--foreground", "/r1"], ["luc", "/r2"], ["luc", "/r3"]])

    def test_default_binary_name_and_unknown_mode(self):
        self.assertEqual(multiwindow.luc_commands("after", ["/r1"]), [["luc", "--foreground", "/r1"]])
        with self.assertRaises(ValueError):
            multiwindow.luc_commands("sideways", ["/r1"])


class ArgsTest(unittest.TestCase):
    def test_windows_list(self):
        a = multiwindow.parse_args(["--windows", "1,2,3", "--out", "o.json"])
        self.assertEqual((a.windows, a.mode, a.rounds), ([1, 2, 3], "before", 5))

    def test_bad_windows_rejected(self):
        with self.assertRaises(SystemExit):
            multiwindow.parse_args(["--windows", "0,x", "--out", "o.json"])


if __name__ == "__main__":
    unittest.main()
