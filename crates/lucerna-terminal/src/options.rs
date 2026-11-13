//! How a terminal is started: shell, arguments and environment.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Everything `Terminal::spawn` needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalOptions {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Variables on top of Lucerna's own environment; `None` removes one.
    pub env: HashMap<String, Option<String>>,
    pub rows: u16,
    pub cols: u16,
    pub cell_width: u16,
    pub cell_height: u16,
    /// Scrollback lines, clamped to [`MAX_SCROLLBACK`].
    pub scrollback: usize,
}

/// Hard cap on scrollback lines, whatever the setting says.
pub const MAX_SCROLLBACK: usize = 100_000;

/// Shell used when neither a profile nor `$SHELL` names one (spec §6.2).
fn fallback_shell() -> &'static str {
    if cfg!(target_os = "macos") {
        "/bin/zsh"
    } else if cfg!(windows) {
        "powershell.exe"
    } else {
        "/bin/sh"
    }
}

/// Picks the shell program and its arguments.
///
/// A profile path wins, then `$SHELL`, then the platform fallback. Profile
/// arguments are used verbatim. Without them, the default shell on macOS runs
/// as a login shell (`-l`) so it loads the user's `PATH`, as VS Code does.
pub fn resolve_shell(
    profile_path: Option<&Path>,
    profile_args: Option<Vec<String>>,
    env_shell: Option<&OsStr>,
) -> (PathBuf, Vec<String>) {
    let program = match profile_path {
        Some(path) => path.to_path_buf(),
        None => env_shell
            .filter(|shell| !shell.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(fallback_shell())),
    };
    let args = profile_args.unwrap_or_else(|| {
        if profile_path.is_none() && cfg!(target_os = "macos") {
            vec!["-l".to_string()]
        } else {
            Vec::new()
        }
    });
    (program, args)
}

fn is_utf8_locale(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("utf-8") || value.contains("utf8")
}

fn apply(env: &mut HashMap<String, String>, layer: &HashMap<String, Option<String>>) {
    for (key, value) in layer {
        match value {
            Some(value) => env.insert(key.clone(), value.clone()),
            None => env.remove(key),
        };
    }
}

/// The child's environment: `base` (Lucerna's own), then the
/// `terminal.integrated.env.<os>` setting, then the profile's `env`, where
/// `None` removes a variable. The terminal variables are always set, and with
/// `detect_locale` a missing or non-UTF-8 `LANG` becomes `en_US.UTF-8`.
pub fn build_env(
    base: impl IntoIterator<Item = (String, String)>,
    setting_env: &HashMap<String, Option<String>>,
    profile_env: &HashMap<String, Option<String>>,
    version: &str,
    detect_locale: bool,
) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = base.into_iter().collect();
    apply(&mut env, setting_env);
    apply(&mut env, profile_env);
    for (key, value) in [
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
        ("TERM_PROGRAM", "lucerna"),
        ("TERM_PROGRAM_VERSION", version),
    ] {
        env.insert(key.to_string(), value.to_string());
    }
    if detect_locale && !env.get("LANG").is_some_and(|lang| is_utf8_locale(lang)) {
        env.insert("LANG".to_string(), "en_US.UTF-8".to_string());
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String {
        v.to_string()
    }

    #[test]
    fn resolve_shell_defaults_to_env_shell_with_login_flag_on_macos() {
        let (program, args) = resolve_shell(None, None, Some("/bin/bash".as_ref()));
        assert_eq!(program, PathBuf::from("/bin/bash"));
        if cfg!(target_os = "macos") {
            assert_eq!(args, vec![s("-l")]);
        } else {
            assert!(args.is_empty());
        }
    }

    #[test]
    fn resolve_shell_profile_args_are_used_verbatim() {
        let (program, args) = resolve_shell(
            Some(Path::new("/opt/fish")),
            Some(vec![s("-i")]),
            Some("/bin/bash".as_ref()),
        );
        assert_eq!(program, PathBuf::from("/opt/fish"));
        assert_eq!(args, vec![s("-i")]);
    }

    #[test]
    fn resolve_shell_profile_without_args_gets_none() {
        let (program, args) = resolve_shell(Some(Path::new("/opt/fish")), None, None);
        assert_eq!(program, PathBuf::from("/opt/fish"));
        assert!(args.is_empty());
    }

    #[test]
    fn resolve_shell_empty_env_shell_counts_as_unset() {
        let (program, _) = resolve_shell(None, None, Some("".as_ref()));
        assert_ne!(program, PathBuf::new());
    }

    #[test]
    fn resolve_shell_falls_back_when_no_shell() {
        let (program, _) = resolve_shell(None, None, None);
        let expected = if cfg!(target_os = "macos") {
            "/bin/zsh"
        } else if cfg!(windows) {
            "powershell.exe"
        } else {
            "/bin/sh"
        };
        assert_eq!(program, PathBuf::from(expected));
    }

    fn env(base: &[(&str, &str)], detect_locale: bool) -> HashMap<String, String> {
        build_env(
            base.iter().map(|(k, v)| (s(k), s(v))),
            &HashMap::new(),
            &HashMap::new(),
            "1.2.3",
            detect_locale,
        )
    }

    #[test]
    fn build_env_sets_term_vars_and_utf8_lang() {
        let e = env(&[("PATH", "/bin")], true);
        assert_eq!(e["TERM"], "xterm-256color");
        assert_eq!(e["COLORTERM"], "truecolor");
        assert_eq!(e["TERM_PROGRAM"], "lucerna");
        assert_eq!(e["TERM_PROGRAM_VERSION"], "1.2.3");
        assert_eq!(e["LANG"], "en_US.UTF-8");
        assert_eq!(e["PATH"], "/bin");
    }

    #[test]
    fn build_env_overrides_inherited_term() {
        let e = env(
            &[("TERM", "dumb"), ("TERM_PROGRAM", "Apple_Terminal")],
            true,
        );
        assert_eq!(e["TERM"], "xterm-256color");
        assert_eq!(e["TERM_PROGRAM"], "lucerna");
    }

    #[test]
    fn build_env_keeps_utf8_lang() {
        assert_eq!(env(&[("LANG", "vi_VN.UTF-8")], true)["LANG"], "vi_VN.UTF-8");
        assert_eq!(env(&[("LANG", "vi_VN.utf8")], true)["LANG"], "vi_VN.utf8");
    }

    #[test]
    fn build_env_replaces_non_utf8_lang() {
        assert_eq!(env(&[("LANG", "C")], true)["LANG"], "en_US.UTF-8");
    }

    #[test]
    fn build_env_detect_locale_off_leaves_lang() {
        assert!(!env(&[], false).contains_key("LANG"));
        assert_eq!(env(&[("LANG", "C")], false)["LANG"], "C");
    }

    #[test]
    fn build_env_null_removes_and_profile_wins() {
        let setting = HashMap::from([(s("A"), Some(s("1"))), (s("B"), None)]);
        let profile = HashMap::from([(s("A"), Some(s("2")))]);
        let e = build_env([(s("B"), s("base"))], &setting, &profile, "1.2.3", true);
        assert_eq!(e["A"], "2");
        assert!(!e.contains_key("B"));
    }

    #[test]
    fn build_env_profile_null_removes_setting_value() {
        let setting = HashMap::from([(s("A"), Some(s("1")))]);
        let profile = HashMap::from([(s("A"), None)]);
        let e = build_env([], &setting, &profile, "1.2.3", true);
        assert!(!e.contains_key("A"));
    }
}
