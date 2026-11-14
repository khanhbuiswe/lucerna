import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import editors  # noqa: E402
import env  # noqa: E402

PATHS = env.BenchPaths(Path("/bench"))


VSCODE = "/Applications/Visual Studio Code.app/Contents/MacOS/Code"


class LaunchTests(unittest.TestCase):
    def vscode_cmd(self, lsp):
        with tempfile.TemporaryDirectory() as d:
            paths = env.BenchPaths(Path(d))
            if lsp is not None:
                (paths.profiles).mkdir(parents=True)
                (paths.profiles / env.PROFILE_MARKER).write_text(json.dumps({"lsp": lsp}))
            with mock.patch.object(editors.subprocess, "Popen") as popen:
                launched = editors.launch("vscode", paths, Path("/bench/data/medium"))
            self.assertIsInstance(launched.t0_ns, int)
            return popen.call_args.args[0], str(paths.profiles)

    def test_vscode_main_binary_without_lsp(self):
        # Directly the Electron main binary: no bin/code node hop.
        cmd, prof = self.vscode_cmd(False)
        self.assertEqual(cmd, [VSCODE, "--user-data-dir", prof + "/vscode",
                               "--extensions-dir", prof + "/vscode-ext",
                               "--disable-extensions", "--new-window", "/bench/data/medium"])

    def test_vscode_lsp_uses_rust_analyzer_extensions(self):
        cmd, prof = self.vscode_cmd(True)
        self.assertEqual(cmd, [VSCODE, "--user-data-dir", prof + "/vscode",
                               "--extensions-dir", prof + "/vscode-ext-lsp",
                               "--new-window", "/bench/data/medium"])

    def test_vscode_env_drops_electron_node_mode(self):
        with mock.patch.dict(editors.os.environ, {"ELECTRON_RUN_AS_NODE": "1",
                                                   "ELECTRON_NO_ATTACH_CONSOLE": "1",
                                                   "KEEP_ME": "x"}), \
                mock.patch.object(editors.subprocess, "Popen") as popen:
            editors.launch("vscode", PATHS, Path("/t"))
        child = popen.call_args.kwargs["env"]
        self.assertNotIn("ELECTRON_RUN_AS_NODE", child)
        self.assertNotIn("ELECTRON_NO_ATTACH_CONSOLE", child)
        self.assertEqual(child["KEEP_ME"], "x")

    def test_vscode_unknown_profile_defaults_to_no_lsp(self):
        cmd, _ = self.vscode_cmd(None)
        self.assertIn("--disable-extensions", cmd)

    def test_lucerna_env_home(self):
        with tempfile.TemporaryDirectory() as d:
            paths = env.BenchPaths(Path(d))
            with mock.patch.object(editors.subprocess, "Popen") as popen, \
                    mock.patch.object(editors, "_lucerna_bin", return_value="/x/luc"), \
                    mock.patch.object(editors, "_check_executable"), \
                    mock.patch.object(editors, "_lstart", return_value="T1"):
                popen.return_value.pid = 123
                editors.launch("lucerna", paths, Path("/bench/data/medium"))
            self.assertEqual(popen.call_args.kwargs["env"]["HOME"], str(paths.lucerna_home))
            rec = json.loads((paths.lucerna_home / editors.PIDFILE_NAME).read_text())
            self.assertEqual(rec, {"pid": 123, "lstart": "T1", "exe": "/x/luc"})
        self.assertEqual(popen.call_args.args[0][0], "/x/luc")

    def test_lucerna_launch_refused_when_binary_vanished(self):
        with mock.patch.object(editors, "_lucerna_bin", return_value="/nonexistent/luc"), \
                mock.patch.object(editors.subprocess, "Popen") as popen:
            with self.assertRaises(editors.BinaryMissing):
                editors.launch("lucerna", PATHS, Path("/t"))
        popen.assert_not_called()


class ResolveLucernaTests(unittest.TestCase):
    def setUp(self):
        editors._LUCERNA.clear()
        self.addCleanup(editors._LUCERNA.clear)
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        self.bin = Path(d.name) / "luc"
        self.bin.write_text("#!/bin/sh\necho 'luc 9.9.9'\n")
        self.bin.chmod(0o755)

    def test_resolved_once_and_cached(self):
        with mock.patch.dict(editors.os.environ, {"LUCERNA_BIN": str(self.bin)}):
            rec = editors.resolve_lucerna()
            with mock.patch.object(editors, "_find_lucerna",
                                   side_effect=AssertionError("resolved twice")):
                self.assertEqual(editors.resolve_lucerna(), rec)
                self.assertEqual(editors._lucerna_bin(), rec["path"])
                self.assertIn("luc 9.9.9", editors.version("lucerna"))
        self.assertEqual(rec["path"], os.path.realpath(self.bin))
        self.assertEqual(rec["version"], "luc 9.9.9")
        self.assertRegex(rec["mtime"], r"^\d{4}-\d\d-\d\dT")

    def test_missing_or_not_executable_raises(self):
        self.bin.chmod(0o644)
        with mock.patch.dict(editors.os.environ, {"LUCERNA_BIN": str(self.bin)}):
            with self.assertRaises(editors.BinaryMissing):
                editors.resolve_lucerna()
        with mock.patch.dict(editors.os.environ, {"LUCERNA_BIN": "/nonexistent/luc"}):
            with self.assertRaises(editors.BinaryMissing):
                editors.resolve_lucerna()


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

    def _lucerna(self, started, table, write=True, exe="/x/Lucerna.app/Contents/MacOS/luc"):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        paths = env.BenchPaths(Path(d.name))
        if write:
            paths.lucerna_home.mkdir(parents=True)
            (paths.lucerna_home / editors.PIDFILE_NAME).write_text(
                json.dumps({"pid": 50, "lstart": started, "exe": exe}))
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

    def test_lucerna_exact_path_with_spaces(self):
        exe = "/Volumes/My Disk/Lucerna.app/Contents/MacOS/luc"
        table = [(50, 1, exe + " /w"), (51, 50, "helper")]
        self.assertEqual(self._lucerna("T1", table, exe=exe), [50, 51])

    def test_lucerna_other_binary_on_same_pid_excluded(self):
        table = [(50, 1, "/other/luc /w")]
        self.assertEqual(self._lucerna("T1", table), [])

    def test_foreign_instances_ignores_bench_and_crash_handler(self):
        zed = "/Applications/Zed.app/Contents/MacOS/zed"
        table = [(1000, 1, zed), (1001, 1000, zed + " --crash-handler x"),
                 (200, 1, zed + " --user-data-dir /bench/profiles/zed"),
                 (300, 1, "/Applications/Zed.app/Contents/MacOS/cli")]
        with mock.patch.object(editors, "_ps", return_value=table):
            self.assertEqual(editors.foreign_instances("zed", PATHS), [1000])

    def test_foreign_zed_anywhere(self):
        table = [(1, 0, "/Users/me/Apps/Zed.app/Contents/MacOS/zed"),
                 (2, 0, "/Volumes/My Disk/Zed.app/Contents/MacOS/zed --foreground"),
                 (3, 0, "Zed.app/Contents/MacOS/zed"),
                 (4, 0, "/usr/bin/vim /x/Zed.app/Contents/MacOS/zed"),     # an argument
                 (5, 0, "/x/Zed.app/Contents/MacOS/zed-helper"),
                 (6, 0, "/x/NotZed.app/Contents/MacOS/zed")]
        with mock.patch.object(editors, "_ps", return_value=table):
            self.assertEqual(editors.foreign_instances("zed", PATHS), [1, 2, 3])

    def test_running_pids_of_executable_and_bundle(self):
        table = [(1, 0, VSCODE), (2, 0, VSCODE + " --user-data-dir /u"),
                 (3, 1, "/Applications/Visual Studio Code.app/Contents/Frameworks/"
                        "Code Helper.app/Contents/MacOS/Code Helper --type=gpu"),
                 (4, 0, "/Applications/Other.app/Contents/MacOS/Other")]
        with mock.patch.object(editors, "_ps", return_value=table):
            self.assertEqual(editors.running_pids(VSCODE), [1, 2, 3])
            self.assertEqual(editors.running_pids("/x/luc"), [])

    def test_zed_launch_refused_while_users_zed_runs(self):
        table = [(1000, 1, "/Applications/Zed.app/Contents/MacOS/zed")]
        with mock.patch.object(editors, "_ps", return_value=table), \
                mock.patch.object(editors.subprocess, "Popen") as popen:
            with self.assertRaises(editors.ForeignInstance) as cm:
                editors.launch("zed", PATHS, Path("/t"))
        popen.assert_not_called()
        self.assertIn("1000", str(cm.exception))

    def test_zed_launches_main_binary_directly(self):
        with mock.patch.object(editors, "_ps", return_value=[]), \
                mock.patch.object(editors.subprocess, "Popen") as popen:
            editors.launch("zed", PATHS, Path("/t"))
        self.assertEqual(popen.call_args.args[0], [
            "/Applications/Zed.app/Contents/MacOS/zed", "--user-data-dir",
            "/bench/profiles/zed", "/t"])


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

    def test_quit_skips_pid_without_start_time(self):
        table = [(7, 1, "zed --user-data-dir /bench/profiles/zed")]
        kills = []
        with mock.patch.object(editors, "_ps", return_value=table), \
                mock.patch.object(editors, "_lstart", return_value=""), \
                mock.patch.object(editors.os, "kill", lambda p, s: kills.append((p, s))), \
                mock.patch.object(editors, "_reap"):
            editors.quit("zed", PATHS)
        self.assertEqual(kills, [])


if __name__ == "__main__":
    unittest.main()
