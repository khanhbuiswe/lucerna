import json
import os
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

HELPER_DIR = Path(__file__).resolve().parent.parent / "helper"
DISPLAY_WATCH = ["watch", "--display", "--exe", "/Applications/X.app/Contents/MacOS/x",
                 "--exclude-pids", "11,12", "--since", "0", "--until", "coverage:0.3",
                 "--background", "1E1E1E", "--timeout", "1"]


def _run(binary, *args, env_extra=None):
    env = dict(os.environ)
    if env_extra:
        env.update(env_extra)
    return subprocess.run([str(binary), *args], capture_output=True, text=True,
                          env=env, timeout=60)


@unittest.skipUnless(shutil.which("swiftc"), "swiftc not installed")
class HelperTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory()
        cls.binary = Path(cls._tmp.name) / "bench-helper"
        build = subprocess.run(["sh", str(HELPER_DIR / "build.sh"), str(cls.binary)],
                               capture_output=True, text=True, timeout=600)
        if build.returncode != 0:
            raise AssertionError("build failed:\n" + build.stdout + build.stderr)
        cls.build_output = build.stdout + build.stderr
        check = _run(cls.binary, "check")
        try:
            perms = json.loads(check.stdout)
            cls.has_all_permissions = perms["screen_recording"] and perms["accessibility"]
        except (ValueError, KeyError):
            cls.has_all_permissions = False

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def test_build_has_no_warnings(self):
        self.assertNotIn("warning:", self.build_output)

    def test_check_reports_permissions(self):
        r = _run(self.binary, "check")
        data = json.loads(r.stdout)
        self.assertIsInstance(data["screen_recording"], bool)
        self.assertIsInstance(data["accessibility"], bool)
        expected = 0 if (data["screen_recording"] and data["accessibility"]) else 3
        self.assertEqual(r.returncode, expected)

    def test_missing_permission_exits_nonzero(self):
        deny = {"BENCH_HELPER_FAKE_DENY": "1"}
        r = _run(self.binary, "check", env_extra=deny)
        self.assertEqual(r.returncode, 3)
        self.assertEqual(json.loads(r.stdout),
                         {"screen_recording": False, "accessibility": False})
        measuring = [
            ["watch", "--pid", str(os.getpid()), "--since", "0",
             "--until", "change", "--timeout", "1"],
            ["scroll", "--pid", str(os.getpid()), "--lines", "10", "--rate", "60"],
            ["type", "--pid", str(os.getpid()), "--text", "ab"],
            ["key", "--pid", str(os.getpid()), "--combo", "cmd+p"],
            ["sample", "--pid", str(os.getpid()), "--timeout", "1"],
            ["record", "--pid", str(os.getpid()), "--seconds", "1"],
            DISPLAY_WATCH,
            ["frame", "--pid", str(os.getpid()), "--x", "0", "--y", "25", "--w", "1280",
             "--h", "800", "--timeout", "1"],
        ]
        for args in measuring:
            with self.subTest(cmd=args[0]):
                r = _run(self.binary, *args, env_extra=deny)
                self.assertEqual(r.returncode, 3)
                self.assertEqual(r.stdout.strip(), "")
                self.assertIn("permission", r.stderr.lower())

    def test_missing_permission_real(self):
        # Without the fake flag: only meaningful when the machine lacks a permission.
        if self.has_all_permissions:
            self.skipTest("machine has both permissions")
        r = _run(self.binary, "check")
        self.assertEqual(r.returncode, 3)
        perms = json.loads(r.stdout)
        if not perms["screen_recording"]:
            r = _run(self.binary, "watch", "--pid", str(os.getpid()), "--since", "0",
                     "--until", "change", "--timeout", "1")
            self.assertEqual(r.returncode, 3)
            self.assertEqual(r.stdout.strip(), "")
        if not perms["accessibility"]:
            r = _run(self.binary, "key", "--pid", str(os.getpid()), "--combo", "cmd+p",
                     "--timeout", "1")
            self.assertEqual(r.returncode, 3)
            self.assertEqual(r.stdout.strip(), "")

    def _expect_denied(self, deny, args):
        r = _run(self.binary, *args, env_extra={"BENCH_HELPER_FAKE_DENY": deny})
        self.assertEqual(r.returncode, 3, r.stderr)
        self.assertEqual(r.stdout.strip(), "")
        self.assertIn("permission", r.stderr.lower())
        return r

    def test_capture_commands_need_screen_recording(self):
        me = str(os.getpid())
        for args in (["watch", "--pid", me, "--since", "0", "--until", "change",
                      "--timeout", "1"],
                     ["sample", "--pid", me, "--timeout", "1"],
                     ["record", "--pid", me, "--seconds", "1"],
                     DISPLAY_WATCH,
                     ["scroll", "--pid", me, "--lines", "5"]):
            with self.subTest(cmd=args[0]):
                r = self._expect_denied("screen", args)
                self.assertIn("screen recording", r.stderr.lower())
                if args[0] != "scroll":  # scroll also needs Accessibility
                    self.assertNotIn("accessibility", r.stderr.lower())

    def test_input_commands_need_accessibility(self):
        me = str(os.getpid())
        for args in (["key", "--pid", me, "--combo", "cmd+p", "--timeout", "1"],
                     ["type", "--pid", me, "--text", "ab", "--timeout", "1"],
                     ["frame", "--pid", me, "--x", "0", "--y", "25", "--w", "1280", "--h", "800"],
                     ["activate", "--pid", me, "--timeout", "1"],
                     ["scroll", "--pid", me, "--lines", "5"]):
            with self.subTest(cmd=args[0]):
                r = self._expect_denied("accessibility", args)
                self.assertIn("accessibility", r.stderr.lower())
                if args[0] != "scroll":  # scroll also needs Screen Recording
                    self.assertNotIn("screen recording", r.stderr.lower())

    def test_key_and_type_do_not_need_screen_recording(self):
        # With Screen Recording denied, key/type get past the permission check
        # and look for a window via CGWindowList; this process has none, so
        # they time out (exit 2) instead of exiting 3.
        check = _run(self.binary, "check")
        if not json.loads(check.stdout)["accessibility"]:
            self.skipTest("Accessibility not granted on this machine")
        me = str(os.getpid())
        for args in (["key", "--pid", me, "--combo", "cmd+p", "--timeout", "0.3"],
                     ["type", "--pid", me, "--text", "ab", "--timeout", "0.3"],
                     ["frame", "--pid", me, "--x", "0", "--y", "25", "--w", "1280", "--h", "800",
                      "--timeout", "0.3"],
                     ["activate", "--pid", me, "--timeout", "0.3"]):
            with self.subTest(cmd=args[0]):
                r = _run(self.binary, *args, env_extra={"BENCH_HELPER_FAKE_DENY": "screen"})
                self.assertEqual(r.returncode, 2, r.stderr)
                self.assertEqual(r.stdout.strip(), "")
                self.assertIn("no on-screen window", r.stderr)

    def test_clock_matches_python(self):
        before = time.clock_gettime_ns(time.CLOCK_UPTIME_RAW)
        r = _run(self.binary, "now")
        after = time.clock_gettime_ns(time.CLOCK_UPTIME_RAW)
        self.assertEqual(r.returncode, 0, r.stderr)
        t = json.loads(r.stdout)["t_ns"]
        # Same clock: the helper's reading lies between two Python readings,
        # and only the process exit separates it from the second one.
        self.assertGreaterEqual(t, before)
        self.assertLessEqual(t, after)
        self.assertLess(after - t, 5_000_000)

    def test_usage_errors_exit_1_before_permission_check(self):
        # Arguments are validated first, so these fail with 1 even when denied.
        deny = {"BENCH_HELPER_FAKE_DENY": "1"}
        bad = [
            ["watch", "--pid", "1", "--since", "0", "--until", "change", "--timeout", "-1"],
            ["watch", "--pid", "1", "--since", "0", "--until", "change", "--timeout", "nan"],
            ["watch", "--pid", "1,x", "--since", "0", "--until", "change"],
            ["watch", "--pid", "1", "--since", "0", "--until", "coverage:2"],
            ["sample", "--pid", "1", "--timeout", "-5"],
            ["scroll", "--pid", "1", "--lines", "0"],
            ["scroll", "--pid", "1", "--lines", "5", "--rate", "-1"],
            ["type", "--pid", "1", "--text", "h\u00e9"],
            ["key", "--pid", "1", "--combo", "hyper+p"],
            # watch --display: absolute --exe, coverage only, --background
            # required, no --pid / --baseline, valid --exclude-pids
            ["watch", "--display", "--exe", "rel/luc", "--since", "0",
             "--until", "coverage:0.3", "--background", "000000"],
            ["watch", "--display", "--since", "0", "--until", "coverage:0.3",
             "--background", "000000"],
            ["watch", "--display", "--exe", "/x/luc", "--since", "0", "--until", "change",
             "--background", "000000"],
            ["watch", "--display", "--exe", "/x/luc", "--since", "0", "--until", "coverage:0.3"],
            ["watch", "--display", "--pid", "1", "--exe", "/x/luc", "--since", "0",
             "--until", "coverage:0.3", "--background", "000000"],
            ["watch", "--display", "--exe", "/x/luc", "--exclude-pids", "1,x", "--since", "0",
             "--until", "coverage:0.3", "--background", "000000"],
            ["watch", "--display", "--exe", "/x/luc", "--since", "0",
             "--until", "coverage:0.3", "--background", "000000", "--baseline", "/x.png"],
            # frame: all four numbers, size >= 100
            ["frame", "--pid", "1", "--x", "0", "--y", "25", "--w", "1280"],
            ["frame", "--pid", "1", "--x", "0", "--y", "25", "--w", "50", "--h", "800"],
            ["frame", "--pid", "1", "--x", "a", "--y", "25", "--w", "1280", "--h", "800"],
            ["activate"],
            ["activate", "--pid", "0"],
            ["activate", "--pid", "1", "--timeout", "-1"],
            # watch --until change diagnostics
            ["watch", "--pid", "1", "--since", "0", "--until", "change", "--min-change", "0"],
            ["watch", "--pid", "1", "--since", "0", "--until", "change", "--min-change", "1.5"],
            ["watch", "--pid", "1", "--since", "0", "--until", "change", "--min-change", "x"],
            ["watch", "--pid", "1", "--since", "0", "--until", "change", "--dump-dir"],
            ["watch", "--pid", "1", "--since", "0", "--until", "coverage:0.3",
             "--min-change", "0.01"],
            ["watch", "--pid", "1", "--since", "0", "--until", "coverage:0.3",
             "--dump-dir", "/tmp/d"],
            ["watch", "--display", "--exe", "/x/luc", "--since", "0",
             "--until", "coverage:0.3", "--background", "000000", "--dump-dir", "/tmp/d"],
            # scroll --unit / --pixels-per-event / --dump-dir
            ["scroll", "--pid", "1", "--lines", "5", "--unit", "page"],
            ["scroll", "--pid", "1", "--lines", "5", "--unit", "pixel",
             "--pixels-per-event", "0"],
            ["scroll", "--pid", "1", "--lines", "5", "--unit", "pixel",
             "--pixels-per-event", "20000"],
            ["scroll", "--pid", "1", "--lines", "5", "--pixels-per-event", "40"],
            ["scroll", "--pid", "1", "--lines", "5", "--dump-dir"],
            # record: --pid and a positive --seconds
            ["record", "--seconds", "5"],
            ["record", "--pid", "1"],
            ["record", "--pid", "1", "--seconds", "0"],
            ["record", "--pid", "1", "--seconds", "-2"],
            ["record", "--pid", "1,x", "--seconds", "5"],
            ["record", "--pid", "1", "--seconds", "5", "--dump-dir"],
        ]
        for args in bad:
            args = [a.encode().decode("unicode_escape") for a in args]
            with self.subTest(args=args):
                r = _run(self.binary, *args, env_extra=deny)
                self.assertEqual(r.returncode, 1, r.stderr)
                self.assertEqual(r.stdout.strip(), "")

    def test_valid_arguments_reach_permission_check(self):
        # Comma pid lists, "\n" in text, shifted punctuation and a default
        # scroll rate all parse; the fake denial then gives exit 3.
        deny = {"BENCH_HELPER_FAKE_DENY": "1"}
        good = [
            ["watch", "--pid", "11,12, 13", "--since", "0", "--until", "coverage:0.4",
             "--background", "1E1E1E"],
            ["sample", "--pid", "11,12"],
            ["scroll", "--pid", "11,12", "--lines", "5"],
            ["type", "--pid", "11,12", "--text", "Hello, World!\n"],
            ["key", "--pid", "11,12", "--combo", "cmd+shift+p"],
            DISPLAY_WATCH,
            ["watch", "--display", "--exe", "/A b/X.app/Contents/MacOS/x", "--since", "0",
             "--until", "coverage:0.3", "--background", "1E1E1E"],
            ["frame", "--pid", "11,12", "--x", "-10", "--y", "25", "--w", "1280", "--h", "800"],
            ["activate", "--pid", "11,12"],
            ["watch", "--pid", "11", "--since", "0", "--until", "change",
             "--min-change", "0.001", "--dump-dir", "/tmp/never-created"],
            ["scroll", "--pid", "11", "--lines", "5", "--unit", "line"],
            ["scroll", "--pid", "11", "--lines", "5", "--unit", "pixel"],
            ["scroll", "--pid", "11", "--lines", "5", "--unit", "pixel",
             "--pixels-per-event", "12", "--dump-dir", "/tmp/never-created"],
            ["record", "--pid", "11,12", "--seconds", "2.5"],
            ["record", "--pid", "11", "--seconds", "5", "--dump-dir", "/tmp/never-created"],
        ]
        for args in good:
            args = [a.encode().decode("unicode_escape") for a in args]
            with self.subTest(args=args):
                r = _run(self.binary, *args, env_extra=deny)
                self.assertEqual(r.returncode, 3, r.stderr)

    def test_display_needs_no_permission(self):
        r = _run(self.binary, "display", env_extra={"BENCH_HELPER_FAKE_DENY": "1"})
        self.assertEqual(r.returncode, 0, r.stderr)
        data = json.loads(r.stdout)
        for key in ("width", "height", "pixel_width", "pixel_height", "scale", "refresh_hz"):
            self.assertIn(key, data)
        self.assertGreater(data["width"], 0)
        self.assertGreater(data["refresh_hz"], 0)

    def test_usage_lists_new_commands(self):
        r = _run(self.binary)
        self.assertEqual(r.returncode, 1)
        for text in ("watch  --display --exe", "frame  --pid", "activate --pid", "display",
                     "--min-change", "--unit line|pixel", "--pixels-per-event",
                     "record --pid"):
            self.assertIn(text, r.stderr)

    def test_bad_usage_exits_nonzero(self):
        r = _run(self.binary, "bogus")
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(r.stdout.strip(), "")


if __name__ == "__main__":
    unittest.main()
