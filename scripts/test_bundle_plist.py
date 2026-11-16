"""Tests for the app bundle's Info.plist channels.
Run: python3 -m unittest scripts/test_bundle_plist.py -v

Spec: docs/superpowers/specs/2026-11-16-dev-bundle-id-design.md
"""

import os
import pathlib
import plistlib
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
RENDER = ROOT / "scripts" / "render-plist.sh"
BUNDLE = ROOT / "scripts" / "bundle-app.sh"
CHANNELS = ("dev", "stable")


def run(channel, version="0.1.0", build="20261116.221211"):
    return subprocess.run(
        [str(RENDER), channel, version, build],
        capture_output=True,
        text=True,
        check=False,
    )


def raw_output(channel, **kwargs):
    result = run(channel, **kwargs)
    if result.returncode != 0:
        raise AssertionError(f"render-plist failed: {result.stderr}")
    return result.stdout


def render(channel, **kwargs):
    return plistlib.loads(raw_output(channel, **kwargs).encode())


def dry_run(*args):
    env = dict(os.environ, LUCERNA_BUNDLE_DRY_RUN="1")
    result = subprocess.run(
        [str(BUNDLE), *args],
        capture_output=True,
        text=True,
        check=False,
        env=env,
    )
    if result.returncode != 0:
        raise AssertionError(f"bundle-app dry run failed: {result.stderr}")
    return result.stdout


class RenderPlistTest(unittest.TestCase):
    def test_dev_channel(self):
        p = render("dev")
        self.assertEqual(p["CFBundleIdentifier"], "dev.lucerna.Lucerna-Dev")
        self.assertEqual(p["CFBundleName"], "Lucerna Dev")
        self.assertEqual(p["CFBundleDisplayName"], "Lucerna Dev")

    def test_stable_channel(self):
        p = render("stable")
        self.assertEqual(p["CFBundleIdentifier"], "dev.lucerna.Lucerna")
        self.assertEqual(p["CFBundleName"], "Lucerna")
        self.assertEqual(p["CFBundleDisplayName"], "Lucerna")

    def test_version_and_build(self):
        for channel in CHANNELS:
            with self.subTest(channel=channel):
                p = render(channel, version="1.2.3", build="20261116.221211")
                self.assertEqual(p["CFBundleShortVersionString"], "1.2.3")
                self.assertEqual(p["CFBundleVersion"], "20261116.221211")

    def test_executable_stays_luc(self):
        for channel in CHANNELS:
            with self.subTest(channel=channel):
                self.assertEqual(render(channel)["CFBundleExecutable"], "luc")

    def test_no_placeholder_left(self):
        for channel in CHANNELS:
            with self.subTest(channel=channel):
                self.assertNotIn("@", raw_output(channel))

    def test_unknown_channel_fails(self):
        result = run("beta")
        self.assertEqual(result.returncode, 2)
        self.assertIn("unknown channel: beta", result.stderr)


class BundleChannelTest(unittest.TestCase):
    def test_install_implies_stable(self):
        self.assertIn("channel=stable", dry_run("--install"))
        self.assertIn("channel=dev", dry_run())
        self.assertIn("channel=stable", dry_run("--stable"))


if __name__ == "__main__":
    unittest.main()
