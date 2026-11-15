import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import editors  # noqa: E402
import env  # noqa: E402
import metrics  # noqa: E402

FIXTURES = Path(__file__).resolve().parent / "fixtures"
EXE = "/A/X.app/Contents/MacOS/x"
FRAME_OK = {"x": 0, "y": 38, "w": 1280, "h": 800, "pid": 11}


def done(rc, stdout="", stderr=""):
    return subprocess.CompletedProcess([], rc, stdout=stdout, stderr=stderr)


class FakeWatch:
    def __init__(self, result, on_result=None):
        self._result = result
        self._on_result = on_result
        self.closed = False
        self.waited = False

    def close(self):
        self.closed = True

    def result(self, timeout):
        self.waited = True
        if self._on_result:
            self._on_result()
        return self._result


class Base(unittest.TestCase):
    fake_background = True

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.paths = env.BenchPaths(Path(self._tmp.name))
        self.paths.data.mkdir(parents=True)
        self.paths.marker_file.write_text("█" * 80 + "\n", encoding="utf-8")
        self.launch = self.patch(editors, "launch",
                                 return_value=editors.Launched(t0_ns=1_000_000_000, popen=None))
        self.quit = self.patch(editors, "quit")
        self.patch(editors, "bench_pids", return_value=[11, 12])
        self.patch(metrics, "_owner_pids", return_value=[11, 12])
        self.patch(editors, "main_executable", return_value=EXE)
        self.running = self.patch(editors, "running_pids", return_value=[])
        if self.fake_background:
            self.patch(metrics, "background", return_value="1E1E1E")
        self.patch(metrics.time, "sleep")

    def tearDown(self):
        self._tmp.cleanup()

    def patch(self, target, name, **kw):
        p = mock.patch.object(target, name, **kw)
        self.addCleanup(p.stop)
        return p.start()


class FootprintTests(unittest.TestCase):
    def test_parse_footprint_sums_processes(self):
        # Recorded with `footprint -f bytes --noCategories -p <sleep> -p <sleep>`.
        text = (FIXTURES / "footprint.txt").read_text()
        self.assertEqual(metrics.parse_footprint(text), 852160 * 2)

    def test_parse_footprint_formatted_units(self):
        text = ("    phys_footprint: 832 KB\n    phys_footprint_peak: 848 KB\n"
                "    phys_footprint: 1.5 MB\n")
        self.assertEqual(metrics.parse_footprint(text), 832 * 1024 + int(1.5 * 1024 ** 2))


class HelperTests(Base):
    def test_metric_timeout_returns_none(self):
        with mock.patch.object(metrics.subprocess, "run", return_value=done(2, stderr="timeout")):
            self.assertIsNone(metrics.scroll_dropped_pct("zed", self.paths))
        self.quit.assert_called_once_with("zed", self.paths)

    def test_missing_permission_raises(self):
        with mock.patch.object(metrics.subprocess, "run",
                               return_value=done(3, stderr="no Accessibility")):
            with self.assertRaises(metrics.PermissionDenied):
                metrics.scroll_dropped_pct("zed", self.paths)
        self.quit.assert_called_once_with("zed", self.paths)

    def test_helper_path_override(self):
        with mock.patch.dict(metrics.os.environ, {"BENCH_HELPER": "/x/h"}):
            self.assertEqual(metrics.helper_path(self.paths), "/x/h")
        with mock.patch.dict(metrics.os.environ, {}, clear=True):
            self.assertEqual(metrics.helper_path(self.paths),
                             str(self.paths.root / "bin" / "bench-helper"))


class StartupTests(Base):
    def run_startup(self, out, editor="vscode"):
        order = []
        watch = FakeWatch(out)
        self.launch.side_effect = lambda *a: order.append("launch") or \
            editors.Launched(t0_ns=1_000_000_000, popen=None)

        def start(paths, args, timeout=metrics.TIMEOUT_S):
            order.append("watch")
            return watch

        def helper(paths, args, timeout=metrics.TIMEOUT_S):
            order.append(args[0])
            return {"pid": 11, "active": True}

        with mock.patch.object(metrics, "_start_watch", side_effect=start) as st, \
                mock.patch.object(metrics, "_helper", side_effect=helper) as h:
            value = metrics.startup_ms(editor, self.paths)
        # only `activate`, with the owner pids, from the background thread
        self.assertEqual([c.args[1] for c in h.call_args_list],
                         [["activate", "--pid", "11,12", "--timeout", "10"]])
        return value, st.call_args.args[1], order, watch

    def test_display_watch_started_before_launch(self):
        out = {"t_ns": 1_250_000_000, "frames": 9, "first_frame_hit": False}
        value, args, order, watch = self.run_startup(out)
        self.assertEqual(value, 250.0)
        self.assertEqual(order, ["watch", "launch", "activate"])
        self.launch.assert_called_once_with("vscode", self.paths, self.paths.marker_file)
        self.assertEqual(args[:4], ["watch", "--display", "--exe", EXE])
        self.assertNotIn("--pid", args)
        self.assertNotIn("--exclude-pids", args)
        # t0 is unknown when the helper starts: absolute times, Python subtracts.
        self.assertEqual(args[args.index("--since") + 1], "0")
        self.assertEqual(args[args.index("--until") + 1], "coverage:0.3")
        self.assertEqual(args[args.index("--background") + 1], "1E1E1E")
        self.assertTrue(watch.closed)
        self.quit.assert_called_once_with("vscode", self.paths)

    def test_running_instances_excluded(self):
        self.running.return_value = [70, 71]
        _, args, _, _ = self.run_startup({"t_ns": 1_100_000_000, "frames": 3})
        self.running.assert_called_once_with(EXE)
        self.assertEqual(args[args.index("--exclude-pids") + 1], "70,71")

    def test_first_frame_hit_returns_none(self):
        out = {"t_ns": 1_250_000_000, "frames": 1, "first_frame_hit": True}
        self.assertIsNone(self.run_startup(out)[0])

    def test_frame_before_launch_returns_none(self):
        out = {"t_ns": 900_000_000, "frames": 4, "first_frame_hit": False}
        self.assertIsNone(self.run_startup(out)[0])

    def test_activation_permission_error_propagates(self):
        def helper(paths, args, timeout=metrics.TIMEOUT_S):
            raise metrics.PermissionDenied("bench-helper activate: no Accessibility")
        watch = FakeWatch({"t_ns": 1_100_000_000, "frames": 3})
        with mock.patch.object(metrics, "_start_watch", return_value=watch), \
                mock.patch.object(metrics, "_helper", side_effect=helper):
            with self.assertRaises(metrics.PermissionDenied):
                metrics.startup_ms("zed", self.paths)
        self.assertTrue(watch.closed)
        self.quit.assert_called_once()

    def test_activation_failure_keeps_measurement(self):
        watch = FakeWatch({"t_ns": 1_100_000_000, "frames": 3, "occluded_frames": 2})
        with mock.patch.object(metrics, "_start_watch", return_value=watch), \
                mock.patch.object(metrics, "_helper", return_value=None):
            self.assertEqual(metrics.startup_ms("zed", self.paths), 100.0)

    def test_watch_not_ready_skips_launch(self):
        with mock.patch.object(metrics, "_start_watch", return_value=None):
            self.assertIsNone(metrics.startup_ms("zed", self.paths))
        self.launch.assert_not_called()

    def test_foreign_instance_closes_watch_and_propagates(self):
        watch = FakeWatch({"t_ns": 1})
        self.launch.side_effect = editors.ForeignInstance("zed")
        with mock.patch.object(metrics, "_start_watch", return_value=watch):
            with self.assertRaises(editors.ForeignInstance):
                metrics.startup_ms("zed", self.paths)
        self.assertTrue(watch.closed)
        self.assertFalse(watch.waited)


class FileChangeTests(Base):
    def test_timing_taken_just_before_write(self):
        reload = self.paths.data / "reload.txt"
        seen = {}

        def clock():
            seen["size_at_t_w"] = reload.stat().st_size
            return 5_000_000_000

        def on_result():
            seen["size_at_result"] = reload.stat().st_size

        shown = {"t_ns": 1, "frames": 3, "first_frame_hit": False}
        watch = FakeWatch({"t_ns": 5_012_000_000, "frames": 4}, on_result)
        with mock.patch.object(metrics, "_helper", side_effect=[FRAME_OK, shown]), \
                mock.patch.object(metrics, "_start_watch", return_value=watch) as start, \
                mock.patch.object(editors, "now_ns", side_effect=clock):
            self.assertEqual(metrics.file_change_ms("lucerna", self.paths), 12.0)
        self.launch.assert_called_once_with("lucerna", self.paths, reload)
        self.assertGreater(seen["size_at_t_w"], 0)
        self.assertEqual(seen["size_at_result"], 0)
        self.assertIn("change", start.call_args.args[1])
        self.assertTrue(watch.closed)
        # The shared marker is never destroyed; the per-run copy is restored.
        self.assertGreater(self.paths.marker_file.stat().st_size, 0)
        self.assertEqual(reload.read_bytes(), self.paths.marker_file.read_bytes())
        self.quit.assert_called_once()


class QuickOpenTests(Base):
    def test_measured_from_last_key(self):
        calls = []

        def helper(paths, args, timeout=metrics.TIMEOUT_S):
            calls.append(args[0] + ":" + (args[-1] if args[0] == "type" else ""))
            if args[0] == "type" and args[-1] == "z":
                return {"t_ns": [7_000_000]}
            if args[0] == "type":
                return {"t_ns": [1, 2, 3, 4, 5, 6]}
            return {"t_ns": 1}

        def start(paths, args, timeout=metrics.TIMEOUT_S):
            calls.append("watch")
            return FakeWatch({"t_ns": 19_000_000, "frames": 2})

        with mock.patch.object(metrics, "_helper", side_effect=helper), \
                mock.patch.object(metrics, "_start_watch", side_effect=start):
            self.assertEqual(metrics.quick_open_ms("zed", self.paths), 12.0)
        # A watch confirms the picker opened after ⌘P, then the prefix, then
        # the measured watch and the last key.
        self.assertEqual(calls, ["frame:", "watch", "key:", "type:mod0042", "watch", "type:z"])
        metrics.time.sleep.assert_any_call(metrics.SETTLE_S / 2)
        self.launch.assert_called_once_with("zed", self.paths, self.paths.large_repo)
        metrics.time.sleep.assert_any_call(metrics.SCAN_WAIT_S)

    def test_settle_between_cmd_p_and_typing(self):
        events = []
        metrics.time.sleep.side_effect = lambda s: events.append(f"sleep {s}")

        def helper(paths, args, timeout=metrics.TIMEOUT_S):
            events.append(args[0])
            return {"t_ns": [1]} if args[0] == "type" else {"t_ns": 1}

        with mock.patch.object(metrics, "_helper", side_effect=helper), \
                mock.patch.object(metrics, "_start_watch", return_value=FakeWatch({"t_ns": 2})):
            metrics.quick_open_ms("zed", self.paths)
        i = events.index("key")
        self.assertEqual(events[i:i + 3], ["key", f"sleep {metrics.SETTLE_S / 2}", "type"])

    def test_failed_last_key_closes_watch_without_waiting(self):
        picker, watch = FakeWatch({"t_ns": 5}), FakeWatch({"t_ns": 99})

        def helper(paths, args, timeout=metrics.TIMEOUT_S):
            return None if args[0] == "type" and args[-1] == "z" else {"t_ns": [1]}

        with mock.patch.object(metrics, "_helper", side_effect=helper), \
                mock.patch.object(metrics, "_start_watch", side_effect=[picker, watch]):
            self.assertIsNone(metrics.quick_open_ms("zed", self.paths))
        self.assertTrue(watch.closed)
        self.assertFalse(watch.waited)

    def test_lost_cmd_p_is_sent_again(self):
        keys = []

        def helper(paths, args, timeout=metrics.TIMEOUT_S):
            if args[0] == "key":
                keys.append(args[-1])
            return {"t_ns": [7_000_000]} if args[0] == "type" else {"t_ns": 1}

        watches = [FakeWatch(None), FakeWatch({"t_ns": 2}),
                   FakeWatch({"t_ns": 19_000_000, "frames": 2})]
        with mock.patch.object(metrics, "_helper", side_effect=helper), \
                mock.patch.object(metrics, "_start_watch", side_effect=watches):
            self.assertEqual(metrics.quick_open_ms("zed", self.paths), 12.0)
        self.assertEqual(keys, ["cmd+p", "cmd+p"])
        self.assertTrue(all(w.closed for w in watches))

    def test_picker_never_opens_is_none_without_typing(self):
        typed = []

        def helper(paths, args, timeout=metrics.TIMEOUT_S):
            if args[0] == "type":
                typed.append(args[-1])
            return {"t_ns": 1}

        with mock.patch.object(metrics, "_helper", side_effect=helper), \
                mock.patch.object(metrics, "_start_watch",
                                  side_effect=[FakeWatch(None), FakeWatch(None)]):
            self.assertIsNone(metrics.quick_open_ms("zed", self.paths))
        self.assertEqual(typed, [])

    def test_change_before_last_key_is_none(self):
        with mock.patch.object(metrics, "_helper", return_value={"t_ns": [50]}), \
                mock.patch.object(metrics, "_start_watch",
                                  return_value=FakeWatch({"t_ns": 10, "frames": 2})):
            self.assertIsNone(metrics.quick_open_ms("zed", self.paths))


class IdleTests(Base):
    def run_idle(self):
        # pid 11 uses 0.3 s CPU per 1 s sample; 12 is idle.
        ps_rows = iter(f"11 0:{1 + 0.3 * i:05.2f}\n12 1:00.50\n"
                       for i in range(metrics.IDLE_S + 1))
        fp = (FIXTURES / "footprint.txt").read_text()

        def run(cmd, **kw):
            if cmd[0] == "footprint":
                return done(0, fp)
            if cmd[:3] == ["ps", "-o", "pid=,time="]:
                return done(0, next(ps_rows))
            raise AssertionError(cmd)

        with mock.patch.object(metrics, "_helper", return_value={"t_ns": 1}) as helper, \
                mock.patch.object(metrics.subprocess, "run", side_effect=run) as sub, \
                mock.patch.object(metrics.time, "monotonic", side_effect=range(10 ** 6)):
            result = metrics.idle_session("vscode", self.paths)
        return result, helper, sub

    def test_idle_session_ram_and_cpu(self):
        (ram, cpu), helper, sub = self.run_idle()
        self.assertAlmostEqual(ram, 852160 * 2 / 1024 ** 2)
        self.assertAlmostEqual(cpu, 30.0)
        self.assertEqual(helper.call_args_list[0].args[1][0], "frame")
        self.launch.assert_called_once_with("vscode", self.paths, self.paths.medium_repo)
        typed = [c.args[1][-1] for c in helper.call_args_list if c.args[1][0] == "type"]
        self.assertEqual([t for t in typed if t != "\n"], list(metrics.IDLE_FILES))
        self.assertEqual(len(metrics.IDLE_FILES), 10)
        fp_cmd = next(c.args[0] for c in sub.call_args_list if c.args[0][0] == "footprint")
        self.assertEqual(fp_cmd, ["footprint", "-f", "bytes", "--noCategories",
                                  "-p", "11", "-p", "12"])
        metrics.time.sleep.assert_any_call(metrics.IDLE_S)
        self.quit.assert_called_once()

    def test_public_wrappers(self):
        with mock.patch.object(metrics, "idle_session", return_value=(1.0, 2.0)):
            self.assertEqual(metrics.idle_ram_mb("zed", self.paths), 1.0)
            self.assertEqual(metrics.idle_cpu_pct("zed", self.paths), 2.0)


class CpuTests(Base):
    def test_parse_cputime(self):
        self.assertAlmostEqual(metrics.parse_cputime("0:01.25"), 1.25)
        self.assertAlmostEqual(metrics.parse_cputime("2210:49.57"), 2210 * 60 + 49.57)
        self.assertAlmostEqual(metrics.parse_cputime("1:02:03.5"), 3723.5)
        self.assertAlmostEqual(metrics.parse_cputime("2-00:00:01.00"), 2 * 86400 + 1)

    def test_cpu_from_cumulative_time_with_pid_refresh(self):
        # pid 20 appears at sample 2 (counts from 0); pid 21 exits after
        # sample 1 (counts to its last value); 22 is constant.
        rows = {0: {21: 5.0, 22: 9.0}, 1: {21: 5.5, 22: 9.0}}
        idle = metrics.IDLE_S

        def times(pids):
            i = state["i"]
            state["i"] += 1
            if i in rows:
                return rows[i]
            return {20: 0.1 * (i - 1), 22: 9.0}
        state = {"i": 0}
        pid_lists = []
        self.patch(editors, "bench_pids",
                   side_effect=lambda e, p: pid_lists.append(1) or [20, 21, 22])
        with mock.patch.object(metrics, "_cpu_times", side_effect=times), \
                mock.patch.object(metrics.time, "monotonic", side_effect=range(10 ** 6)):
            cpu = metrics._cpu_pct("zed", self.paths)
        self.assertEqual(len(pid_lists), idle + 1)     # refreshed every sample
        used = 0.5 + 0.1 * (idle - 1)
        self.assertAlmostEqual(cpu, used / idle * 100)

    def test_no_process_is_none(self):
        self.patch(editors, "bench_pids", return_value=[])
        with mock.patch.object(metrics.time, "monotonic", side_effect=range(10 ** 6)):
            self.assertIsNone(metrics._cpu_pct("zed", self.paths))


class FrameTests(Base):
    def test_frame_set_before_measuring(self):
        outs = [FRAME_OK, {"t_ns": 1, "frames": 5}, {"frames": 10, "dropped": 1}]
        with mock.patch.object(metrics, "_helper", side_effect=outs) as helper:
            metrics.scroll_dropped_pct("zed", self.paths)
        args = helper.call_args_list[0].args[1]
        f = metrics.WINDOW_FRAME
        self.assertEqual(args[:3], ["frame", "--pid", "11,12"])
        for k in ("x", "y", "w", "h"):
            self.assertEqual(args[args.index("--" + k) + 1], str(f[k]))
        self.assertEqual((f["w"], f["h"]), (1280, 800))

    def test_actual_frame_remembered(self):
        metrics.LAST_FRAME.clear()
        self.addCleanup(metrics.LAST_FRAME.clear)
        outs = [FRAME_OK, {"t_ns": 1, "frames": 5}, {"frames": 10, "dropped": 1}]
        with mock.patch.object(metrics, "_helper", side_effect=outs):
            metrics.scroll_dropped_pct("zed", self.paths)
        self.assertEqual(metrics.LAST_FRAME["zed"], {"x": 0, "y": 38, "w": 1280, "h": 800})

    def test_frame_failure_drops_round(self):
        for fn in (metrics.scroll_dropped_pct, metrics.file_change_ms, metrics.quick_open_ms):
            with self.subTest(fn=fn.__name__), \
                    mock.patch.object(metrics, "_helper", return_value=None) as helper, \
                    mock.patch.object(metrics, "_start_watch") as start:
                self.assertIsNone(fn("zed", self.paths))
                self.assertEqual([c.args[1][0] for c in helper.call_args_list], ["frame"])
                start.assert_not_called()
        with mock.patch.object(metrics, "_helper", return_value=None):
            self.assertEqual(metrics.idle_session("zed", self.paths), (None, None))


class HelperTableTests(Base):
    """Each metric runs exactly the helper subcommands METRIC_HELPERS lists,
    so run.py's permission check (derived from it) is right."""

    def used(self, name):
        cmds = set()
        replies = {"frame": FRAME_OK, "watch": {"t_ns": 1, "frames": 3},
                   "key": {"t_ns": 1}, "type": {"t_ns": [1]},
                   "scroll": {"frames": 10, "dropped": 1}, "activate": {"pid": 11}}

        def helper(paths, args, timeout=metrics.TIMEOUT_S):
            cmds.add(args[0])
            return replies[args[0]]

        def start(paths, args, timeout=metrics.TIMEOUT_S):
            cmds.add(args[0])
            return FakeWatch({"t_ns": 2_000_000_000, "frames": 3})

        metrics.background.side_effect = lambda e, p: cmds.add("sample") or "1E1E1E"
        fp = (FIXTURES / "footprint.txt").read_text()
        with mock.patch.object(metrics, "_helper", side_effect=helper), \
                mock.patch.object(metrics, "_start_watch", side_effect=start), \
                mock.patch.object(metrics, "_cpu_pct", return_value=1.0), \
                mock.patch.object(metrics.subprocess, "run", return_value=done(0, fp)):
            if name in ("idle_ram_mb", "idle_cpu_pct"):
                getattr(metrics, name)("zed", self.paths)
            elif name == "app_size_mb":
                with mock.patch.object(editors, "app_bundle", return_value=Path("/A.app")), \
                        mock.patch.object(metrics.subprocess, "run",
                                          return_value=done(0, "1\t/A.app\n")):
                    metrics.app_size_mb("zed")
            else:
                getattr(metrics, name)("zed", self.paths)
        return cmds

    def test_table_matches_calls(self):
        for name, listed in metrics.METRIC_HELPERS.items():
            with self.subTest(metric=name):
                self.assertEqual(self.used(name), set(listed))

    def test_permissions(self):
        self.assertEqual(metrics.metric_permissions("scroll_dropped_pct"),
                         {"screen", "accessibility"})
        self.assertEqual(metrics.metric_permissions("startup_ms"),
                         {"screen", "accessibility"})
        self.assertEqual(metrics.metric_permissions("idle_ram_mb"), {"accessibility"})
        self.assertEqual(metrics.metric_permissions("app_size_mb"), set())


class ScrollTests(Base):
    def test_dropped_percent(self):
        outs = [FRAME_OK, {"t_ns": 1, "frames": 5, "first_frame_hit": False},
                {"frames": 200, "dropped": 3, "refresh_hz": 120, "rate": 120}]
        with mock.patch.object(metrics, "_helper", side_effect=outs) as helper:
            self.assertEqual(metrics.scroll_dropped_pct("zed", self.paths), 1.5)
        args = helper.call_args.args[1]
        self.assertEqual(args[:1], ["scroll"])
        # Continuous pixel events at the display rate (no --rate): line
        # events measured how editors batch wheel ticks (docs, "Scrolling").
        self.assertEqual(args[args.index("--unit") + 1], "pixel")
        self.assertEqual(args[args.index("--lines") + 1], "600")
        self.assertEqual(args[args.index("--pixels-per-event") + 1], "40")
        self.assertNotIn("--rate", args)
        self.assertLessEqual(int(args[args.index("--timeout") + 1]), metrics.TIMEOUT_S)
        self.launch.assert_called_once_with("zed", self.paths, self.paths.big_file)

    def test_background_fetched_before_launch(self):
        # Calibrating launches and quits the editor: never inside a session.
        order = []
        metrics.background.side_effect = lambda e, p: order.append("bg") or "1E1E1E"
        self.launch.side_effect = lambda *a: order.append("launch") or \
            editors.Launched(t0_ns=1, popen=None)
        with mock.patch.object(metrics, "_helper", return_value=None), \
                mock.patch.object(metrics, "_start_watch", return_value=FakeWatch(None)):
            metrics.scroll_dropped_pct("zed", self.paths)
            metrics.file_change_ms("zed", self.paths)
            metrics.startup_ms("zed", self.paths)
        self.assertEqual(order, ["bg", "launch"] * 3)

    def test_zero_frames_is_none(self):
        outs = [FRAME_OK, {"t_ns": 1, "frames": 5}, {"frames": 0, "dropped": 0}]
        with mock.patch.object(metrics, "_helper", side_effect=outs):
            self.assertIsNone(metrics.scroll_dropped_pct("zed", self.paths))


class SizeTests(Base):
    def test_app_size_mb(self):
        with mock.patch.object(editors, "app_bundle", return_value=Path("/A.app")), \
                mock.patch.object(metrics.subprocess, "run",
                                  return_value=done(0, "2048\t/A.app\n")) as run:
            self.assertEqual(metrics.app_size_mb("zed"), 2.0)
        self.assertEqual(run.call_args.args[0], ["du", "-sk", "/A.app"])


class CalibrationTests(Base):
    fake_background = False

    def test_background_cached_by_version(self):
        with mock.patch.object(editors, "version", return_value="1.0"), \
                mock.patch.object(metrics, "calibrate", return_value="282C34") as cal:
            self.assertEqual(metrics.background("zed", self.paths), "282C34")
            self.assertEqual(metrics.background("zed", self.paths), "282C34")
        cal.assert_called_once()
        saved = json.loads((self.paths.root / "calibration.json").read_text())
        self.assertEqual(saved, {"zed 1.0": "282C34"})

    def test_calibration_rejects_mixed_centre(self):
        out = {"center_dominant": "FFFFFF", "center_dominant_frac": 0.6}
        with mock.patch.object(metrics, "_helper", return_value=out), \
                mock.patch.object(editors, "version", return_value="1.0"):
            self.assertIsNone(metrics.calibrate("zed", self.paths))
            self.assertIsNone(metrics.background("zed", self.paths))
            self.assertIsNone(metrics.startup_ms("zed", self.paths))
        self.assertFalse((self.paths.root / "calibration.json").exists())
        self.assertEqual(self.quit.call_count, 3)  # each calibration quits; no measured launch

    def test_calibrate_samples_empty_file(self):
        with mock.patch.object(metrics, "_helper",
                               return_value={"center_dominant": "FFFFFF",
                                             "center_dominant_frac": 0.97}) as helper:
            self.assertEqual(metrics.calibrate("lucerna", self.paths), "FFFFFF")
        empty = self.paths.data / "empty.txt"
        self.launch.assert_called_once_with("lucerna", self.paths, empty)
        self.assertEqual(empty.read_text(), "")
        self.assertEqual(helper.call_args.args[1][:3], ["sample", "--pid", "11,12"])
        self.quit.assert_called_once()


class RealProcessTests(unittest.TestCase):
    """_helper / _start_watch against a stand-in helper script."""

    def script(self, body):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        path = Path(d.name) / "bench-helper"
        path.write_text("#!/bin/sh\n" + body)
        path.chmod(0o755)
        p = mock.patch.dict(metrics.os.environ, {"BENCH_HELPER": str(path)})
        p.start()
        self.addCleanup(p.stop)
        return env.BenchPaths(Path(d.name))

    def test_watch_waits_for_ready_then_returns_json(self):
        paths = self.script('echo ready >&2; sleep 0.2; echo \'{"t_ns": 42}\'\n')
        watch = metrics._start_watch(paths, ["watch"], timeout=5)
        self.assertIsNotNone(watch)
        self.assertEqual(watch.result(5), {"t_ns": 42})

    def test_watch_permission_error_raises(self):
        paths = self.script("echo 'bench-helper: no Screen Recording' >&2; exit 3\n")
        with self.assertRaises(metrics.PermissionDenied):
            metrics._start_watch(paths, ["watch"], timeout=5)

    def test_helper_bad_json_is_none(self):
        paths = self.script("echo not-json\n")
        self.assertIsNone(metrics._helper(paths, ["now"], timeout=5))


class OwnerPidTests(unittest.TestCase):
    VSCODE = "/Applications/Visual Studio Code.app/Contents/MacOS/Code"
    ZED = "/Applications/Zed.app/Contents/MacOS"

    def owner(self, editor, rounds):
        """rounds: successive (pids, ps command lines) the poll sees."""
        seq = iter(rounds)
        state = {}

        def bench_pids(e, p):
            state["cur"] = next(seq, rounds[-1])
            return state["cur"][0]

        def run(cmd, **kw):
            return done(0, "".join(c + "\n" for c in state["cur"][1]))

        with mock.patch.object(editors, "bench_pids", side_effect=bench_pids), \
                mock.patch.object(metrics.subprocess, "run", side_effect=run) as sub, \
                mock.patch.object(metrics.time, "sleep"), \
                mock.patch.object(metrics.time, "monotonic", side_effect=range(10 ** 6)):
            return metrics._owner_pids(editor, env.BenchPaths(Path("/b")), 50), sub

    def test_vscode_waits_for_main_binary(self):
        helper = f"{self.VSCODE} Helper --type=renderer --user-data-dir /b/profiles/vscode"
        main = f"{self.VSCODE} --user-data-dir /b/profiles/vscode /b/data/marker.txt"
        pids, _ = self.owner("vscode", [([], []), ([2], [helper]), ([1, 2], [main, helper])])
        self.assertEqual(pids, [1, 2])

    def test_zed_cli_is_not_the_owner(self):
        cli = f"{self.ZED}/cli --user-data-dir /b/profiles/zed --new /b/data/marker.txt"
        main = f"{self.ZED}/zed --user-data-dir /b/profiles/zed"
        pids, _ = self.owner("zed", [([5], [cli]), ([5, 6], [cli, main])])
        self.assertEqual(pids, [5, 6])

    def test_zed_new_foreign_process_is_logged_loudly(self):
        cli = f"{self.ZED}/cli --user-data-dir /b/profiles/zed"
        with mock.patch.object(editors, "foreign_instances", return_value=[777]), \
                mock.patch.object(metrics, "_log") as log:
            pids, _ = self.owner("zed", [([5], [cli])])
        self.assertIsNone(pids)
        msgs = " ".join(c.args[0] for c in log.call_args_list)
        self.assertIn("777", msgs)
        self.assertIn("REAL", msgs)

    def test_zed_never_owner_times_out(self):
        cli = f"{self.ZED}/cli --user-data-dir /b/profiles/zed"
        pids, _ = self.owner("zed", [([5], [cli])])
        self.assertIsNone(pids)

    def test_lucerna_first_pid(self):
        pids, sub = self.owner("lucerna", [([], []), ([9], ["/x/luc /b/data/marker.txt"])])
        self.assertEqual(pids, [9])
        sub.assert_not_called()


class WatchCloseTests(unittest.TestCase):
    def test_close_kills_running_watch(self):
        p = subprocess.Popen(["/bin/sleep", "30"], stdout=subprocess.PIPE, text=True)
        w = metrics._Watch(p, ["watch"], [])
        w.close()
        self.assertIsNotNone(p.poll())
        w.close()  # idempotent


if __name__ == "__main__":
    unittest.main()
