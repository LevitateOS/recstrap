//! Rootfs type detection, validation, and extraction.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::constants::{EROFS_MAGIC, ESSENTIAL_DIRS, SQUASHFS_MAGIC};
use crate::error::{ErrorCode, RecError, Result};
use crate::guarded_ensure;

/// Rootfs type detected from file extension
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootfsType {
    Erofs,
    Squashfs,
}

impl RootfsType {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("erofs") => Some(Self::Erofs),
            Some("squashfs") => Some(Self::Squashfs),
            _ => None,
        }
    }
}

/// Validate rootfs magic bytes match expected format.
/// Returns Ok(()) or Err if magic doesn't match.
pub fn validate_rootfs_magic(path: &Path, expected: RootfsType) -> std::io::Result<()> {
    let mut f = File::open(path)?;

    match expected {
        RootfsType::Erofs => {
            // EROFS superblock is at offset 1024, magic is first 4 bytes
            f.seek(SeekFrom::Start(1024))?;
            let mut buf = [0u8; 4];
            f.read_exact(&mut buf)?;
            let magic = u32::from_le_bytes(buf);
            if magic != EROFS_MAGIC {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "not a valid EROFS image (magic: 0x{:08x}, expected: 0x{:08x})",
                        magic, EROFS_MAGIC
                    ),
                ));
            }
        }
        RootfsType::Squashfs => {
            // Squashfs magic is at offset 0
            let mut buf = [0u8; 4];
            f.read_exact(&mut buf)?;
            if &buf != SQUASHFS_MAGIC {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "not a valid squashfs image (magic: {:?}, expected: {:?})",
                        buf, SQUASHFS_MAGIC
                    ),
                ));
            }
        }
    }

    Ok(())
}

/// RAII guard for EROFS mount cleanup.
/// Ensures unmount and directory removal happen even on panic or interrupt.
struct MountGuard {
    mount_point: PathBuf,
    mounted: bool,
}

impl MountGuard {
    fn new(mount_point: PathBuf) -> Self {
        Self {
            mount_point,
            mounted: false,
        }
    }

    fn set_mounted(&mut self) {
        self.mounted = true;
    }
}

impl Drop for MountGuard {
    fn drop(&mut self) {
        if self.mounted {
            let _ = Command::new("umount").arg(&self.mount_point).status();
        }
        let _ = fs::remove_dir_all(&self.mount_point);
    }
}

/// Extract EROFS image by mounting and copying.
///
/// EROFS cannot be extracted with a simple tool like unsquashfs.
/// We mount it read-only, cp -a all files, then unmount.
/// Uses cp -a instead of rsync as it's always available on minimal systems.
///
/// Uses a RAII guard to ensure cleanup even on panic/interrupt.
pub fn extract_erofs(rootfs: &Path, target: &Path, quiet: bool) -> Result<()> {
    // Create temporary mount point
    let mount_point = std::env::temp_dir().join("recstrap-erofs-mount");
    if mount_point.exists() {
        // Try to unmount if leftover from previous run
        let _ = Command::new("umount").arg(&mount_point).status();
        fs::remove_dir_all(&mount_point).ok();
    }
    fs::create_dir_all(&mount_point).map_err(|e| {
        RecError::new(
            ErrorCode::ExtractionFailed,
            format!("failed to create mount point: {}", e),
        )
    })?;

    // Guard ensures cleanup on any exit path
    let mut guard = MountGuard::new(mount_point.clone());

    // Mount EROFS read-only
    if !quiet {
        eprintln!("Mounting EROFS image...");
    }
    let mount_status = Command::new("mount")
        .args(["-t", "erofs", "-o", "ro,loop"])
        .arg(rootfs)
        .arg(&mount_point)
        .status()
        .map_err(|e| {
            RecError::new(
                ErrorCode::ExtractionFailed,
                format!("failed to run mount: {}", e),
            )
        })?;

    if !mount_status.success() {
        return Err(RecError::new(
            ErrorCode::ExtractionFailed,
            format!(
                "mount failed (exit {}). Is the kernel EROFS module loaded?",
                mount_status.code().unwrap_or(-1)
            ),
        ));
    }

    // Mark as mounted so guard will unmount on drop
    guard.set_mounted();

    // Copy all files using cp -aT (preserves permissions, symlinks, etc.)
    // -a = archive mode (recursive, preserves everything)
    // -T = treat destination as normal file (copy contents, not subdir)
    // cp is always available, unlike rsync
    if !quiet {
        eprintln!("Copying files from EROFS to target (this may take a while)...");
    }

    let cp_status = Command::new("cp")
        .args(["-aT"])
        .arg(&mount_point)
        .arg(target)
        .status()
        .map_err(|e| {
            RecError::new(
                ErrorCode::ExtractionFailed,
                format!("failed to run cp: {}", e),
            )
        })?;

    if !cp_status.success() {
        return Err(RecError::new(
            ErrorCode::ExtractionFailed,
            format!("cp failed (exit {})", cp_status.code().unwrap_or(-1)),
        ));
    }

    if !quiet {
        eprintln!("Extraction complete, cleaning up...");
    }

    // Guard drop will handle unmount and cleanup
    Ok(())
}

/// Extract squashfs image using unsquashfs.
pub fn extract_squashfs(rootfs: &Path, target: &Path) -> Result<()> {
    // -f tells unsquashfs to overwrite existing files (safe: we checked empty or --force)
    // -d specifies destination directory
    let status = Command::new("unsquashfs")
        .args(["-f", "-d"])
        .arg(target)
        .arg(rootfs)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| {
            RecError::new(
                ErrorCode::ExtractionFailed,
                format!("failed to run unsquashfs: {}", e),
            )
        })?;

    guarded_ensure!(
        status.success(),
        RecError::extraction_failed(&format!(
            "unsquashfs exit code {}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string())
        )),
        protects = "Extraction actually completed successfully",
        severity = "CRITICAL",
        cheats = [
            "Ignore exit code",
            "Only check if process ran",
            "Accept partial extraction",
            "Retry without reporting failure"
        ],
        consequence = "Partially extracted system, missing files, unbootable result"
    );

    Ok(())
}

/// Verify that essential directories exist after extraction.
/// These directories are required for a functioning Linux system.
///
/// # Cheat Vectors
///
/// - EASY: Reduce ESSENTIAL_DIRS to fewer directories
/// - EASY: Check for files instead of directories
/// - MEDIUM: Only check if path exists (could be file/symlink)
/// - HARD: Remove verification entirely
///
/// # Consequence if Cheated
///
/// System appears to extract successfully but is missing critical directories.
/// User boots into broken system, /bin or /usr missing, nothing works.
pub fn verify_extraction(target: &Path) -> Result<()> {
    let missing: Vec<&str> = ESSENTIAL_DIRS
        .iter()
        .filter(|dir| !target.join(dir).is_dir())
        .copied()
        .collect();

    guarded_ensure!(
        missing.is_empty(),
        RecError::extraction_verification_failed(&missing),
        protects = "Extracted system has all essential directories",
        severity = "CRITICAL",
        cheats = [
            "Reduce ESSENTIAL_DIRS list",
            "Move missing dirs to 'optional' list",
            "Check exists() instead of is_dir()",
            "Skip verification entirely",
            "Only check one directory"
        ],
        consequence = "System extracts 'successfully' but is incomplete - /bin, /usr, or /etc missing, unbootable"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rootfs_type_from_path() {
        assert_eq!(
            RootfsType::from_path(Path::new("/path/to/file.erofs")),
            Some(RootfsType::Erofs)
        );
        assert_eq!(
            RootfsType::from_path(Path::new("/path/to/file.squashfs")),
            Some(RootfsType::Squashfs)
        );
        assert_eq!(RootfsType::from_path(Path::new("/path/to/file.img")), None);
        assert_eq!(RootfsType::from_path(Path::new("/path/to/file")), None);
    }

    #[test]
    fn test_validate_rootfs_magic_invalid_file() {
        // Create a temp file with wrong magic at offset 1024
        // EROFS superblock is at offset 1024, so we need at least 1028 bytes
        let temp = std::env::temp_dir().join("recstrap_test_badmagic.erofs");
        let mut data = vec![0u8; 1028];
        // Put wrong magic at offset 1024
        data[1024..1028].copy_from_slice(b"NOPE");
        fs::write(&temp, &data).unwrap();

        let result = validate_rootfs_magic(&temp, RootfsType::Erofs);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("not a valid EROFS"),
            "Error was: {}",
            err
        );

        let _ = fs::remove_file(&temp);
    }

    #[test]
    fn test_validate_rootfs_magic_squashfs_invalid() {
        // Create a temp file with wrong magic for squashfs
        let temp = std::env::temp_dir().join("recstrap_test_badsquash.squashfs");
        fs::write(&temp, b"not squashfs").unwrap();

        let result = validate_rootfs_magic(&temp, RootfsType::Squashfs);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not a valid squashfs"));

        let _ = fs::remove_file(&temp);
    }
}
