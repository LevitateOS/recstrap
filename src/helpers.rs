//! Utility functions for recstrap.

use std::fs::{self, File};
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::constants::ROOTFS_SEARCH_PATHS;

// Re-export from distro-spec (single source of truth)
pub use distro_spec::shared::{is_mount_point, is_protected_path, is_root};

/// Check if unsquashfs is available (only needed for squashfs)
pub fn unsquashfs_available() -> bool {
    Command::new("unsquashfs")
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Find rootfs from search paths (prefers EROFS over squashfs)
pub fn find_rootfs() -> Option<&'static str> {
    ROOTFS_SEARCH_PATHS
        .iter()
        .find(|path| Path::new(path).exists())
        .copied()
}

/// Check if directory is empty for extraction purposes.
/// Ignores:
/// - lost+found (auto-created on ext4 mount points)
/// - .recstrap_write_test (leftover from interrupted write permission check)
pub fn is_dir_empty(path: &Path) -> std::io::Result<bool> {
    for entry in path.read_dir()? {
        let entry = entry?;
        let name = entry.file_name();
        // Ignore filesystem artifacts and our own test files
        if name != "lost+found" && name != ".recstrap_write_test" {
            return Ok(false);
        }
    }
    Ok(true)
}

// Note: is_mount_point() is now in distro-spec::shared::system (single source of truth)
// Re-exported above from distro_spec::shared::is_mount_point

/// Convert OsStr to CString for libc calls, preserving non-UTF8 bytes
pub fn path_to_cstring(path: &Path) -> std::io::Result<std::ffi::CString> {
    let bytes = path.as_os_str().as_bytes();
    std::ffi::CString::new(bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
}

/// Get available space on filesystem containing path (in bytes)
#[allow(clippy::unnecessary_cast)] // Cast needed - types vary by platform
pub fn get_available_space(path: &Path) -> std::io::Result<u64> {
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let c_path = path_to_cstring(path)?;

    let ret = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }

    // Available space = f_bavail * f_frsize
    Ok(stat.f_bavail as u64 * stat.f_frsize as u64)
}

/// Check if rootfs path is inside target directory
pub fn is_rootfs_inside_target(rootfs: &Path, target: &Path) -> bool {
    rootfs.starts_with(target)
}

/// Check if we can read the rootfs file (at least the first few bytes)
pub fn can_read_rootfs(path: &Path) -> bool {
    match File::open(path) {
        Ok(mut f) => {
            let mut buf = [0u8; 4];
            f.read_exact(&mut buf).is_ok()
        }
        Err(_) => false,
    }
}

/// Check if EROFS filesystem support is available in the kernel.
/// Checks /proc/filesystems for "erofs" entry.
pub fn erofs_supported() -> bool {
    match fs::read_to_string("/proc/filesystems") {
        Ok(content) => content.lines().any(|line| line.contains("erofs")),
        Err(_) => false,
    }
}

/// Try to load EROFS kernel module if not already loaded.
/// Returns true if EROFS is available after the attempt.
pub fn ensure_erofs_module() -> bool {
    if erofs_supported() {
        return true;
    }

    // Try to load the module (requires root, which we already checked)
    let _ = Command::new("modprobe")
        .arg("erofs")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // Check again
    erofs_supported()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_mount_point_root() {
        // Root should always be a mount point
        assert!(is_mount_point(Path::new("/")).unwrap());
    }

    #[test]
    fn test_get_available_space_works() {
        // Should succeed on root
        let result = get_available_space(Path::new("/"));
        assert!(result.is_ok());
        // Should return something reasonable (at least 1MB)
        assert!(result.unwrap() > 1024 * 1024);
    }

    #[test]
    fn test_protected_paths_include_critical() {
        assert!(is_protected_path(Path::new("/")));
        assert!(is_protected_path(Path::new("/usr")));
        assert!(is_protected_path(Path::new("/etc")));
        assert!(is_protected_path(Path::new("/bin")));
        assert!(is_protected_path(Path::new("/var")));
        assert!(is_protected_path(Path::new("/home")));
    }

    #[test]
    fn test_protected_paths_allow_mnt() {
        assert!(!is_protected_path(Path::new("/mnt")));
        assert!(!is_protected_path(Path::new("/mnt/target")));
        assert!(!is_protected_path(Path::new("/media/usb")));
    }

    #[test]
    fn test_rootfs_inside_target_detection() {
        assert!(is_rootfs_inside_target(
            Path::new("/mnt/fs.erofs"),
            Path::new("/mnt")
        ));
        assert!(is_rootfs_inside_target(
            Path::new("/mnt/subdir/fs.erofs"),
            Path::new("/mnt")
        ));
        assert!(!is_rootfs_inside_target(
            Path::new("/media/cdrom/fs.erofs"),
            Path::new("/mnt")
        ));
    }

    #[test]
    fn test_can_read_existing_file() {
        // /etc/passwd should be readable
        assert!(can_read_rootfs(Path::new("/etc/passwd")));
    }

    #[test]
    fn test_cannot_read_nonexistent_file() {
        assert!(!can_read_rootfs(Path::new("/nonexistent/file")));
    }

    #[test]
    fn test_path_to_cstring_works() {
        let result = path_to_cstring(Path::new("/tmp/test"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_bytes(), b"/tmp/test");
    }

    #[test]
    fn test_is_dir_empty_with_lost_found() {
        // Create temp dir with lost+found - should be considered empty
        let temp = std::env::temp_dir().join("recstrap_test_lostfound");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();
        fs::create_dir(temp.join("lost+found")).unwrap();

        assert!(
            is_dir_empty(&temp).unwrap(),
            "Directory with only lost+found should be considered empty"
        );

        // Add another file - now it's not empty
        fs::write(temp.join("test_file"), b"test").unwrap();
        assert!(
            !is_dir_empty(&temp).unwrap(),
            "Directory with lost+found AND other files should NOT be empty"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_is_dir_empty_ignores_write_test_file() {
        // Leftover .recstrap_write_test from interrupted run should be ignored
        let temp = std::env::temp_dir().join("recstrap_test_writetest");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();
        fs::write(temp.join(".recstrap_write_test"), b"test").unwrap();

        assert!(
            is_dir_empty(&temp).unwrap(),
            "Directory with only .recstrap_write_test should be considered empty"
        );

        // With both ignored entries
        fs::create_dir(temp.join("lost+found")).unwrap();
        assert!(
            is_dir_empty(&temp).unwrap(),
            "Directory with lost+found AND .recstrap_write_test should be empty"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_is_dir_empty_truly_empty() {
        let temp = std::env::temp_dir().join("recstrap_test_empty");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        assert!(
            is_dir_empty(&temp).unwrap(),
            "Empty directory should be empty"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_is_dir_empty_with_file() {
        let temp = std::env::temp_dir().join("recstrap_test_withfile");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();
        fs::write(temp.join("some_file"), b"content").unwrap();

        assert!(
            !is_dir_empty(&temp).unwrap(),
            "Directory with file should NOT be empty"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_erofs_supported_checks_proc_filesystems() {
        // This test just verifies the function runs without panic
        // The actual result depends on kernel configuration
        let _ = erofs_supported();
    }
}
