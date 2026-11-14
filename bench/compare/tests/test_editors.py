import sys
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
        with mock.patch.object(editors.subprocess, "Popen") as popen, \
                mock.patch.object(editors, "_lucerna_bin", return_value="/x/luc"):
            editors.launch("lucerna", PATHS, Path("/bench/data/medium"))
        self.assertEqual(popen.call_args.kwargs["env"]["HOME"], "/bench/profiles/lucerna-home")
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


if __name__ == "__main__":
    unittest.main()
