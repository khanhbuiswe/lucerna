//! Foreground process of a terminal and the tab title derived from it (spec §6.6).

use std::path::PathBuf;

use crate::Terminal;

/// Longest tab title, in chars, counting the trailing `…`.
const MAX_TITLE_CHARS: usize = 24;

/// Interpreters whose script argument is a better name than the interpreter.
const GENERIC_ENTRY_NAMES: [&str; 3] = ["cli", "index", "main"];
/// Directories that are layout, not a package name, when walking up from an entry file.
const LAYOUT_DIRS: [&str; 5] = ["dist", "lib", "bin", "src", "build"];

/// The process in the foreground of a terminal's PTY.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
}

/// Foreground process of `t`'s PTY. Blocking (syscalls, `/proc`): call it from a
/// background thread only. `None` when there is no PTY, the process is gone or the
/// platform has no support.
pub fn foreground_process(t: &Terminal) -> Option<ProcessInfo> {
    #[cfg(unix)]
    {
        let fd = t.pty_fd()?;
        // SAFETY: `tcgetpgrp` only reads the descriptor; a closed or reused fd makes
        // it fail with an error code, which is handled below.
        let pgid = unsafe { libc::tcgetpgrp(fd) };
        if pgid <= 0 {
            return None;
        }
        platform::process_info(pgid as u32)
    }
    #[cfg(not(unix))]
    {
        let _ = t;
        None
    }
}

/// Last OSC 0/2 title and the foreground pid that set it.
#[derive(Debug, Clone, Default)]
pub struct TitleState {
    title: Option<String>,
    pid: Option<u32>,
}

impl TitleState {
    /// Record an OSC title set while `fg_pid` was in the foreground; `None` clears it.
    pub fn on_osc_title(&mut self, title: Option<String>, fg_pid: Option<u32>) {
        self.pid = fg_pid;
        self.title = title;
    }

    /// Tab name: the OSC title while the process that set it is still in the
    /// foreground, else the process name, else `Terminal`; at most 24 chars.
    pub fn tab_title(&self, fg: Option<&ProcessInfo>) -> String {
        let title = self
            .title
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .filter(|_| fg.map(|f| f.pid) == self.pid);
        let name = match (title, fg) {
            (Some(t), _) => t.to_string(),
            (None, Some(info)) => display_name(info),
            (None, None) => String::new(),
        };
        let name = if name.trim().is_empty() {
            "Terminal".to_string()
        } else {
            name
        };
        truncate(&name)
    }
}

fn truncate(s: &str) -> String {
    if s.chars().count() <= MAX_TITLE_CHARS {
        return s.to_string();
    }
    let mut out: String = s.chars().take(MAX_TITLE_CHARS - 1).collect();
    out.push('…');
    out
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn is_interpreter(name: &str) -> bool {
    if matches!(name, "node" | "bun" | "deno" | "python" | "ruby") {
        return true;
    }
    // `python3`, `python3.12`
    name.strip_prefix("python").is_some_and(|v| {
        !v.is_empty()
            && v.starts_with(|c: char| c.is_ascii_digit())
            && v.chars().all(|c| c.is_ascii_digit() || c == '.')
    })
}

fn stem(file: &str) -> &str {
    match file.rfind('.') {
        Some(i) if i > 0 => &file[..i],
        _ => file,
    }
}

/// Name of the thing an interpreter runs: a module, a `/bin/<name>` command or a script.
fn script_name(script: &str) -> String {
    if let Some((_, rest)) = script.rsplit_once("/bin/")
        && !rest.is_empty()
        && !rest.contains('/')
    {
        return stem(rest).to_string();
    }
    let file = stem(basename(script));
    if !GENERIC_ENTRY_NAMES.contains(&file) {
        return file.to_string();
    }
    let package = script
        .split('/')
        .rev()
        .skip(1)
        .find(|d| !d.is_empty() && !LAYOUT_DIRS.contains(d) && *d != "." && *d != "..");
    match package {
        Some(dir) => dir.trim_start_matches('@').to_string(),
        None => file.to_string(),
    }
}

/// Name shown for a process: the script for an interpreter, else the process name.
pub fn display_name(info: &ProcessInfo) -> String {
    let name = basename(&info.name);
    let from_argv0 = info.argv.first().map(|a| basename(a)).unwrap_or("");
    let program = if name.is_empty() { from_argv0 } else { name };
    if !is_interpreter(program) {
        return if info.name.is_empty() {
            program.to_string()
        } else {
            info.name.clone()
        };
    }
    let mut args = info.argv.iter().skip(1);
    while let Some(arg) = args.next() {
        if arg == "-m" {
            if let Some(module) = args.next() {
                return module.clone();
            }
            break;
        }
        if arg.starts_with('-') {
            continue;
        }
        return script_name(arg);
    }
    info.name.clone()
}

/// `/proc/<pid>/cmdline` and `KERN_PROCARGS2` parsing, pure so it is testable anywhere.
#[cfg(any(target_os = "linux", test))]
fn parse_cmdline(raw: &[u8]) -> Vec<String> {
    raw.split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect()
}

/// Layout: `argc: i32`, exec path, NUL padding, then `argc` NUL-terminated args.
#[cfg(any(target_os = "macos", test))]
fn parse_procargs2(raw: &[u8]) -> Vec<String> {
    let Some(head) = raw.get(..4) else {
        return Vec::new();
    };
    let argc = i32::from_ne_bytes([head[0], head[1], head[2], head[3]]).max(0) as usize;
    let rest = &raw[4..];
    let Some(end) = rest.iter().position(|b| *b == 0) else {
        return Vec::new();
    };
    let rest = &rest[end..];
    let start = rest.iter().position(|b| *b != 0).unwrap_or(rest.len());
    rest[start..]
        .split(|b| *b == 0)
        .take(argc)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect()
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    pub fn process_info(pid: u32) -> Option<ProcessInfo> {
        let dir = PathBuf::from(format!("/proc/{pid}"));
        let name = std::fs::read_to_string(dir.join("comm"))
            .map(|s| s.trim_end().to_string())
            .unwrap_or_default();
        let argv = std::fs::read(dir.join("cmdline"))
            .map(|raw| parse_cmdline(&raw))
            .unwrap_or_default();
        let cwd = std::fs::read_link(dir.join("cwd")).ok();
        if name.is_empty() && argv.is_empty() && cwd.is_none() {
            return None;
        }
        Some(ProcessInfo {
            pid,
            name,
            argv,
            cwd,
        })
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::CStr;

    pub fn process_info(pid: u32) -> Option<ProcessInfo> {
        let argv = argv(pid);
        let name = name(pid)
            .or_else(|| argv.first().map(|a| basename(a).to_string()))
            .unwrap_or_default();
        let cwd = cwd(pid);
        if name.is_empty() && argv.is_empty() && cwd.is_none() {
            return None;
        }
        Some(ProcessInfo {
            pid,
            name,
            argv,
            cwd,
        })
    }

    fn name(pid: u32) -> Option<String> {
        let mut buf = [0u8; 256];
        // SAFETY: `buf` is valid for `buf.len()` bytes; `proc_name` writes at most that
        // many, NUL-terminated, and returns the length or <= 0 on failure.
        let n = unsafe { libc::proc_name(pid as i32, buf.as_mut_ptr().cast(), buf.len() as u32) };
        if n <= 0 {
            return None;
        }
        let name = String::from_utf8_lossy(&buf[..(n as usize).min(buf.len())]).into_owned();
        let name = name.trim_end_matches('\0').to_string();
        (!name.is_empty()).then_some(name)
    }

    fn cwd(pid: u32) -> Option<PathBuf> {
        // SAFETY: `proc_vnodepathinfo` is plain old data, so all-zero is a valid value.
        let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
        // SAFETY: `info` is valid for `size` bytes, which is what the call may write.
        let n = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                (&mut info as *mut libc::proc_vnodepathinfo).cast(),
                size,
            )
        };
        if n < size {
            return None;
        }
        // SAFETY: the kernel NUL-terminates `vip_path` within its fixed-size array.
        let path = unsafe { CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast()) };
        let path = path.to_str().ok()?;
        (!path.is_empty()).then(|| PathBuf::from(path))
    }

    fn argv(pid: u32) -> Vec<String> {
        // `kern.argmax` bounds the buffer; 1 MiB is plenty for the argv we read.
        let mut buf = vec![0u8; 1 << 20];
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as i32];
        let mut len = buf.len();
        // SAFETY: `mib` has 3 entries as declared, `buf` is valid for `len` bytes and
        // `len` is updated by the kernel to the number of bytes written.
        let rc = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as u32,
                buf.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 {
            return Vec::new();
        }
        parse_procargs2(&buf[..len.min(buf.len())])
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
mod platform {
    use super::*;

    pub fn process_info(_pid: u32) -> Option<ProcessInfo> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(pid: u32, name: &str, argv: &[&str]) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.into(),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            cwd: None,
        }
    }

    #[test]
    fn osc_title_wins_while_same_process() {
        let mut t = TitleState::default();
        t.on_osc_title(Some("✳ working".into()), Some(10));
        assert_eq!(t.tab_title(Some(&info(10, "node", &[]))), "✳ working");
    }

    #[test]
    fn osc_title_dropped_when_foreground_changes() {
        let mut t = TitleState::default();
        t.on_osc_title(Some("✳ working".into()), Some(10));
        assert_eq!(t.tab_title(Some(&info(11, "vim", &["vim"]))), "vim");
    }

    #[test]
    fn osc_title_none_clears() {
        let mut t = TitleState::default();
        t.on_osc_title(Some("x".into()), Some(10));
        t.on_osc_title(None, Some(10));
        assert_eq!(t.tab_title(Some(&info(10, "zsh", &[]))), "zsh");
    }

    #[test]
    fn node_shows_script_name() {
        let argv = [
            "node",
            "/usr/local/lib/node_modules/@anthropic-ai/claude-code/cli.js",
        ];
        assert_eq!(display_name(&info(1, "node", &argv)), "claude-code");
        let bin = ["node", "/usr/local/bin/claude"];
        assert_eq!(display_name(&info(1, "node", &bin)), "claude");
        let flags = ["node", "--inspect", "/srv/app/server.js"];
        assert_eq!(display_name(&info(1, "node", &flags)), "server");
        let dist = ["node", "/p/node_modules/codex/dist/index.js"];
        assert_eq!(display_name(&info(1, "node", &dist)), "codex");
    }

    #[test]
    fn python_module() {
        let argv = ["python3", "-m", "http.server"];
        assert_eq!(display_name(&info(1, "python3", &argv)), "http.server");
        let versioned = ["python3.12", "/x/run.py"];
        assert_eq!(display_name(&info(1, "python3.12", &versioned)), "run");
    }

    #[test]
    fn plain_binary_name() {
        assert_eq!(display_name(&info(1, "vim", &["vim", "a.txt"])), "vim");
        assert_eq!(display_name(&info(1, "env", &["env", "x.js"])), "env");
    }

    #[test]
    fn interpreter_without_script_keeps_name() {
        assert_eq!(display_name(&info(1, "node", &["node"])), "node");
    }

    #[test]
    fn fallback_terminal() {
        assert_eq!(TitleState::default().tab_title(None), "Terminal");
    }

    #[test]
    fn truncates_to_24_with_ellipsis() {
        let mut t = TitleState::default();
        t.on_osc_title(Some("a".repeat(30)), None);
        let s = t.tab_title(None);
        assert_eq!(s.chars().count(), 24);
        assert!(s.ends_with('…'));
        t.on_osc_title(Some("ế".repeat(24)), None);
        assert_eq!(t.tab_title(None), "ế".repeat(24));
    }

    #[test]
    fn parses_cmdline_and_procargs2() {
        assert_eq!(parse_cmdline(b"node\0a.js\0"), ["node", "a.js"]);
        let mut raw = 2i32.to_ne_bytes().to_vec();
        raw.extend_from_slice(b"/usr/bin/node\0\0\0node\0a.js\0PATH=/bin\0");
        assert_eq!(parse_procargs2(&raw), ["node", "a.js"]);
        assert!(parse_procargs2(&[1]).is_empty());
    }
}
