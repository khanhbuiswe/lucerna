//! The pinned wasi-sdk used to compile grammars to WASM.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::builder::{BuildError, unique_suffix};
use crate::disk::{MIN_FREE_BYTES, free_bytes};
use crate::process::{CancelToken, run};

pub const WASI_SDK_VERSION: &str = "34.0";

const SHA256_ARM64: &str = "9c59398106b417f8f14913380fdf0097a8cc0ff4af9eb3ce0065a859e88d49e9";
const SHA256_X86_64: &str = "87d27fa8adc68dee59bfbf2e22a6d34ef717c34d6bf1d8af2a56fc929d9ce0eb";

/// `curl --max-time` in seconds; the process timeout leaves curl room to
/// report it.
const CURL_MAX_TIME: &str = "120";
const CURL_TIMEOUT: Duration = Duration::from_secs(125);
const TAR_TIMEOUT: Duration = Duration::from_secs(300);

/// Directory name of the toolchain under `<root>/toolchain/`.
const DIR_NAME: &str = "wasi-sdk-34";

/// Release URL and pinned SHA-256 for this machine's architecture.
pub fn wasi_sdk_release() -> (String, &'static str) {
    let (arch, sha) = if std::env::consts::ARCH == "aarch64" {
        ("arm64", SHA256_ARM64)
    } else {
        ("x86_64", SHA256_X86_64)
    };
    (
        format!(
            "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sdk-{WASI_SDK_VERSION}-{arch}-macos.tar.gz"
        ),
        sha,
    )
}

/// Returns the wasi-sdk directory: `$WASI_SDK_PATH` when it has `bin/clang`,
/// else `<root>/toolchain/wasi-sdk-34`, downloading it first if needed.
/// `progress` is called while a download is under way.
pub fn ensure_wasi_sdk(
    root: &Path,
    cancel: &CancelToken,
    log: &mut dyn Write,
    progress: &dyn Fn(),
) -> Result<PathBuf, BuildError> {
    if let Some(p) = std::env::var_os("WASI_SDK_PATH").map(PathBuf::from)
        && p.join("bin/clang").is_file()
    {
        let _ = writeln!(log, "using WASI_SDK_PATH={}", p.display());
        return Ok(p);
    }
    let (url, sha) = wasi_sdk_release();
    ensure_wasi_sdk_from(root, &url, sha, cancel, log, progress)
}

fn is_installed(dir: &Path) -> bool {
    dir.join("bin/clang").is_file()
        && std::fs::read_to_string(dir.join("VERSION")).is_ok_and(|v| v.trim() == WASI_SDK_VERSION)
}

fn io_err(what: &str, path: &Path, e: impl std::fmt::Display) -> BuildError {
    BuildError::Io(format!("{what} {}: {e}", path.display()))
}

/// [`ensure_wasi_sdk`] without the environment override, from `url` checked
/// against `sha256` (lowercase hex).
pub fn ensure_wasi_sdk_from(
    root: &Path,
    url: &str,
    sha256: &str,
    cancel: &CancelToken,
    log: &mut dyn Write,
    progress: &dyn Fn(),
) -> Result<PathBuf, BuildError> {
    let toolchain = root.join("toolchain");
    let dest = toolchain.join(DIR_NAME);
    if is_installed(&dest) {
        return Ok(dest);
    }
    std::fs::create_dir_all(&toolchain).map_err(|e| io_err("cannot create", &toolchain, e))?;
    // Serialise preparation across builds and processes; whoever gets the
    // lock second finds the toolchain installed.
    let _lock = lock_toolchain(&toolchain, cancel)?;
    if is_installed(&dest) {
        return Ok(dest);
    }
    sweep_temp_files(&toolchain);
    progress();
    let free = free_bytes(&toolchain).map_err(|e| io_err("cannot stat", &toolchain, e))?;
    if free < MIN_FREE_BYTES {
        return Err(BuildError::DiskFull(free));
    }
    let suffix = unique_suffix();
    let tarball = toolchain.join(format!(".download-{suffix}.tar.gz"));
    let result = download_and_extract(url, sha256, &tarball, &dest, cancel, log, progress);
    let _ = std::fs::remove_file(&tarball);
    result.map(|()| dest)
}

/// Name of the lock file held while the toolchain is being prepared.
const LOCK_NAME: &str = ".lock";

/// Takes an exclusive `flock` on `<toolchain>/.lock`, polling so that
/// cancellation is honoured while waiting. Released when the file drops.
fn lock_toolchain(toolchain: &Path, cancel: &CancelToken) -> Result<std::fs::File, BuildError> {
    use std::os::fd::AsRawFd;
    let path = toolchain.join(LOCK_NAME);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| io_err("cannot open", &path, e))?;
    loop {
        // SAFETY: `file` owns a valid descriptor for the duration of the call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(file);
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::EWOULDBLOCK) {
            return Err(io_err("cannot lock", &path, err));
        }
        if cancel.is_cancelled() {
            return Err(BuildError::Cancelled);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Removes `.download-*` / `.extract-*` left by an interrupted preparation.
/// Only called while holding the toolchain lock.
fn sweep_temp_files(toolchain: &Path) {
    let Ok(rd) = std::fs::read_dir(toolchain) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(".download-") || name.starts_with(".extract-") {
            let p = e.path();
            let _ = std::fs::remove_dir_all(&p).or_else(|_| std::fs::remove_file(&p));
        }
    }
}

fn download_and_extract(
    url: &str,
    sha256: &str,
    tarball: &Path,
    dest: &Path,
    cancel: &CancelToken,
    log: &mut dyn Write,
    progress: &dyn Fn(),
) -> Result<(), BuildError> {
    let mut curl = Command::new("curl");
    curl.args(["-fL", "--max-time", CURL_MAX_TIME, "-o"])
        .arg(tarball)
        .arg(url);
    run(curl, CURL_TIMEOUT, cancel, log)?;
    progress();

    let actual = file_sha256(tarball).map_err(|e| io_err("cannot read", tarball, e))?;
    if !actual.eq_ignore_ascii_case(sha256) {
        let _ = writeln!(log, "checksum mismatch: expected {sha256}, got {actual}");
        let _ = std::fs::remove_file(tarball);
        return Err(BuildError::Checksum);
    }

    let parent = dest.parent().unwrap_or(dest);
    let tmp = parent.join(format!(".extract-{}", unique_suffix()));
    std::fs::create_dir_all(&tmp).map_err(|e| io_err("cannot create", &tmp, e))?;
    let r = extract_into(tarball, &tmp, dest, cancel, log);
    let _ = std::fs::remove_dir_all(&tmp);
    progress();
    r
}

fn extract_into(
    tarball: &Path,
    tmp: &Path,
    dest: &Path,
    cancel: &CancelToken,
    log: &mut dyn Write,
) -> Result<(), BuildError> {
    let mut tar = Command::new("tar");
    tar.arg("-xzf").arg(tarball).arg("-C").arg(tmp);
    run(tar, TAR_TIMEOUT, cancel, log)?;
    // The tarball holds a single top-level `wasi-sdk-34.0-<arch>-macos/`.
    let top = if tmp.join("VERSION").is_file() {
        tmp.to_path_buf()
    } else {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(tmp)
            .map_err(|e| io_err("cannot read", tmp, e))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        if dirs.len() != 1 {
            return Err(BuildError::Io(
                "unexpected wasi-sdk archive layout".to_string(),
            ));
        }
        dirs.remove(0)
    };
    if !is_installed(&top) {
        return Err(BuildError::Io(format!(
            "the wasi-sdk archive is not version {WASI_SDK_VERSION}"
        )));
    }
    if is_installed(dest) {
        // Installed meanwhile (e.g. a lock-less writer): never replace a
        // toolchain that may be in use; discard our copy.
        return Ok(());
    }
    if dest.exists() {
        // Wrong version or partial: nothing can be using it.
        std::fs::remove_dir_all(dest).map_err(|e| io_err("cannot remove", dest, e))?;
    }
    match std::fs::rename(&top, dest) {
        Ok(()) => Ok(()),
        Err(_) if is_installed(dest) => Ok(()),
        Err(e) => Err(io_err("cannot install", dest, e)),
    }
}

fn file_sha256(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
