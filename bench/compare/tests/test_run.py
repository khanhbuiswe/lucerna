import io
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
        self.foreign = []
        self.ra_installed = True
        self.lucerna = {"path": "/x/luc", "mtime": "2026-11-14T00:00:00+00:00",
                        "version": "luc 0.1.0"}

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
            mock.patch.object(run.env, "reset_profiles",
                              lambda p: h.calls.append(("reset", None))),
            mock.patch.object(run.env, "vscode_rust_analyzer_installed",
                              lambda p: h.ra_installed),
            mock.patch.object(run.editors, "resolve_lucerna", self._resolve),
            mock.patch.object(run.editors, "foreign_instances",
                              lambda e, p: list(h.foreign)),
            mock.patch.object(run, "collect_meta", lambda lsp_modes, paths: dict(META)),
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

    def _resolve(self, refresh=False):
        if isinstance(self.lucerna, Exception):
            raise self.lucerna
        return dict(self.lucerna)

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

    def test_warm_up_round_covers_every_launching_metric(self):
        # rounds + 1 calls per launching metric and editor; round 0 discarded.
        h = Harness(self, metric_values={"scroll_dropped_pct": 4.0})
        self.assertEqual(h.main("--rounds", "3"), 0)
        for e in editors.EDITORS:
            for name in ("startup_ms", "scroll_dropped_pct", "file_change_ms",
                         "quick_open_ms", "idle_session"):
                self.assertEqual(h.count(name, e), 4, (name, e))
            self.assertEqual(h.count("app_size_mb", e), 1)
        m = h.result()["configs"][0]["metrics"]
        for name in run.METRICS:
            if name != "app_size_mb":
                self.assertEqual(len(m[name]["zed"]), 3, name)

    def test_warm_up_before_any_recorded_value(self):
        h = Harness(self)
        h.main("--rounds", "2", "--only", "scroll_dropped_pct")
        first = [s for s in h.snapshots if s["configs"]][0]
        self.assertEqual(first["configs"][0]["metrics"]["scroll_dropped_pct"]["zed"], [])

    def test_profiles_wiped_then_written_each_config(self):
        h = Harness(self)
        h.main("--rounds", "1", "--lsp", "both", "--only", "app_size_mb")
        seq = [c for c in h.calls if c[0] in ("reset", "profiles")]
        self.assertEqual(seq, [("reset", None), ("profiles", False),
                               ("reset", None), ("profiles", True)])

    def test_lsp_without_vscode_extension_aborts_up_front(self):
        h = Harness(self)
        h.ra_installed = False
        with mock.patch.object(run, "_log") as lg:
            self.assertEqual(h.main("--rounds", "1", "--lsp", "on"), 1)
        self.assertEqual([c for c in h.calls if c[0] not in ("profiles", "reset")], [])
        self.assertIn("rust-analyzer", " ".join(c.args[0] for c in lg.call_args_list))

    def test_lsp_off_does_not_need_vscode_extension(self):
        h = Harness(self)
        h.ra_installed = False
        self.assertEqual(h.main("--rounds", "1", "--only", "scroll_dropped_pct"), 0)

    def test_missing_lucerna_binary_aborts_before_anything(self):
        h = Harness(self)
        h.lucerna = run.editors.BinaryMissing("không tìm thấy `luc`")
        with mock.patch.object(run, "_log") as lg:
            self.assertEqual(h.main("--rounds", "1"), 1)
        self.assertEqual(h.calls, [])
        self.assertIn("luc", lg.call_args_list[-1].args[0])

    def test_binary_vanishing_mid_run_stops_and_saves(self):
        h = Harness(self)
        n = {"i": 0}

        def vanish(editor, paths):
            n["i"] += 1
            if n["i"] > 2:
                raise run.editors.BinaryMissing("binary Lucerna /x/luc không tồn tại")
            return 1.0
        orig = h.metric
        h.metric = lambda name: vanish if name == "scroll_dropped_pct" else orig(name)
        self.assertEqual(h.main("--rounds", "2", "--only", "scroll_dropped_pct"), 1)
        self.assertEqual(n["i"], 3)       # stopped at once, not recorded as null
        self.assertTrue(h.out.exists())

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
        names = {c[0] for c in h.calls} - {"profiles", "reset"}
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
            self.assertEqual(h.count("idle_session", e), 4)   # warm-up + 3
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
        self.assertEqual([c for c in h.calls if c[0] not in ("profiles", "reset")], [])

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
            if n["i"] > 7:   # 3 warm-up calls first
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
        self.assertEqual(h.sleeps.count(20), 5)  # 3 warm-up + 3 runs, rest between

    def test_app_size_failure_is_null(self):
        h = Harness(self)
        h.app_size = mock.Mock(side_effect=OSError("x"))
        self.assertEqual(h.main("--rounds", "1", "--only", "app_size_mb"), 0)
        self.assertEqual(h.result()["configs"][0]["metrics"]["app_size_mb"]["zed"], [None])

    def test_ignore_preflight_continues_and_records(self):
        h = Harness(self, preflight=lambda: ["build running"])
        rc = h.main("--rounds", "2", "--ignore-preflight", "--only", "scroll_dropped_pct")
        self.assertEqual(rc, 0)
        self.assertEqual(h.sleeps.count(30), 0)
        self.assertEqual(h.count("scroll_dropped_pct"), 9)   # warm-up + 2 rounds
        ign = h.result()["meta"]["preflight_ignored"]
        self.assertEqual([x["round"] for x in ign], [0, 1, 2])
        self.assertEqual(ign[0]["reasons"], ["build running"])

    def test_no_preflight_ignored_key_without_flag(self):
        h = Harness(self)
        h.main("--rounds", "1", "--only", "scroll_dropped_pct")
        self.assertNotIn("preflight_ignored", h.result()["meta"])

    def test_unexpected_error_saves_partial_nonzero(self):
        h = Harness(self)
        err = io.StringIO()
        with mock.patch.object(run, "wait_quiet", side_effect=RuntimeError("boom")), \
                mock.patch.object(run.sys, "stderr", err):
            self.assertNotEqual(h.main("--rounds", "1", "--only", "scroll_dropped_pct"), 0)
        self.assertTrue(h.out.exists())
        self.assertIn("Traceback", err.getvalue())

    def test_failed_metric_logs_traceback(self):
        h = Harness(self)

        def boom(editor, paths):
            raise ValueError("bad frame")
        h.metric = lambda name: boom
        err = io.StringIO()
        with mock.patch.object(run.sys, "stderr", err):
            self.assertEqual(h.main("--rounds", "1", "--only", "scroll_dropped_pct"), 0)
        self.assertIn("Traceback", err.getvalue())
        self.assertIn("ValueError: bad frame", err.getvalue())

    def test_eof_on_cold_prompt(self):
        h = Harness(self)
        with mock.patch("builtins.input", side_effect=EOFError):
            self.assertNotEqual(h.main("--cold", "--only", "startup_ms"), 0)
        self.assertTrue(h.out.exists())


class FakeHelper(unittest.TestCase):
    def test_check_exit3_names_missing_permission(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        fake = Path(tmp.name) / "helper"
        fake.write_text('#!/bin/sh\necho \'{"accessibility":true,"screen_recording":false}\'\nexit 3\n')
        fake.chmod(0o755)
        paths = run.BenchPaths(Path(tmp.name))
        with mock.patch.dict("os.environ", {"BENCH_HELPER": str(fake)}):
            self.assertEqual(run.helper_permissions(paths),
                             {"accessibility": True, "screen_recording": False})
            with self.assertRaises(run.Abort) as cm:
                run.check_permissions(paths, ["startup_ms", "idle_ram_mb", "app_size_mb"])
        msg = str(cm.exception)
        self.assertIn("Screen Recording", msg)
        self.assertIn("startup_ms", msg)
        self.assertIn("--only idle_ram_mb,app_size_mb", msg)

    def test_check_unusable_helper_gives_none(self):
        paths = run.BenchPaths(Path("/nonexistent-bench"))
        self.assertIsNone(run.helper_permissions(paths))

    def test_midrun_denied_without_detail_has_fallback(self):
        h = Harness(self)
        with mock.patch.object(run, "wait_quiet",
                               side_effect=run.metrics.PermissionDenied("bench-helper check: ")):
            with mock.patch.object(run, "_log") as lg:
                self.assertEqual(h.main("--rounds", "1", "--only", "scroll_dropped_pct"), 1)
        self.assertNotIn("check: \n", lg.call_args_list[-1][0][0])
        self.assertIn("Screen Recording", lg.call_args_list[-1][0][0])


class Permissions(unittest.TestCase):
    def test_needs_derived_from_helper_table(self):
        self.assertEqual(set(run.NEEDS_SCREEN),
                         {"startup_ms", "scroll_dropped_pct", "file_change_ms", "quick_open_ms"})
        self.assertEqual(set(run.NEEDS_AX),
                         {"startup_ms", "scroll_dropped_pct", "file_change_ms",
                          "quick_open_ms", "idle_ram_mb", "idle_cpu_pct"})

    def test_actual_window_frames_once_per_editor_per_config(self):
        h = Harness(self)
        orig = h.metric

        def framed(name):
            fn = orig(name)

            def wrapped(editor, paths):
                run.metrics.LAST_FRAME[editor] = {"x": 0, "y": 38, "w": 1280, "h": 800}
                return fn(editor, paths)
            return wrapped
        h.metric = framed
        self.addCleanup(run.metrics.LAST_FRAME.clear)
        h.main("--rounds", "2", "--lsp", "both", "--only", "scroll_dropped_pct")
        frames = h.result()["meta"]["window_frames"]
        self.assertEqual(set(frames), set(editors.EDITORS))
        self.assertEqual(frames["zed"], [
            {"lsp": False, "cold": False, "x": 0, "y": 38, "w": 1280, "h": 800},
            {"lsp": True, "cold": False, "x": 0, "y": 38, "w": 1280, "h": 800}])

    def test_missing_accessibility_blocks_scroll(self):
        h = Harness(self, perms={"screen_recording": True, "accessibility": False})
        with mock.patch.object(run, "_log") as lg:
            self.assertEqual(h.main("--rounds", "1", "--only", "scroll_dropped_pct"), 1)
        self.assertEqual(h.count("scroll_dropped_pct"), 0)
        self.assertIn("Accessibility", lg.call_args_list[-1].args[0])

    def test_meta_records_binary_helper_display_window(self):
        with tempfile.TemporaryDirectory() as d:
            paths = run.BenchPaths(Path(d))
            fake = Path(d) / "helper"
            fake.write_text('#!/bin/sh\necho \'{"width": 1470, "height": 956, "scale": 2}\'\n')
            fake.chmod(0o755)
            rec = {"path": "/x/luc", "mtime": "T", "version": "luc 1"}
            with mock.patch.dict("os.environ", {"BENCH_HELPER": str(fake)}), \
                    mock.patch.object(run.editors, "resolve_lucerna", return_value=rec), \
                    mock.patch.object(run.editors, "version", return_value="v"):
                meta = run.collect_meta([False], paths)
        self.assertEqual(meta["lucerna_binary"], rec)
        self.assertEqual(meta["helper"]["path"], str(fake))
        self.assertIsNotNone(meta["helper"]["mtime"])
        self.assertEqual(meta["display"], {"width": 1470, "height": 956, "scale": 2})
        self.assertEqual(meta["window_frame"], {"x": 0, "y": 25, "w": 1280, "h": 800})


class ForeignZed(unittest.TestCase):
    def test_aborts_up_front_when_users_zed_runs(self):
        h = Harness(self)
        h.foreign = [4242]
        self.assertEqual(h.main("--rounds", "1"), 1)
        self.assertEqual(h.calls, [])

    def test_app_size_only_does_not_need_zed_closed(self):
        h = Harness(self)
        h.foreign = [4242]
        self.assertEqual(h.main("--only", "app_size_mb"), 0)

    def test_foreign_raised_mid_run_saves_partial_and_fails(self):
        h = Harness(self)

        def boom(editor, paths):
            raise run.editors.ForeignInstance("Zed của bạn đang chạy (pid 1)")
        h.metric = lambda name: boom
        self.assertEqual(h.main("--rounds", "1", "--only", "startup_ms"), 1)
        self.assertTrue(h.out.exists())


class Cli(unittest.TestCase):
    def test_help_under_isolated_mode(self):
        r = subprocess.run([sys.executable, "-I", str(Path(run.__file__)), "--help"],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)


if __name__ == "__main__":
    unittest.main()
