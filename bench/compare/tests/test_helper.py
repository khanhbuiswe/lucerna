import json
import os
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

HELPER_DIR = Path(__file__).resolve().parent.parent / "helper"


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
        r = _run(self.binary, "watch", "--pid", str(os.getpid()), "--since", "0",
                 "--until", "change", "--timeout", "1")
        self.assertEqual(r.returncode, 3)
        self.assertEqual(r.stdout.strip(), "")

    def test_clock_matches_python(self):
        before = time.monotonic_ns()
        r = _run(self.binary, "now")
        after = time.monotonic_ns()
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
        ]
        for args in good:
            args = [a.encode().decode("unicode_escape") for a in args]
            with self.subTest(args=args):
                r = _run(self.binary, *args, env_extra=deny)
                self.assertEqual(r.returncode, 3, r.stderr)

    def test_bad_usage_exits_nonzero(self):
        r = _run(self.binary, "bogus")
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(r.stdout.strip(), "")


if __name__ == "__main__":
    unittest.main()
