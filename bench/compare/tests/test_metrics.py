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


def done(rc, stdout="", stderr=""):
    return subprocess.CompletedProcess([], rc, stdout=stdout, stderr=stderr)


class FakeWatch:
    def __init__(self, result, on_result=None):
        self._result = result
        self._on_result = on_result

    def result(self, timeout):
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
            self.assertIsNone(metrics.startup_ms("zed", self.paths))
        self.quit.assert_called_once_with("zed", self.paths)

    def test_missing_permission_raises(self):
        with mock.patch.object(metrics.subprocess, "run",
                               return_value=done(3, stderr="no Screen Recording")):
            with self.assertRaises(metrics.PermissionDenied):
                metrics.startup_ms("zed", self.paths)
        self.quit.assert_called_once_with("zed", self.paths)

    def test_helper_path_override(self):
        with mock.patch.dict(metrics.os.environ, {"BENCH_HELPER": "/x/h"}):
            self.assertEqual(metrics.helper_path(self.paths), "/x/h")
        with mock.patch.dict(metrics.os.environ, {}, clear=True):
            self.assertEqual(metrics.helper_path(self.paths),
                             str(self.paths.root / "bin" / "bench-helper"))


class StartupTests(Base):
    def test_startup_uses_marker_and_coverage(self):
        out = {"t_ns": 1_250_000_000, "frames": 9, "first_frame_hit": False}
        with mock.patch.object(metrics, "_helper", return_value=out) as helper:
            self.assertEqual(metrics.startup_ms("vscode", self.paths), 250.0)
        self.launch.assert_called_once_with("vscode", self.paths, self.paths.marker_file)
        args = helper.call_args.args[1]
        self.assertEqual(args[0], "watch")
        self.assertEqual(args[args.index("--pid") + 1], "11,12")
        self.assertEqual(args[args.index("--since") + 1], "1000000000")
        self.assertEqual(args[args.index("--until") + 1], "coverage:0.3")
        self.assertEqual(args[args.index("--background") + 1], "1E1E1E")
        self.quit.assert_called_once_with("vscode", self.paths)

    def test_first_frame_hit_returns_none(self):
        out = {"t_ns": 1_250_000_000, "frames": 1, "first_frame_hit": True}
        with mock.patch.object(metrics, "_helper", return_value=out):
            self.assertIsNone(metrics.startup_ms("zed", self.paths))


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
        with mock.patch.object(metrics, "_helper", return_value=shown), \
                mock.patch.object(metrics, "_start_watch", return_value=watch) as start, \
                mock.patch.object(editors, "now_ns", side_effect=clock):
            self.assertEqual(metrics.file_change_ms("lucerna", self.paths), 12.0)
        self.launch.assert_called_once_with("lucerna", self.paths, reload)
        self.assertGreater(seen["size_at_t_w"], 0)
        self.assertEqual(seen["size_at_result"], 0)
        self.assertIn("change", start.call_args.args[1])
        # The shared marker is never destroyed; the per-run copy is restored.
        self.assertGreater(self.paths.marker_file.stat().st_size, 0)
        self.assertEqual(reload.read_bytes(), self.paths.marker_file.read_bytes())
        self.quit.assert_called_once()


class QuickOpenTests(Base):
    def test_measured_from_last_key(self):
        calls = []

        def helper(paths, args, timeout=metrics.TIMEOUT_S):
            calls.append(args[0] + ":" + (args[-1] if args[0] == "type" else ""))
            if args[0] == "type" and args[-1] == "2":
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
        self.assertEqual(calls, ["key:", "type:mod004", "watch", "type:2"])
        self.launch.assert_called_once_with("zed", self.paths, self.paths.large_repo)
        metrics.time.sleep.assert_any_call(metrics.SCAN_WAIT_S)

    def test_change_before_last_key_is_none(self):
        with mock.patch.object(metrics, "_helper", return_value={"t_ns": [50]}), \
                mock.patch.object(metrics, "_start_watch",
                                  return_value=FakeWatch({"t_ns": 10, "frames": 2})):
            self.assertIsNone(metrics.quick_open_ms("zed", self.paths))


class IdleTests(Base):
    def run_idle(self):
        ps_rows = iter(["  1.5\n  0.5\n"] + ["  0.0\n  1.0\n"] * (metrics.IDLE_S - 1))
        fp = (FIXTURES / "footprint.txt").read_text()

        def run(cmd, **kw):
            if cmd[0] == "footprint":
                return done(0, fp)
            if cmd[0] == "ps":
                return done(0, next(ps_rows))
            raise AssertionError(cmd)

        with mock.patch.object(metrics, "_helper", return_value={"t_ns": 1}) as helper, \
                mock.patch.object(metrics.subprocess, "run", side_effect=run) as sub:
            result = metrics.idle_session("vscode", self.paths)
        return result, helper, sub

    def test_idle_session_ram_and_cpu(self):
        (ram, cpu), helper, sub = self.run_idle()
        self.assertAlmostEqual(ram, 852160 * 2 / 1024 ** 2)
        self.assertAlmostEqual(cpu, (2.0 + 1.0 * (metrics.IDLE_S - 1)) / metrics.IDLE_S)
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


class ScrollTests(Base):
    def test_dropped_percent(self):
        outs = [{"t_ns": 1, "frames": 5, "first_frame_hit": False},
                {"frames": 200, "dropped": 3, "refresh_hz": 120, "rate": 120}]
        with mock.patch.object(metrics, "_helper", side_effect=outs) as helper:
            self.assertEqual(metrics.scroll_dropped_pct("zed", self.paths), 1.5)
        args = helper.call_args.args[1]
        self.assertEqual(args[:1], ["scroll"])
        self.assertEqual(args[args.index("--lines") + 1], "3000")
        self.assertEqual(args[args.index("--rate") + 1], "120")
        self.launch.assert_called_once_with("zed", self.paths, self.paths.big_file)

    def test_background_fetched_before_launch(self):
        # Calibrating launches and quits the editor: never inside a session.
        order = []
        metrics.background.side_effect = lambda e, p: order.append("bg") or "1E1E1E"
        self.launch.side_effect = lambda *a: order.append("launch") or \
            editors.Launched(t0_ns=1, popen=None)
        with mock.patch.object(metrics, "_helper", return_value=None), \
                mock.patch.object(metrics, "_start_watch", return_value=None):
            metrics.scroll_dropped_pct("zed", self.paths)
            metrics.file_change_ms("zed", self.paths)
            metrics.startup_ms("zed", self.paths)
        self.assertEqual(order, ["bg", "launch"] * 3)

    def test_zero_frames_is_none(self):
        outs = [{"t_ns": 1, "frames": 5}, {"frames": 0, "dropped": 0}]
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

    def test_calibrate_samples_empty_file(self):
        with mock.patch.object(metrics, "_helper",
                               return_value={"center_dominant": "FFFFFF"}) as helper:
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


if __name__ == "__main__":
    unittest.main()
