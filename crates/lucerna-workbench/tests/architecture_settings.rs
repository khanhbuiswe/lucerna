//! Architecture guards over `src/` (spec §8.3 of the multi-window design):
//! window-scope settings are read through the window's `WindowSettings`,
//! never through the app's global `AppState::settings`, outside the files
//! that own the app's (default + user) settings or have no window.

use std::path::{Path, PathBuf};

/// The files that may read `AppState::settings`: they load or write the
/// user files, read app-scope keys only, or run where no workbench window
/// exists (Welcome, Get Started, the app menu).
const ALLOWED: &[&str] = &[
    "config_files.rs",
    "app.rs",
    "router.rs",
    "startup.rs",
    "session.rs",
    "extensions/service.rs",
    "theme_picker.rs",
    "onboarding/mod.rs",
    "welcome.rs",
    "window_settings.rs",
    "app_menu.rs",
];

/// Every `.rs` file under `dir`.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `src`-relative paths with `/` separators, and each file's text.
fn sources() -> Vec<(String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let rel = path
                .strip_prefix(&src)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let text = std::fs::read_to_string(&path).unwrap();
            (rel, text)
        })
        .collect()
}

/// Whether `text` reads `AppState>().settings`, whitespace (a chained call
/// split over lines) ignored.
fn reads_app_settings(text: &str) -> bool {
    let squeezed: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let needle = "AppState>().settings";
    squeezed.match_indices(needle).any(|(at, _)| {
        // `.settings` itself, not a longer field name.
        squeezed[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
    })
}

#[test]
fn window_settings_are_not_read_through_app_state() {
    let offenders: Vec<String> = sources()
        .into_iter()
        .filter(|(rel, text)| !ALLOWED.contains(&rel.as_str()) && reads_app_settings(text))
        .map(|(rel, _)| rel)
        .collect();
    assert!(
        offenders.is_empty(),
        "read the window's settings (`WindowSettings`) instead of \
         `AppState::settings` in: {offenders:?}"
    );
}

#[test]
fn the_scan_finds_a_split_read() {
    assert!(reads_app_settings(
        "cx.global::<AppState>()\n            .settings\n            .get::<bool>(k, None)"
    ));
    assert!(reads_app_settings("&cx.global::<AppState>().settings;"));
    assert!(!reads_app_settings("cx.global::<AppState>().theme.ui"));
    assert!(!reads_app_settings("cx.global::<AppState>().settings_x"));
}

/// Processes start through the shared spawner, not `std::process` directly.
#[test]
#[ignore = "turned on by multi-window Task 7, which moves every spawn onto the shared spawner"]
fn no_direct_process_spawn() {
    let offenders: Vec<String> = sources()
        .into_iter()
        .filter(|(_, text)| text.contains("process::spawn") || text.contains("SpawnRequest"))
        .map(|(rel, _)| rel)
        .collect();
    assert!(
        offenders.is_empty(),
        "no `process::spawn` / `SpawnRequest` in: {offenders:?}"
    );
}
