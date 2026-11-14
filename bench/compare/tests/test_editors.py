import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import editors  # noqa: E402
import env  # noqa: E402

PATHS = env.BenchPaths(Path("/bench"))


class LaunchTests(unittest.TestCase):
    def test_launch_command_vscode_uses_bench_profile(self):
        with mock.patch.object(editors.subprocess, "Popen") as popen:
            launched = editors.launch("vscode", PATHS, Path("/bench/data/medium"))
        cmd = popen.call_args.args[0]
        self.assertIn("--user-data-dir", cmd)
        self.assertEqual(cmd[cmd.index("--user-data-dir") + 1], "/bench/profiles/vscode")
        self.assertEqual(cmd[cmd.index("--extensions-dir") + 1], "/bench/profiles/vscode-ext")
        self.assertIn("/bench/data/medium", cmd)
        self.assertIsInstance(launched.t0_ns, int)

    def test_lucerna_env_home(self):
        with tempfile.TemporaryDirectory() as d:
            paths = env.BenchPaths(Path(d))
            with mock.patch.object(editors.subprocess, "Popen") as popen, \
                    mock.patch.object(editors, "_lucerna_bin", return_value="/x/luc"), \
                    mock.patch.object(editors, "_lstart", return_value="T1"):
                popen.return_value.pid = 123
                editors.launch("lucerna", paths, Path("/bench/data/medium"))
            self.assertEqual(popen.call_args.kwargs["env"]["HOME"], str(paths.lucerna_home))
            rec = json.loads((paths.lucerna_home / editors.PIDFILE_NAME).read_text())
            self.assertEqual(rec, {"pid": 123, "lstart": "T1"})
        self.assertEqual(popen.call_args.args[0][0], "/x/luc")


class PidTests(unittest.TestCase):
    def test_bench_pids_ignores_users_own_session(self):
        table = [
            (100, 1, "/Applications/Zed.app/Contents/MacOS/zed"),
            (101, 100, "/Applications/Zed.app/Contents/MacOS/zed --helper"),
            (200, 1, "/Applications/Zed.app/Contents/MacOS/zed --user-data-dir /bench/profiles/zed"),
            (201, 200, "/Applications/Zed.app/Contents/MacOS/zed --helper"),
            (300, 1, "/Applications/Zed.app/Contents/MacOS/zed --user-data-dir /bench/profiles/zed-other"),
        ]
        with mock.patch.object(editors, "_ps", return_value=table):
            self.assertEqual(sorted(editors.bench_pids("zed", PATHS)), [200, 201])

    def test_user_data_dir_equals_form(self):
        table = [(5, 1, "zed --user-data-dir=/bench/profiles/zed"),
                 (6, 1, "zed --user-data-dir=/bench/profiles/zed2")]
        with mock.patch.object(editors, "_ps", return_value=table):
            self.assertEqual(editors.bench_pids("zed", PATHS), [5])

    def _lucerna(self, started, table, write=True):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        paths = env.BenchPaths(Path(d.name))
        if write:
            paths.lucerna_home.mkdir(parents=True)
            (paths.lucerna_home / editors.PIDFILE_NAME).write_text(
                json.dumps({"pid": 50, "lstart": started}))
        with mock.patch.object(editors, "_ps", return_value=table), \
                mock.patch.object(editors, "_lstart", return_value="T1"):
            return editors.bench_pids("lucerna", paths)

    TABLE = [(50, 1, "/x/Lucerna.app/Contents/MacOS/luc /w"),
             (51, 50, "helper"), (60, 1, "/usr/bin/luc /mine")]

    def test_lucerna_pidfile_match(self):
        self.assertEqual(self._lucerna("T1", self.TABLE), [50, 51])

    def test_lucerna_reused_pid_excluded(self):
        self.assertEqual(self._lucerna("OLD", self.TABLE), [])

    def test_lucerna_without_pidfile_excluded(self):
        self.assertEqual(self._lucerna("T1", self.TABLE, write=False), [])


class QuitTests(unittest.TestCase):
    def test_quit_kills_snapshotted_orphan(self):
        table = [(7, 1, "zed --user-data-dir /bench/profiles/zed")]
        kills = []
        with mock.patch.object(editors, "_ps", return_value=table), \
                mock.patch.object(editors, "_lstart", return_value="T1"), \
                mock.patch.object(editors.os, "kill", lambda p, s: kills.append((p, s))), \
                mock.patch.object(editors, "_reap"), \
                mock.patch.object(editors, "QUIT_GRACE_S", 0.2):
            editors.quit("zed", PATHS)
        import signal
        self.assertEqual(kills, [(7, signal.SIGTERM), (7, signal.SIGKILL)])

    def test_quit_skips_reused_pid(self):
        table = [(7, 1, "zed --user-data-dir /bench/profiles/zed")]
        starts = iter(["T1"])
        kills = []
        with mock.patch.object(editors, "_ps", return_value=table), \
                mock.patch.object(editors, "_lstart", lambda p: next(starts, "T2")), \
                mock.patch.object(editors.os, "kill", lambda p, s: kills.append((p, s))), \
                mock.patch.object(editors, "_reap"), \
                mock.patch.object(editors, "QUIT_GRACE_S", 0.2):
            editors.quit("zed", PATHS)
        self.assertEqual([s for _, s in kills], [15])


if __name__ == "__main__":
    unittest.main()
