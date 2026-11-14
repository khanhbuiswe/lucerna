import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import editors  # noqa: E402
import run  # noqa: E402

META = {"date": "2026-11-14", "lucerna_commit": "abc", "versions": {},
        "macos": "x", "chip": "y", "ram_gb": 16, "rust_analyzer": None}
ALL_OK = {"screen_recording": True, "accessibility": True}


class Harness:
    """Patches every side effect of run.main; records the calls."""

    def __init__(self, test, perms=ALL_OK, preflight=None, metric_values=None):
        self.test = test
        self.tmp = tempfile.TemporaryDirectory()
        test.addCleanup(self.tmp.cleanup)
        self.out = Path(self.tmp.name) / "out" / "r.json"
        self.calls = []          # (name, editor)
        self.sleeps = []
        self.preflights = 0
        self.snapshots = []      # result file content after each round
        self.perms = perms
        self.preflight_fn = preflight or (lambda: [])
        self.values = metric_values or {}
        self.app_size = lambda e: 5.0

    def metric(self, name):
        def fn(editor, paths):
            self.calls.append((name, editor))
            return self.values.get(name, 1.0)
        return fn

    def main(self, *argv):
        h = self
        patches = [
            mock.patch.object(run, "REST_S", 20),
            mock.patch.object(run, "_sleep", lambda s: h.sleeps.append(s)),
            mock.patch.object(run.env, "preflight", self._pre),
            mock.patch.object(run.env, "prepare_data", lambda p: None),
            mock.patch.object(run.env, "write_profiles",
                              lambda p, lsp: h.calls.append(("profiles", lsp))),
            mock.patch.object(run, "collect_meta", lambda lsp_modes: dict(META)),
            mock.patch.object(run, "helper_permissions", lambda p: h.perms),
            mock.patch.object(run.metrics, "load_calibration", lambda p: {"c": "1"}),
            mock.patch.object(run.metrics, "app_size_mb",
                              lambda e: h.calls.append(("app_size_mb", e)) or h.app_size(e)),
            mock.patch.object(run.metrics, "idle_session",
                              lambda e, p: h.calls.append(("idle_session", e)) or (7.0, 2.0)),
        ]
        for n in ("startup_ms", "scroll_dropped_pct", "file_change_ms", "quick_open_ms"):
            patches.append(mock.patch.object(run.metrics, n, self.metric(n)))
        for p in patches:
            p.start()
            self.test.addCleanup(p.stop)
        args = ["--bench-dir", self.tmp.name, "--out", str(self.out), "--skip-prepare", *argv]
        return run.main(args)

    def _pre(self):
        self.preflights += 1
        if self.out.exists():
            self.snapshots.append(json.loads(self.out.read_text()))
        return self.preflight_fn()

    def result(self):
        return json.loads(self.out.read_text())

    def count(self, name, editor=None):
        return sum(1 for c in self.calls if c[0] == name and editor in (None, c[1]))


class StatsOrder(unittest.TestCase):
    def test_order_rotates(self):
        self.assertNotEqual(run.order(0), run.order(1))
        firsts = [run.order(r)[0] for r in range(3)]
        self.assertEqual(sorted(firsts), sorted(editors.EDITORS))
        for r in range(3):
            self.assertEqual(sorted(run.order(r)), sorted(editors.EDITORS))

    def test_stats_ignore_failed(self):
        s = run.stats([10, None, 30, 20])
        self.assertEqual((s["median"], s["failed"]), (20, 1))
        self.assertEqual(s["p90"], 30)

    def test_stats_all_failed(self):
        self.assertEqual(run.stats([None, None]),
                         {"median": None, "p90": None, "failed": 2})

    def test_p90_nearest_rank(self):
        self.assertEqual(run.stats(list(range(1, 11)))["p90"], 9)


class Runner(unittest.TestCase):
    def test_first_startup_discarded(self):
        h = Harness(self)
        self.assertEqual(h.main("--rounds", "10", "--only", "startup_ms"), 0)
        for e in editors.EDITORS:
            self.assertEqual(h.count("startup_ms", e), 11)
        m = h.result()["configs"][0]["metrics"]["startup_ms"]
        for e in editors.EDITORS:
            self.assertEqual(len(m[e]), 10)

    def test_preflight_rechecked_each_round(self):
        h = Harness(self)
        h.main("--rounds", "4", "--only", "startup_ms")
        self.assertGreaterEqual(h.preflights, 4 + 1)  # rounds + warm-up

    def test_preflight_retries_then_stops_with_partial(self):
        h = Harness(self, preflight=lambda: ["build running"])
        rc = h.main("--rounds", "2", "--only", "scroll_dropped_pct")
        self.assertNotEqual(rc, 0)
        self.assertEqual(h.preflights, 11)       # first try + 10 retries
        self.assertEqual(h.sleeps.count(30), 10)
        self.assertEqual(h.count("scroll_dropped_pct"), 0)
        self.assertTrue(h.out.exists())

    def test_preflight_recovers(self):
        seq = iter([["busy"], ["busy"]])
        h = Harness(self, preflight=lambda: next(seq, []))
        self.assertEqual(h.main("--rounds", "1", "--only", "scroll_dropped_pct"), 0)
        self.assertIn(30, h.sleeps)

    def test_app_size_once_per_editor(self):
        h = Harness(self)
        h.main("--rounds", "3", "--only", "app_size_mb")
        for e in editors.EDITORS:
            self.assertEqual(h.count("app_size_mb", e), 1)
        m = h.result()["configs"][0]["metrics"]["app_size_mb"]
        self.assertEqual(m["zed"], [5.0])

    def test_only_filtering(self):
        h = Harness(self)
        h.main("--rounds", "1", "--only", "quick_open_ms,file_change_ms")
        names = {c[0] for c in h.calls} - {"profiles"}
        self.assertEqual(names, {"quick_open_ms", "file_change_ms"})
        self.assertEqual(set(h.result()["configs"][0]["metrics"]),
                         {"quick_open_ms", "file_change_ms"})

    def test_only_unknown_metric_rejected(self):
        h = Harness(self)
        with self.assertRaises(SystemExit):
            h.main("--only", "bogus")

    def test_idle_session_once_per_editor_round(self):
        h = Harness(self)
        h.main("--rounds", "3", "--only", "idle_ram_mb,idle_cpu_pct")
        for e in editors.EDITORS:
            self.assertEqual(h.count("idle_session", e), 3)
        m = h.result()["configs"][0]["metrics"]
        self.assertEqual(m["idle_ram_mb"]["vscode"], [7.0] * 3)
        self.assertEqual(m["idle_cpu_pct"]["vscode"], [2.0] * 3)

    def test_idle_only_ram_records_only_ram(self):
        h = Harness(self)
        h.main("--rounds", "1", "--only", "idle_ram_mb")
        self.assertEqual(set(h.result()["configs"][0]["metrics"]), {"idle_ram_mb"})

    def test_missing_permission_stops_before_measuring(self):
        h = Harness(self, perms={"screen_recording": False, "accessibility": True})
        rc = h.main("--rounds", "1")
        self.assertNotEqual(rc, 0)
        self.assertEqual([c for c in h.calls if c[0] != "profiles"], [])

    def test_permission_ok_for_subset(self):
        h = Harness(self, perms={"screen_recording": False, "accessibility": True})
        self.assertEqual(h.main("--rounds", "1", "--only", "app_size_mb"), 0)

    def test_partial_write_after_each_round(self):
        h = Harness(self)
        h.main("--rounds", "3", "--only", "scroll_dropped_pct")
        lens = [len(s["configs"][0]["metrics"]["scroll_dropped_pct"]["lucerna"])
                for s in h.snapshots if s["configs"]]
        self.assertIn(1, lens)
        self.assertIn(2, lens)
        self.assertEqual(len(h.result()["configs"][0]["metrics"]
                             ["scroll_dropped_pct"]["lucerna"]), 3)
        self.assertEqual(h.result()["meta"]["calibration"], {"c": "1"})
        self.assertEqual(h.result()["meta"]["rounds"], 3)

    def test_permission_denied_midrun_saves_partial(self):
        h = Harness(self)
        n = {"i": 0}

        def boom(editor, paths):
            n["i"] += 1
            if n["i"] > 4:
                raise run.metrics.PermissionDenied("x")
            return 3.0
        orig = h.metric
        h.metric = lambda name: boom if name == "scroll_dropped_pct" else orig(name)
        rc = h.main("--rounds", "3", "--only", "scroll_dropped_pct")
        self.assertNotEqual(rc, 0)
        m = h.result()["configs"][0]["metrics"]["scroll_dropped_pct"]
        # round 2 runs zed, then vscode raises: zed keeps its value
        self.assertEqual((len(m["lucerna"]), len(m["zed"]), len(m["vscode"])), (1, 2, 1))

    def test_failed_round_recorded_as_null(self):
        h = Harness(self, metric_values={"scroll_dropped_pct": None})
        h.main("--rounds", "2", "--only", "scroll_dropped_pct")
        self.assertEqual(h.result()["configs"][0]["metrics"]["scroll_dropped_pct"]["zed"],
                         [None, None])

    def test_lsp_both_and_cold_configs(self):
        h = Harness(self)
        with mock.patch("builtins.input", return_value="") as inp:
            h.main("--rounds", "2", "--lsp", "both", "--cold", "--only", "startup_ms")
            cfgs = h.result()["configs"]
            self.assertEqual([(c["lsp"], c["cold"]) for c in cfgs],
                             [(False, True), (True, True)])
            # 3 editors x (1 warm-up + 2 rounds) x 2 configs
            self.assertEqual(inp.call_count, 18)
        self.assertEqual([c for c in h.calls if c[0] == "profiles"],
                         [("profiles", False), ("profiles", True)])

    def test_rest_between_editor_runs(self):
        h = Harness(self)
        h.main("--rounds", "1", "--only", "scroll_dropped_pct")
        self.assertEqual(h.sleeps.count(20), 2)  # 3 runs, rest between

    def test_app_size_failure_is_null(self):
        h = Harness(self)
        h.app_size = mock.Mock(side_effect=OSError("x"))
        self.assertEqual(h.main("--rounds", "1", "--only", "app_size_mb"), 0)
        self.assertEqual(h.result()["configs"][0]["metrics"]["app_size_mb"]["zed"], [None])

    def test_unexpected_error_saves_partial_nonzero(self):
        h = Harness(self)
        with mock.patch.object(run, "wait_quiet", side_effect=RuntimeError("boom")):
            self.assertNotEqual(h.main("--rounds", "1", "--only", "scroll_dropped_pct"), 0)
        self.assertTrue(h.out.exists())

    def test_eof_on_cold_prompt(self):
        h = Harness(self)
        with mock.patch("builtins.input", side_effect=EOFError):
            self.assertNotEqual(h.main("--cold", "--only", "startup_ms"), 0)
        self.assertTrue(h.out.exists())


class Cli(unittest.TestCase):
    def test_help_under_isolated_mode(self):
        r = subprocess.run([sys.executable, "-I", str(Path(run.__file__)), "--help"],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)


if __name__ == "__main__":
    unittest.main()
