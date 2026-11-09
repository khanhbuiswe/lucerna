//! Free disk space checks.

use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// Free disk required before a fetch or build (1.2 GiB).
pub const MIN_FREE_BYTES: u64 = 1_288_490_189;

/// Bytes available to unprivileged users on the volume holding `path`.
pub fn free_bytes(path: &Path) -> io::Result<u64> {
    let c = CString::new(path.as_os_str().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut st = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c` is a valid C string and `st` is a valid out pointer.
    let rc = unsafe { libc::statvfs(c.as_ptr(), st.as_mut_ptr()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: statvfs succeeded and initialised the struct.
    let st = unsafe { st.assume_init() };
    Ok(u64::from(st.f_bavail).saturating_mul(st.f_frsize))
}
