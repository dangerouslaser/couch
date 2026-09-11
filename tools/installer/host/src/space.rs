//! Available bytes for the current user, for backup admission before boot writes.
//! This is a preflight observation, not a reservation; writes still handle ENOSPC.
use anyhow::{ensure, Context, Result};
use std::path::Path;

pub fn available(path: &Path) -> Result<u64> {
    let path = path
        .canonicalize()
        .context("Backup directory is unavailable")?;
    ensure!(path.is_dir(), "Expected a backup directory");
    platform_available(&path)
}

// libc counter widths differ between macOS and Linux ABIs.
#[cfg(unix)]
#[allow(clippy::unnecessary_cast)]
fn platform_available(path: &Path) -> Result<u64> {
    use std::{ffi::CString, mem::MaybeUninit, os::unix::ffi::OsStrExt};
    let path = CString::new(path.as_os_str().as_bytes())?;
    let mut stats = MaybeUninit::<libc::statvfs>::uninit();
    // The NUL-terminated path remains live; statvfs initializes stats on success.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error()).context("Could not measure backup free space");
    }
    let stats = unsafe { stats.assume_init() };
    (stats.f_bavail as u64)
        .checked_mul(stats.f_frsize as u64)
        .context("Backup free-space count overflow")
}

#[cfg(windows)]
fn platform_available(path: &Path) -> Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut bytes = 0u64;
    // Request the caller-available count, respecting volume quotas. The two
    // optional total counters are intentionally omitted.
    if unsafe {
        GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut bytes,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error()).context("Could not measure backup free space");
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measures_real_directory_and_rejects_missing_or_file_targets() {
        let root = tempfile::tempdir().unwrap();
        assert!(available(root.path()).unwrap() > 0);
        assert!(available(&root.path().join("missing")).is_err());
        let file = root.path().join("file");
        std::fs::write(&file, []).unwrap();
        assert!(available(&file).is_err());
    }
}
