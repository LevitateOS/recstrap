//! Rootfs type detection, validation, and extraction.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::constants::{EROFS_MAGIC, ESSENTIAL_DIRS};
use crate::error::{ErrorCode, RecError, Result};
use crate::guarded_ensure;

/// Rootfs type detected from file extension
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootfsType {
    Erofs,
}

impl RootfsType {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("erofs") => Some(Self::Erofs),
            _ => None,
        }
    }
}

/// Validate rootfs magic bytes match expected format.
/// Returns Ok(()) or Err if magic doesn't match.
pub fn validate_rootfs_magic(path: &Path, expected: RootfsType) -> std::io::Result<()> {
    let mut f = File::open(path)?;

    if expected == RootfsType::Erofs {
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

    Ok(())
}

/// RAII guard for EROFS mount cleanup.
///
/// Unmounts on drop and removes the (empty) mount directory only after a
/// successful unmount, so a still-mounted image is never deleted through.
/// SIGINT/SIGTERM are ignored by recstrap while the guard is live (see
/// `extract_erofs`), so an interrupt ends the copy and this guard still runs.
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
            let unmounted = Command::new("umount")
                .arg(&self.mount_point)
                .status()
                .map(|status| status.success())
                .unwrap_or(false);
            if !unmounted {
                eprintln!(
                    "recstrap: warning: failed to unmount '{}'; leaving it in place",
                    self.mount_point.display()
                );
                return;
            }
        }
        // remove_dir (not remove_dir_all): the directory must be empty now.
        let _ = fs::remove_dir(&self.mount_point);
    }
}

/// Restores the previous SIGINT/SIGTERM dispositions when dropped.
struct IgnoreInterrupts {
    previous: [(libc::c_int, libc::sighandler_t); 2],
}

impl IgnoreInterrupts {
    fn new() -> Self {
        let mut previous = [(libc::SIGINT, libc::SIG_DFL), (libc::SIGTERM, libc::SIG_DFL)];
        for (sig, old) in previous.iter_mut() {
            // SAFETY: installing SIG_IGN has no handler code to be unsafe in.
            *old = unsafe { libc::signal(*sig, libc::SIG_IGN) };
        }
        Self { previous }
    }
}

impl Drop for IgnoreInterrupts {
    fn drop(&mut self) {
        for (sig, old) in self.previous {
            // SAFETY: restores the disposition captured in `new`.
            unsafe {
                libc::signal(sig, old);
            }
        }
    }
}

/// Create a fresh, private mount directory (mkdtemp) so concurrent runs never
/// share one and nothing pre-existing is ever deleted.
fn create_private_mount_dir() -> Result<PathBuf> {
    use std::ffi::{CString, OsString};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let base = if Path::new("/run").is_dir() {
        PathBuf::from("/run")
    } else {
        std::env::temp_dir()
    };
    let template = CString::new(base.join("recstrap-erofs.XXXXXX").as_os_str().as_bytes())
        .map_err(|e| RecError::new(ErrorCode::ExtractionFailed, e.to_string()))?;
    let mut raw = template.into_bytes_with_nul();
    // SAFETY: `raw` is a writable NUL-terminated template ending in XXXXXX.
    let ptr = unsafe { libc::mkdtemp(raw.as_mut_ptr().cast()) };
    if ptr.is_null() {
        return Err(RecError::new(
            ErrorCode::ExtractionFailed,
            format!(
                "failed to create mount point under {}: {}",
                base.display(),
                std::io::Error::last_os_error()
            ),
        ));
    }
    raw.pop();
    Ok(PathBuf::from(OsString::from_vec(raw)))
}

/// Extract EROFS image by mounting and copying.
///
/// EROFS cannot be extracted with a simple tool like unsquashfs.
/// We mount it read-only, cp -a all files, then unmount.
/// Uses cp -a instead of rsync as it's always available on minimal systems.
///
/// Uses a RAII guard to ensure cleanup even on panic/interrupt.
pub fn extract_erofs(rootfs: &Path, target: &Path, quiet: bool) -> Result<()> {
    let mount_point = create_private_mount_dir()?;

    // Ignore interrupts in recstrap itself (children get defaults back), so
    // Ctrl-C stops mount/cp and the guard below still unmounts and cleans up.
    let _interrupts = IgnoreInterrupts::new();

    // Guard ensures cleanup on any exit path
    let mut guard = MountGuard::new(mount_point.clone());

    // Mount EROFS read-only
    if !quiet {
        eprintln!("Mounting EROFS image...");
    }
    let mount_status = with_default_signals(Command::new("mount"))
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

    let cp_status = with_default_signals(Command::new("cp"))
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

/// Restore default SIGINT/SIGTERM handling in a child (ignored dispositions
/// would otherwise survive exec and make the child uninterruptible).
fn with_default_signals(mut cmd: Command) -> Command {
    use std::os::unix::process::CommandExt;
    // SAFETY: only async-signal-safe signal() calls run between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            Ok(())
        });
    }
    cmd
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
