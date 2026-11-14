import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import env  # noqa: E402

THERM_QUIET = """Note: No thermal warning level has been recorded
Note: No performance warning level has been recorded
Note: No CPU power status has been recorded
"""
THERM_LIMITED = """Note: No thermal warning level has been recorded
Note: No performance warning level has been recorded
CPU_Scheduler_Limit\t= 100
CPU_Available_CPUs\t= 8
CPU_Speed_Limit\t\t= 80
"""
BATT_AC = """Now drawing from 'AC Power'
 -InternalBattery-0 (id=7209059)\t66%; charging; 2:12 remaining present: true
"""
BATT_BATTERY = """Now drawing from 'Battery Power'
 -InternalBattery-0 (id=7209059)\t66%; discharging; 3:00 remaining present: true
"""
SETTINGS_OFF = """System-wide power settings:
Currently in use:
 standby              1
 disksleep            10
 lowpowermode         0
 womp                 1
"""
SETTINGS_LPM = SETTINGS_OFF.replace("lowpowermode         0", "lowpowermode         1")


def fake_pmset(therm=THERM_QUIET, batt=BATT_AC, settings=SETTINGS_OFF):
    table = {("-g", "therm"): therm, ("-g", "batt"): batt, ("-g",): settings}
    return lambda args: table[tuple(args)]


class ProfilesTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.paths = env.BenchPaths(Path(self.tmp.name) / "bench")

    def test_write_profiles_vscode_disables_trust_and_telemetry(self):
        env.write_profiles(self.paths, lsp=False)
        s = json.loads((self.paths.profiles / "vscode/User/settings.json").read_text())
        self.assertEqual(s["telemetry.telemetryLevel"], "off")
        self.assertEqual(s["update.mode"], "none")
        self.assertIs(s["security.workspace.trust.enabled"], False)
        self.assertEqual(s["workbench.startupEditor"], "none")
        self.assertEqual(s["window.restoreWindows"], "none")
        self.assertEqual(s["editor.fontSize"], 14)
        self.assertIs(s["editor.minimap.enabled"], False)
        self.assertIs(s["extensions.autoCheckUpdates"], False)
        self.assertIs(s["workbench.editor.enablePreviewFromQuickOpen"], False)

    def test_write_profiles_zed_no_lsp(self):
        env.write_profiles(self.paths, lsp=False)
        s = json.loads((self.paths.profiles / "zed/config/settings.json").read_text())
        self.assertEqual(s["languages"]["Rust"]["language_servers"], [])
        self.assertEqual(s["telemetry"], {"diagnostics": False, "metrics": False})
        self.assertIs(s["disable_ai"], True)
        self.assertEqual(s["buffer_font_size"], 14)
        self.assertIs(s["preview_tabs"]["enable_preview_from_file_finder"], False)
        self.assertEqual(s["auto_install_extensions"], {"html": False})
        self.assertIs(s["suggest_extensions"], False)

    def test_profile_marker_records_lsp(self):
        self.assertFalse(env.profile_lsp(self.paths))
        env.write_profiles(self.paths, lsp=False)
        self.assertFalse(env.profile_lsp(self.paths))
        self.assertTrue((self.paths.profiles / "vscode-ext").is_dir())
        with mock.patch.object(env.shutil, "which", return_value="/x/rust-analyzer"):
            env.write_profiles(self.paths, lsp=True)
        self.assertTrue(env.profile_lsp(self.paths))

    def test_vscode_extension_dirs(self):
        self.assertEqual(env.vscode_ext_dir(self.paths, False), self.paths.profiles / "vscode-ext")
        self.assertEqual(env.vscode_ext_dir(self.paths, True),
                         self.paths.profiles / "vscode-ext-lsp")
        self.assertFalse(env.vscode_rust_analyzer_installed(self.paths))
        (self.paths.profiles / "vscode-ext-lsp" / "rust-lang.rust-analyzer-0.3.2-darwin-arm64"
         ).mkdir(parents=True)
        self.assertTrue(env.vscode_rust_analyzer_installed(self.paths))


class ResetProfilesTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.paths = env.BenchPaths(Path(self.tmp.name) / "bench")

    def test_wipes_all_but_lsp_extensions_and_pidfile(self):
        env.write_profiles(self.paths, lsp=False)
        prof = self.paths.profiles
        (prof / "zed" / "data" / "db").mkdir(parents=True)
        (prof / "vscode-ext" / "stray").mkdir(parents=True)
        (prof / "vscode-ext-lsp" / "rust-lang.rust-analyzer-1").mkdir(parents=True)
        pidfile = self.paths.lucerna_home / env.PIDFILE_NAME
        pidfile.write_text("{}")
        outside = Path(self.tmp.name) / "outside"
        outside.mkdir()
        (outside / "keep").write_text("x")
        (prof / "link").symlink_to(outside)
        (prof / "notes.txt").write_text("mine")
        with mock.patch.object(env.sys, "stderr") as err:
            env.reset_profiles(self.paths)
        # unknown entries survive, with a warning
        self.assertEqual(sorted(p.name for p in prof.iterdir()),
                         ["link", "lucerna-home", "notes.txt", "vscode-ext-lsp"])
        self.assertEqual((prof / "notes.txt").read_text(), "mine")
        warned = "".join(c.args[0] for c in err.write.call_args_list)
        self.assertIn("notes.txt", warned)
        self.assertEqual([p.name for p in self.paths.lucerna_home.iterdir()],
                         [env.PIDFILE_NAME])
        self.assertTrue((prof / "vscode-ext-lsp" / "rust-lang.rust-analyzer-1").is_dir())
        self.assertTrue((outside / "keep").exists())   # unknown symlink never followed
        self.assertTrue(self.paths.data.parent.exists())

    def test_refuses_symlinked_profiles_dir(self):
        real = Path(self.tmp.name) / "real-profiles"
        (real / "zed").mkdir(parents=True)
        self.paths.root.mkdir(parents=True)
        self.paths.profiles.symlink_to(real)
        with self.assertRaises(RuntimeError) as cm:
            env.reset_profiles(self.paths)
        self.assertIn("symlink", str(cm.exception))
        self.assertTrue((real / "zed").is_dir())

    def test_known_entries_cover_what_write_profiles_creates(self):
        env.write_profiles(self.paths, lsp=False)
        for entry in self.paths.profiles.iterdir():
            self.assertIn(entry.name, env.PROFILE_ENTRIES)

    def test_no_profiles_dir_is_noop(self):
        self.assertEqual(env.reset_profiles(self.paths), [])

    def test_refuses_outside_profiles(self):
        prof = self.paths.profiles
        prof.mkdir(parents=True)
        (prof / "zed").mkdir()
        real_iterdir = Path.iterdir

        def fake_iterdir(p):
            if p == prof:
                return iter([Path(self.tmp.name) / "zed"])   # known name, wrong place
            return real_iterdir(p)
        with mock.patch.object(Path, "iterdir", fake_iterdir):
            with self.assertRaises(RuntimeError):
                env.reset_profiles(self.paths)
        self.assertTrue((prof / "zed").exists())

    def test_write_profiles_lucerna_real_keys_and_trust(self):
        env.write_profiles(self.paths, lsp=False)
        home = self.paths.profiles / "lucerna-home"
        s = json.loads((home / ".config/lucerna/settings.json").read_text())
        self.assertEqual(s["editor.fontSize"], 14)
        self.assertEqual(s["workbench.startupEditor"], "none")
        self.assertIs(s["lucerna.lsp.enabled"], False)
        self.assertIs(s["workbench.editor.enablePreview"], False)
        trust = json.loads(
            (home / "Library/Application Support/Lucerna/trusted.json").read_text()
        )
        self.assertIn(str(self.paths.data), trust["parents"])

    def test_lsp_true_points_all_at_rust_analyzer(self):
        with mock.patch.object(env.shutil, "which", return_value="/x/rust-analyzer"):
            env.write_profiles(self.paths, lsp=True)
        v = json.loads((self.paths.profiles / "vscode/User/settings.json").read_text())
        self.assertEqual(v["rust-analyzer.server.path"], "/x/rust-analyzer")
        z = json.loads((self.paths.profiles / "zed/config/settings.json").read_text())
        self.assertEqual(z["lsp"]["rust-analyzer"]["binary"]["path"], "/x/rust-analyzer")
        l = json.loads(
            (self.paths.profiles / "lucerna-home/.config/lucerna/settings.json").read_text()
        )
        self.assertEqual(l["lsp"]["rust-analyzer"]["binary"]["path"], "/x/rust-analyzer")
        self.assertIs(l["lucerna.lsp.enabled"], True)


class PrepareDataTest(unittest.TestCase):
    def test_marker_and_calls(self):
        with tempfile.TemporaryDirectory() as tmp:
            paths = env.BenchPaths(Path(tmp) / "bench")
            with mock.patch.object(env.subprocess, "run") as run:
                env.prepare_data(paths)
            self.assertEqual(run.call_count, 2)
            lines = paths.marker_file.read_text().split("\n")
            self.assertEqual(lines[:-1], ["█" * 80] * 200)
            self.assertEqual(paths.big_file, paths.large_repo / "big.rs")


class PreflightTest(unittest.TestCase):
    def run_preflight(self, procs=("launchd", "zsh"), load=0.5, **pm):
        with mock.patch.object(env, "_processes", return_value=list(procs)), \
             mock.patch.object(env, "_pmset", fake_pmset(**pm)), \
             mock.patch.object(env.os, "getloadavg", return_value=(load, 0, 0)):
            return env.preflight()

    def test_preflight_flags_cargo_and_thermal(self):
        reasons = self.run_preflight(procs=("zsh", "cargo"), therm=THERM_LIMITED)
        self.assertEqual(len(reasons), 2, reasons)

    def test_preflight_ok_when_quiet(self):
        self.assertEqual(self.run_preflight(), [])

    def test_preflight_flags_load_battery_low_power(self):
        reasons = self.run_preflight(load=3.0, batt=BATT_BATTERY, settings=SETTINGS_LPM)
        self.assertEqual(len(reasons), 3, reasons)


if __name__ == "__main__":
    unittest.main()
