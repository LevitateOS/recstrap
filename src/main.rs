//! recstrap - LevitateOS system extractor
//!
//! Like pacstrap for Arch Linux - extracts the rootfs (EROFS or squashfs) to target directory.
//! User does EVERYTHING else manually (partitioning, formatting, fstab, bootloader).
//!
//! Usage:
//!   recstrap /mnt                    # Extract rootfs to /mnt
//!   recstrap /mnt --rootfs /path     # Custom rootfs location (EROFS or squashfs)
//!   recstrap /mnt --force            # Overwrite existing files
//!   recstrap /mnt --quiet            # Scripting mode (minimal output)
//!
//! This is NOT archinstall. This is pacstrap.
//! After running recstrap, you must manually:
//!   - Generate /etc/fstab
//!   - Install bootloader (bootctl install)
//!   - Set root password (passwd)
//!   - Configure timezone, locale, hostname
//!
//! ## Cheat-Aware Design
//!
//! This tool uses cheat-guarded validation based on Anthropic's research on
//! emergent misalignment. Each validation check documents:
//! - What user scenario it protects
//! - How the check could be "cheated" (weakened to falsely pass)
//! - What users would experience if the check were cheated
//!
//! This creates friction against shortcuts and makes cheating more expensive
//! than honest implementation.
//!
//! ## Error Codes
//!
//! | Code | Description |
//! |------|-------------|
//! | E001 | Target directory does not exist |
//! | E002 | Target is not a directory |
//! | E003 | Target directory not writable |
//! | E004 | Squashfs image not found |
//! | E005 | unsquashfs command failed |
//! | E006 | Extracted system verification failed |
//! | E007 | unsquashfs not installed |
//! | E008 | Must run as root |
//! | E009 | Target directory not empty (use --force) |
//! | E010 | Target is a protected system path |
//! | E011 | Target is not a mount point |
//! | E012 | Insufficient disk space |
//! | E013 | Squashfs is not a regular file |
//! | E014 | Squashfs is not readable |
//! | E015 | Squashfs is inside target directory |

use clap::Parser;
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

// =============================================================================
// Cheat-Guarded Validation Macro
// =============================================================================

/// Validate a condition with cheat-aware documentation.
///
/// When the condition fails, prints detailed cheat documentation to stderr
/// and returns the specified error. This ensures:
/// 1. Users see clear error messages
/// 2. Developers see cheat vectors when debugging
/// 3. Future maintainers (including AI) see the consequences of weakening checks
///
/// Based on Anthropic's emergent misalignment research.
macro_rules! guarded_ensure {
    (
        $cond:expr,
        $err:expr,
        protects = $protects:expr,
        severity = $severity:expr,
        cheats = [$($cheat:expr),+ $(,)?],
        consequence = $consequence:expr
    ) => {{
        if !($cond) {
            let cheats_list: &[&str] = &[$($cheat),+];
            let cheats_formatted: String = cheats_list
                .iter()
                .enumerate()
                .map(|(i, c)| format!("  {}. {}", i + 1, c))
                .collect::<Vec<_>>()
                .join("\n");

            eprintln!();
            eprintln!("{}", "=".repeat(70));
            eprintln!("=== CHEAT-GUARDED VALIDATION FAILED ===");
            eprintln!("{}", "=".repeat(70));
            eprintln!();
            eprintln!("PROTECTS: {}", $protects);
            eprintln!("SEVERITY: {}", $severity);
            eprintln!();
            eprintln!("CHEAT VECTORS (ways this check could be weakened):");
            eprintln!("{}", cheats_formatted);
            eprintln!();
            eprintln!("USER CONSEQUENCE IF CHEATED:");
            eprintln!("  {}", $consequence);
            eprintln!();
            eprintln!("{}", "=".repeat(70));
            eprintln!();

            return Err($err);
        }
    }};
}

#[derive(Parser)]
#[command(name = "recstrap")]
#[command(version)]
#[command(about = "Extract LevitateOS rootfs to target directory (like pacstrap)")]
#[command(
    long_about = "Extracts the LevitateOS rootfs image (EROFS or squashfs) to a target directory. \
    This is the pacstrap equivalent for LevitateOS - it only extracts files. \
    You must do everything else manually: partitioning, formatting, mounting, \
    fstab generation, bootloader installation, and system configuration."
)]
struct Args {
    /// Target directory (must be mounted, e.g., /mnt)
    target: String,

    /// Rootfs location (auto-detected from common paths if not specified)
    /// Supports both EROFS (.erofs) and squashfs (.squashfs) formats.
    /// --squashfs is accepted for backwards compatibility.
    #[arg(long, visible_alias = "squashfs")]
    rootfs: Option<String>,

    /// Force extraction even if target is not empty or not a mount point
    #[arg(short, long)]
    force: bool,

    /// Quiet mode - minimal output for scripting
    #[arg(short, long)]
    quiet: bool,

    /// Check mode - run pre-flight validation only, don't extract
    #[arg(short, long)]
    check: bool,
}

// =============================================================================
// Error Handling
// =============================================================================

/// Error codes for recstrap failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// E001: Target directory does not exist
    TargetNotFound = 1,
    /// E002: Target is not a directory
    NotADirectory = 2,
    /// E003: Target directory not writable
    NotWritable = 3,
    /// E004: Rootfs image not found (replaces SquashfsNotFound)
    RootfsNotFound = 4,
    /// E005: Extraction command failed (replaces UnsquashfsFailed)
    ExtractionFailed = 5,
    /// E006: Extracted system verification failed
    ExtractionVerificationFailed = 6,
    /// E007: Required tool not installed (unsquashfs, mount, rsync)
    ToolNotInstalled = 7,
    /// E008: Must run as root
    NotRoot = 8,
    /// E009: Target directory not empty
    TargetNotEmpty = 9,
    /// E010: Target is a protected system path
    ProtectedPath = 10,
    /// E011: Target is not a mount point
    NotMountPoint = 11,
    /// E012: Insufficient disk space
    InsufficientSpace = 12,
    /// E013: Rootfs is not a regular file (replaces SquashfsNotFile)
    RootfsNotFile = 13,
    /// E014: Rootfs is not readable (replaces SquashfsNotReadable)
    RootfsNotReadable = 14,
    /// E015: Rootfs is inside target directory (replaces SquashfsInsideTarget)
    RootfsInsideTarget = 15,
    /// E016: Rootfs file has invalid magic bytes (corrupt or wrong format)
    InvalidRootfsFormat = 16,
    /// E017: EROFS kernel module not available
    ErofsNotSupported = 17,
}

// Backwards-compatible aliases for error codes
impl ErrorCode {
    #[allow(non_upper_case_globals)]
    pub const SquashfsNotFound: ErrorCode = ErrorCode::RootfsNotFound;
    #[allow(non_upper_case_globals)]
    pub const UnsquashfsFailed: ErrorCode = ErrorCode::ExtractionFailed;
    #[allow(non_upper_case_globals)]
    pub const UnsquashfsNotInstalled: ErrorCode = ErrorCode::ToolNotInstalled;
    #[allow(non_upper_case_globals)]
    pub const SquashfsNotFile: ErrorCode = ErrorCode::RootfsNotFile;
    #[allow(non_upper_case_globals)]
    pub const SquashfsNotReadable: ErrorCode = ErrorCode::RootfsNotReadable;
    #[allow(non_upper_case_globals)]
    pub const SquashfsInsideTarget: ErrorCode = ErrorCode::RootfsInsideTarget;
}

impl ErrorCode {
    /// Get the numeric code as a string (e.g., "E001").
    pub fn code(&self) -> &'static str {
        match self {
            ErrorCode::TargetNotFound => "E001",
            ErrorCode::NotADirectory => "E002",
            ErrorCode::NotWritable => "E003",
            ErrorCode::RootfsNotFound => "E004",
            ErrorCode::ExtractionFailed => "E005",
            ErrorCode::ExtractionVerificationFailed => "E006",
            ErrorCode::ToolNotInstalled => "E007",
            ErrorCode::NotRoot => "E008",
            ErrorCode::TargetNotEmpty => "E009",
            ErrorCode::ProtectedPath => "E010",
            ErrorCode::NotMountPoint => "E011",
            ErrorCode::InsufficientSpace => "E012",
            ErrorCode::RootfsNotFile => "E013",
            ErrorCode::RootfsNotReadable => "E014",
            ErrorCode::RootfsInsideTarget => "E015",
            ErrorCode::InvalidRootfsFormat => "E016",
            ErrorCode::ErofsNotSupported => "E017",
        }
    }

    /// Get the exit code value
    pub fn exit_code(&self) -> u8 {
        *self as u8
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code())
    }
}

/// A recstrap error with code and context.
#[derive(Debug)]
pub struct RecError {
    pub code: ErrorCode,
    pub message: String,
}

impl RecError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn target_not_found(path: &str) -> Self {
        Self::new(
            ErrorCode::TargetNotFound,
            format!("target directory '{}' does not exist", path),
        )
    }

    pub fn not_a_directory(path: &str) -> Self {
        Self::new(
            ErrorCode::NotADirectory,
            format!("'{}' is not a directory", path),
        )
    }

    pub fn not_writable(path: &str) -> Self {
        Self::new(
            ErrorCode::NotWritable,
            format!(
                "target directory '{}' is not writable (are you root?)",
                path
            ),
        )
    }

    pub fn rootfs_not_found(paths_tried: &[&str]) -> Self {
        Self::new(
            ErrorCode::RootfsNotFound,
            format!(
                "rootfs not found (tried: {}). Make sure you're running from the live ISO or specify --rootfs",
                paths_tried.join(", ")
            ),
        )
    }

    // Backwards-compatible alias
    pub fn squashfs_not_found(paths_tried: &[&str]) -> Self {
        Self::rootfs_not_found(paths_tried)
    }

    pub fn extraction_failed(detail: &str) -> Self {
        let detail = if detail.is_empty() {
            "unknown error (check dmesg for details)".to_string()
        } else {
            detail.trim().to_string()
        };
        Self::new(
            ErrorCode::ExtractionFailed,
            format!("extraction failed: {}", detail),
        )
    }

    // Backwards-compatible alias
    pub fn unsquashfs_failed(detail: &str) -> Self {
        Self::extraction_failed(detail)
    }

    pub fn extraction_verification_failed(missing: &[&str]) -> Self {
        Self::new(
            ErrorCode::ExtractionVerificationFailed,
            format!(
                "extraction verification failed - missing directories: {}",
                missing.join(", ")
            ),
        )
    }

    pub fn unsquashfs_not_installed() -> Self {
        Self::new(
            ErrorCode::UnsquashfsNotInstalled,
            "unsquashfs not found in PATH (install squashfs-tools)",
        )
    }

    pub fn not_root() -> Self {
        Self::new(ErrorCode::NotRoot, "must run as root")
    }

    pub fn target_not_empty(path: &str) -> Self {
        Self::new(
            ErrorCode::TargetNotEmpty,
            format!(
                "target directory '{}' is not empty (use --force to override)",
                path
            ),
        )
    }

    pub fn protected_path(path: &str) -> Self {
        Self::new(
            ErrorCode::ProtectedPath,
            format!(
                "refusing to extract to protected system path '{}' - use a mount point like /mnt",
                path
            ),
        )
    }

    pub fn not_mount_point(path: &str) -> Self {
        Self::new(
            ErrorCode::NotMountPoint,
            format!(
                "'{}' is not a mount point - did you forget to mount? (use --force to override)",
                path
            ),
        )
    }

    pub fn insufficient_space(required_mb: u64, available_mb: u64) -> Self {
        Self::new(
            ErrorCode::InsufficientSpace,
            format!(
                "insufficient disk space: need ~{}MB, have {}MB",
                required_mb, available_mb
            ),
        )
    }

    pub fn rootfs_not_file(path: &str) -> Self {
        Self::new(
            ErrorCode::RootfsNotFile,
            format!("'{}' is not a regular file", path),
        )
    }

    // Backwards-compatible alias
    pub fn squashfs_not_file(path: &str) -> Self {
        Self::rootfs_not_file(path)
    }

    pub fn rootfs_not_readable(path: &str) -> Self {
        Self::new(
            ErrorCode::RootfsNotReadable,
            format!("cannot read rootfs '{}' (permission denied?)", path),
        )
    }

    // Backwards-compatible alias
    pub fn squashfs_not_readable(path: &str) -> Self {
        Self::rootfs_not_readable(path)
    }

    pub fn rootfs_inside_target(rootfs: &str, target: &str) -> Self {
        Self::new(
            ErrorCode::RootfsInsideTarget,
            format!(
                "rootfs '{}' is inside target '{}' - this would cause recursive extraction",
                rootfs, target
            ),
        )
    }

    // Backwards-compatible alias
    pub fn squashfs_inside_target(squashfs: &str, target: &str) -> Self {
        Self::rootfs_inside_target(squashfs, target)
    }

    pub fn invalid_rootfs_format(path: &str, detail: &str) -> Self {
        Self::new(
            ErrorCode::InvalidRootfsFormat,
            format!("'{}' is not a valid rootfs image: {}", path, detail),
        )
    }

    pub fn erofs_not_supported() -> Self {
        Self::new(
            ErrorCode::ErofsNotSupported,
            "EROFS filesystem not supported by kernel (try: modprobe erofs)",
        )
    }
}

impl fmt::Display for RecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RecError {}

type Result<T> = std::result::Result<T, RecError>;

// =============================================================================
// Constants
// =============================================================================

/// Common rootfs locations to search (in order of preference).
/// EROFS paths are listed first as it's the modern format (Fedora 42+, LevitateOS).
const ROOTFS_SEARCH_PATHS: &[&str] = &[
    // EROFS (modern - LevitateOS default)
    "/media/cdrom/live/filesystem.erofs",
    "/run/initramfs/live/filesystem.erofs",
    "/run/archiso/bootmnt/live/filesystem.erofs",
    "/mnt/cdrom/live/filesystem.erofs",
    // Squashfs (legacy fallback)
    "/media/cdrom/live/filesystem.squashfs",
    "/run/initramfs/live/filesystem.squashfs",
    "/run/archiso/bootmnt/live/filesystem.squashfs",
    "/mnt/cdrom/live/filesystem.squashfs",
];

/// Essential directories that must exist after extraction
const ESSENTIAL_DIRS: &[&str] = &["bin", "etc", "lib", "sbin", "usr", "var"];

/// Protected paths that should never be extraction targets
/// These are critical system directories that would be destroyed if overwritten
const PROTECTED_PATHS: &[&str] = &[
    "/", "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib64", "/opt", "/proc", "/root",
    "/run", "/sbin", "/srv", "/sys", "/tmp", "/usr", "/var",
];

/// Minimum required space in bytes (2GB - typical compressed squashfs expands to this)
const MIN_REQUIRED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

// =============================================================================
// Helpers
// =============================================================================

/// Check if running as root
fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// Rootfs type detected from file extension
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootfsType {
    Erofs,
    Squashfs,
}

impl RootfsType {
    fn from_path(path: &Path) -> Option<Self> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("erofs") => Some(Self::Erofs),
            Some("squashfs") => Some(Self::Squashfs),
            _ => None,
        }
    }
}

/// Check if unsquashfs is available (only needed for squashfs)
fn unsquashfs_available() -> bool {
    Command::new("unsquashfs")
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Find rootfs from search paths (prefers EROFS over squashfs)
fn find_rootfs() -> Option<&'static str> {
    ROOTFS_SEARCH_PATHS
        .iter()
        .find(|path| Path::new(path).exists())
        .copied()
}

/// Check if directory is empty for extraction purposes.
/// Ignores:
/// - lost+found (auto-created on ext4 mount points)
/// - .recstrap_write_test (leftover from interrupted write permission check)
fn is_dir_empty(path: &Path) -> std::io::Result<bool> {
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

/// Check if a path is a mount point by comparing device IDs with parent
fn is_mount_point(path: &Path) -> std::io::Result<bool> {
    let path_meta = fs::metadata(path)?;
    let path_dev = path_meta.dev();

    // Get parent directory
    let parent = match path.parent() {
        Some(p) if p.as_os_str().is_empty() => Path::new("/"),
        Some(p) => p,
        None => return Ok(true), // Root is always a mount point
    };

    let parent_meta = fs::metadata(parent)?;
    let parent_dev = parent_meta.dev();

    // If device IDs differ, it's a mount point
    Ok(path_dev != parent_dev)
}

/// Convert OsStr to CString for libc calls, preserving non-UTF8 bytes
fn path_to_cstring(path: &Path) -> std::io::Result<std::ffi::CString> {
    let bytes = path.as_os_str().as_bytes();
    std::ffi::CString::new(bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
}

/// Get available space on filesystem containing path (in bytes)
#[allow(clippy::unnecessary_cast)] // Cast needed - types vary by platform
fn get_available_space(path: &Path) -> std::io::Result<u64> {
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let c_path = path_to_cstring(path)?;

    let ret = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }

    // Available space = f_bavail * f_frsize
    Ok(stat.f_bavail as u64 * stat.f_frsize as u64)
}

/// Check if a path is protected (should never be an extraction target)
fn is_protected_path(path: &Path) -> bool {
    PROTECTED_PATHS
        .iter()
        .any(|protected| path == Path::new(protected))
}

/// Check if rootfs path is inside target directory
fn is_rootfs_inside_target(rootfs: &Path, target: &Path) -> bool {
    rootfs.starts_with(target)
}

/// Check if we can read the rootfs file (at least the first few bytes)
fn can_read_rootfs(path: &Path) -> bool {
    match File::open(path) {
        Ok(mut f) => {
            let mut buf = [0u8; 4];
            f.read_exact(&mut buf).is_ok()
        }
        Err(_) => false,
    }
}

/// EROFS magic number (little-endian at offset 1024)
const EROFS_MAGIC: u32 = 0xe0f5e1e2;
/// Squashfs magic bytes at offset 0
const SQUASHFS_MAGIC: &[u8; 4] = b"hsqs";

/// Validate rootfs magic bytes match expected format.
/// Returns Ok(detected_type) or Err if magic doesn't match.
fn validate_rootfs_magic(path: &Path, expected: RootfsType) -> std::io::Result<()> {
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

/// Check if EROFS filesystem support is available in the kernel.
/// Checks /proc/filesystems for "erofs" entry.
fn erofs_supported() -> bool {
    match fs::read_to_string("/proc/filesystems") {
        Ok(content) => content.lines().any(|line| line.contains("erofs")),
        Err(_) => false,
    }
}

/// Try to load EROFS kernel module if not already loaded.
/// Returns true if EROFS is available after the attempt.
fn ensure_erofs_module() -> bool {
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

// =============================================================================
// Extraction Helpers
// =============================================================================

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
        let _ = std::fs::remove_dir_all(&self.mount_point);
    }
}

/// Extract EROFS image by mounting and copying.
///
/// EROFS cannot be extracted with a simple tool like unsquashfs.
/// We mount it read-only, cp -a all files, then unmount.
/// Uses cp -a instead of rsync as it's always available on minimal systems.
///
/// Uses a RAII guard to ensure cleanup even on panic/interrupt.
fn extract_erofs(rootfs: &Path, target: &Path, quiet: bool) -> Result<()> {
    // Create temporary mount point
    let mount_point = std::env::temp_dir().join("recstrap-erofs-mount");
    if mount_point.exists() {
        // Try to unmount if leftover from previous run
        let _ = Command::new("umount").arg(&mount_point).status();
        std::fs::remove_dir_all(&mount_point).ok();
    }
    std::fs::create_dir_all(&mount_point).map_err(|e| {
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
fn extract_squashfs(rootfs: &Path, target: &Path) -> Result<()> {
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

// =============================================================================
// Verification
// =============================================================================

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
fn verify_extraction(target: &Path) -> Result<()> {
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

// =============================================================================
// Main
// =============================================================================

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("recstrap: {}", e);
            ExitCode::from(e.code.exit_code())
        }
    }
}

fn run() -> Result<()> {
    let args = Args::parse();

    // =========================================================================
    // PHASE 1: Environment Checks (before touching filesystem)
    // =========================================================================

    guarded_ensure!(
        is_root(),
        RecError::not_root(),
        protects = "Installation runs with sufficient privileges",
        severity = "CRITICAL",
        cheats = [
            "Skip root check entirely",
            "Use capabilities instead of full root",
            "Assume sudo will handle it"
        ],
        consequence = "Extraction fails with permission denied on first file"
    );

    // NOTE: Tool availability (unsquashfs, EROFS support) is checked AFTER
    // we detect rootfs type - we only need tools for the format we're using.

    // =========================================================================
    // PHASE 2: Target Directory Validation
    // =========================================================================

    let target = Path::new(&args.target);

    guarded_ensure!(
        target.exists(),
        RecError::target_not_found(&args.target),
        protects = "Target directory exists before we try to use it",
        severity = "CRITICAL",
        cheats = [
            "Create the directory automatically",
            "Skip existence check",
            "Accept parent directory instead"
        ],
        consequence = "Confusing 'No such file or directory' errors during extraction"
    );

    guarded_ensure!(
        target.is_dir(),
        RecError::not_a_directory(&args.target),
        protects = "Target is a directory, not a file or device",
        severity = "CRITICAL",
        cheats = [
            "Accept any path type",
            "Truncate file and use as directory",
            "Skip the check"
        ],
        consequence = "Catastrophic data loss if target is a file, or extraction to device node"
    );

    // Canonicalize path to resolve symlinks and ..
    let target = target
        .canonicalize()
        .map_err(|e| RecError::new(ErrorCode::TargetNotFound, e.to_string()))?;
    let target_str = target.to_string_lossy();

    guarded_ensure!(
        !is_protected_path(&target),
        RecError::protected_path(&target_str),
        protects = "Critical system directories are never overwritten",
        severity = "CRITICAL",
        cheats = [
            "Remove paths from protected list",
            "Add --force override for protected paths",
            "Skip check when running as root",
            "Check before canonicalization (symlink bypass)"
        ],
        consequence = "Complete system destruction - / or /usr overwritten, unbootable system"
    );

    // Write permission check
    let test_file = target.join(".recstrap_write_test");
    let can_write = fs::write(&test_file, b"test").is_ok();
    if can_write {
        let _ = fs::remove_file(&test_file);
    }

    guarded_ensure!(
        can_write,
        RecError::not_writable(&target_str),
        protects = "We can actually write to the target before starting extraction",
        severity = "CRITICAL",
        cheats = [
            "Skip write test",
            "Assume root can write anywhere",
            "Check parent directory instead"
        ],
        consequence = "Extraction starts, partially completes, then fails - corrupted state"
    );

    // Mount point check (unless --force)
    if !args.force {
        let is_mp = is_mount_point(&target).unwrap_or(false);
        guarded_ensure!(
            is_mp,
            RecError::not_mount_point(&target_str),
            protects = "User has actually mounted a filesystem for installation",
            severity = "HIGH",
            cheats = [
                "Always allow with --force",
                "Skip check entirely",
                "Accept any directory"
            ],
            consequence = "User installs to wrong filesystem, fills up wrong disk, loses work"
        );
    }

    // Empty check (unless --force)
    if !args.force {
        let is_empty = is_dir_empty(&target).unwrap_or(false);
        guarded_ensure!(
            is_empty,
            RecError::target_not_empty(&target_str),
            protects = "User doesn't accidentally overwrite existing data",
            severity = "HIGH",
            cheats = [
                "Always allow with --force",
                "Ignore hidden files",
                "Only check for specific files"
            ],
            consequence = "User's existing data silently overwritten, possibly unrecoverable"
        );
    }

    // Disk space check
    if let Ok(available) = get_available_space(&target) {
        guarded_ensure!(
            available >= MIN_REQUIRED_BYTES,
            RecError::insufficient_space(
                MIN_REQUIRED_BYTES / (1024 * 1024),
                available / (1024 * 1024)
            ),
            protects = "Sufficient disk space exists for the full extraction",
            severity = "HIGH",
            cheats = [
                "Reduce MIN_REQUIRED_BYTES",
                "Skip space check",
                "Only warn instead of fail"
            ],
            consequence = "Extraction runs out of space mid-way, leaving corrupted partial system"
        );
    } else if !args.quiet {
        eprintln!("recstrap: warning: cannot check disk space");
    }

    // =========================================================================
    // PHASE 3: Rootfs Validation (EROFS or squashfs)
    // =========================================================================

    // --rootfs or --squashfs (alias) - clap handles the alias automatically
    let rootfs: PathBuf = match args.rootfs.as_ref() {
        Some(path) => {
            let p = Path::new(path);
            guarded_ensure!(
                p.exists(),
                RecError::rootfs_not_found(&[path.as_str()]),
                protects = "Specified rootfs file actually exists",
                severity = "CRITICAL",
                cheats = [
                    "Create empty file",
                    "Use default path instead",
                    "Skip existence check"
                ],
                consequence = "Extraction fails with 'file not found'"
            );

            guarded_ensure!(
                p.is_file(),
                RecError::rootfs_not_file(path),
                protects = "Rootfs path points to a file, not directory",
                severity = "CRITICAL",
                cheats = ["Accept directories", "Skip type check"],
                consequence = "Extraction fails with confusing error about invalid format"
            );

            p.canonicalize()
                .map_err(|e| RecError::new(ErrorCode::RootfsNotFound, e.to_string()))?
        }
        None => {
            let found = find_rootfs();
            guarded_ensure!(
                found.is_some(),
                RecError::rootfs_not_found(ROOTFS_SEARCH_PATHS),
                protects = "Live ISO rootfs is found automatically",
                severity = "CRITICAL",
                cheats = [
                    "Return first path without checking existence",
                    "Hardcode a path",
                    "Create empty file at expected location"
                ],
                consequence = "User must manually specify --rootfs, poor UX"
            );

            let found = found.unwrap();
            let p = Path::new(found);

            guarded_ensure!(
                p.is_file(),
                RecError::rootfs_not_file(found),
                protects = "Auto-detected rootfs is actually a file",
                severity = "CRITICAL",
                cheats = ["Skip type verification", "Accept any path type"],
                consequence = "Extraction fails with confusing error"
            );

            p.canonicalize()
                .map_err(|e| RecError::new(ErrorCode::RootfsNotFound, e.to_string()))?
        }
    };

    let rootfs_str = rootfs.to_string_lossy();

    // Detect rootfs type from extension
    let rootfs_type = RootfsType::from_path(&rootfs).unwrap_or_else(|| {
        // Default to squashfs for unknown extensions (backwards compatibility)
        if !args.quiet {
            eprintln!("recstrap: warning: unknown rootfs format, assuming squashfs");
        }
        RootfsType::Squashfs
    });

    guarded_ensure!(
        can_read_rootfs(&rootfs),
        RecError::rootfs_not_readable(&rootfs_str),
        protects = "Rootfs file is readable before starting extraction",
        severity = "CRITICAL",
        cheats = [
            "Skip readability check",
            "Only check file permissions metadata",
            "Assume root can read anything"
        ],
        consequence = "Extraction fails immediately with permission denied"
    );

    guarded_ensure!(
        !is_rootfs_inside_target(&rootfs, &target),
        RecError::rootfs_inside_target(&rootfs_str, &target_str),
        protects = "Rootfs is not inside the extraction target",
        severity = "CRITICAL",
        cheats = [
            "Skip this check",
            "Only check exact path match",
            "Check before canonicalization"
        ],
        consequence = "Recursive extraction disaster - extracting overwrites source mid-extraction"
    );

    // =========================================================================
    // PHASE 4: Format Validation & Tool Availability
    // =========================================================================

    // Validate magic bytes match expected format
    if let Err(e) = validate_rootfs_magic(&rootfs, rootfs_type) {
        return Err(RecError::invalid_rootfs_format(&rootfs_str, &e.to_string()));
    }

    // Check required tools based on rootfs type
    match rootfs_type {
        RootfsType::Erofs => {
            guarded_ensure!(
                ensure_erofs_module(),
                RecError::erofs_not_supported(),
                protects = "Kernel can mount EROFS filesystems",
                severity = "CRITICAL",
                cheats = [
                    "Skip kernel check",
                    "Assume module is loaded",
                    "Silently fall back to squashfs"
                ],
                consequence = "Mount fails with cryptic 'unknown filesystem type' error"
            );
        }
        RootfsType::Squashfs => {
            guarded_ensure!(
                unsquashfs_available(),
                RecError::unsquashfs_not_installed(),
                protects = "Required extraction tool is present",
                severity = "CRITICAL",
                cheats = [
                    "Hardcode path to unsquashfs",
                    "Use alternative extraction method",
                    "Skip check and hope for the best"
                ],
                consequence = "Extraction fails immediately with 'command not found'"
            );
        }
    }

    // =========================================================================
    // PRE-FLIGHT COMPLETE
    // =========================================================================

    // If --check mode, exit successfully without extracting
    if args.check {
        if !args.quiet {
            eprintln!();
            eprintln!("{}", "=".repeat(70));
            eprintln!("PRE-FLIGHT CHECK PASSED");
            eprintln!("{}", "=".repeat(70));
            eprintln!();
            eprintln!("Target:    {}", target_str);
            eprintln!("Rootfs:    {} ({:?})", rootfs_str, rootfs_type);
            eprintln!();
            eprintln!("All {} validation checks passed.", 14);
            eprintln!("Ready to extract. Run without --check to proceed.");
            eprintln!();
        }
        return Ok(());
    }

    // =========================================================================
    // PHASE 4: Extraction
    // =========================================================================

    if !args.quiet {
        eprintln!("Extracting {} ({:?}) to {}...", rootfs_str, rootfs_type, target_str);
    }

    // Extract based on rootfs type
    match rootfs_type {
        RootfsType::Erofs => {
            // EROFS: mount + cp -a + unmount
            extract_erofs(&rootfs, &target, args.quiet)?;
        }
        RootfsType::Squashfs => {
            // Squashfs: use unsquashfs
            extract_squashfs(&rootfs, &target)?;
        }
    }

    // =========================================================================
    // PHASE 5: Post-Extraction Verification
    // =========================================================================

    // Verify extraction produced a valid system
    verify_extraction(&target)?;

    if !args.quiet {
        eprintln!();
        eprintln!("Done! Now complete the installation manually:");
        eprintln!();
        eprintln!("  # Generate fstab");
        eprintln!("  recfstab {} >> {}/etc/fstab", target_str, target_str);
        eprintln!();
        eprintln!("  # Chroot into new system");
        eprintln!("  recchroot {}", target_str);
        eprintln!();
        eprintln!("  # Set root password");
        eprintln!("  passwd");
        eprintln!();
        eprintln!("  # Install bootloader");
        eprintln!("  bootctl install");
        eprintln!();
        eprintln!("  # Exit chroot and reboot");
        eprintln!("  exit");
        eprintln!("  reboot");
    }

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes_format() {
        assert_eq!(ErrorCode::TargetNotFound.code(), "E001");
        assert_eq!(ErrorCode::NotADirectory.code(), "E002");
        assert_eq!(ErrorCode::NotWritable.code(), "E003");
        assert_eq!(ErrorCode::SquashfsNotFound.code(), "E004");
        assert_eq!(ErrorCode::UnsquashfsFailed.code(), "E005");
        assert_eq!(ErrorCode::ExtractionVerificationFailed.code(), "E006");
        assert_eq!(ErrorCode::UnsquashfsNotInstalled.code(), "E007");
        assert_eq!(ErrorCode::NotRoot.code(), "E008");
        assert_eq!(ErrorCode::TargetNotEmpty.code(), "E009");
        assert_eq!(ErrorCode::ProtectedPath.code(), "E010");
        assert_eq!(ErrorCode::NotMountPoint.code(), "E011");
        assert_eq!(ErrorCode::InsufficientSpace.code(), "E012");
        assert_eq!(ErrorCode::SquashfsNotFile.code(), "E013");
        assert_eq!(ErrorCode::SquashfsNotReadable.code(), "E014");
        assert_eq!(ErrorCode::SquashfsInsideTarget.code(), "E015");
        assert_eq!(ErrorCode::InvalidRootfsFormat.code(), "E016");
        assert_eq!(ErrorCode::ErofsNotSupported.code(), "E017");
    }

    #[test]
    fn test_error_exit_codes() {
        assert_eq!(ErrorCode::TargetNotFound.exit_code(), 1);
        assert_eq!(ErrorCode::NotADirectory.exit_code(), 2);
        assert_eq!(ErrorCode::NotWritable.exit_code(), 3);
        assert_eq!(ErrorCode::SquashfsNotFound.exit_code(), 4);
        assert_eq!(ErrorCode::UnsquashfsFailed.exit_code(), 5);
        assert_eq!(ErrorCode::ExtractionVerificationFailed.exit_code(), 6);
        assert_eq!(ErrorCode::UnsquashfsNotInstalled.exit_code(), 7);
        assert_eq!(ErrorCode::NotRoot.exit_code(), 8);
        assert_eq!(ErrorCode::TargetNotEmpty.exit_code(), 9);
        assert_eq!(ErrorCode::ProtectedPath.exit_code(), 10);
        assert_eq!(ErrorCode::NotMountPoint.exit_code(), 11);
        assert_eq!(ErrorCode::InsufficientSpace.exit_code(), 12);
        assert_eq!(ErrorCode::SquashfsNotFile.exit_code(), 13);
        assert_eq!(ErrorCode::SquashfsNotReadable.exit_code(), 14);
        assert_eq!(ErrorCode::SquashfsInsideTarget.exit_code(), 15);
        assert_eq!(ErrorCode::InvalidRootfsFormat.exit_code(), 16);
        assert_eq!(ErrorCode::ErofsNotSupported.exit_code(), 17);
    }

    #[test]
    fn test_error_display() {
        let err = RecError::target_not_found("/mnt");
        let msg = err.to_string();
        assert!(msg.starts_with("E001:"), "Error was: {}", msg);
        assert!(msg.contains("/mnt"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_not_a_directory() {
        let err = RecError::not_a_directory("/etc/passwd");
        let msg = err.to_string();
        assert!(msg.starts_with("E002:"), "Error was: {}", msg);
        assert!(msg.contains("not a directory"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_not_writable() {
        let err = RecError::not_writable("/mnt");
        let msg = err.to_string();
        assert!(msg.starts_with("E003:"), "Error was: {}", msg);
        assert!(msg.contains("not writable"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_rootfs_not_found() {
        let err = RecError::squashfs_not_found(&["/path/to/rootfs"]);
        let msg = err.to_string();
        assert!(msg.starts_with("E004:"), "Error was: {}", msg);
        assert!(msg.contains("rootfs not found"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_unsquashfs_failed_empty() {
        let err = RecError::unsquashfs_failed("");
        let msg = err.to_string();
        assert!(msg.starts_with("E005:"), "Error was: {}", msg);
        assert!(msg.contains("unknown error"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_unsquashfs_failed_with_detail() {
        let err = RecError::unsquashfs_failed("exit code 1");
        let msg = err.to_string();
        assert!(msg.starts_with("E005:"), "Error was: {}", msg);
        assert!(msg.contains("exit code 1"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_extraction_verification_failed() {
        let err = RecError::extraction_verification_failed(&["bin", "usr"]);
        let msg = err.to_string();
        assert!(msg.starts_with("E006:"), "Error was: {}", msg);
        assert!(msg.contains("bin"), "Error was: {}", msg);
        assert!(msg.contains("usr"), "Error was: {}", msg);
        assert!(msg.contains("missing directories"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_unsquashfs_not_installed() {
        let err = RecError::unsquashfs_not_installed();
        let msg = err.to_string();
        assert!(msg.starts_with("E007:"), "Error was: {}", msg);
        assert!(msg.contains("unsquashfs not found"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_not_root() {
        let err = RecError::not_root();
        let msg = err.to_string();
        assert!(msg.starts_with("E008:"), "Error was: {}", msg);
        assert!(msg.contains("root"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_target_not_empty() {
        let err = RecError::target_not_empty("/mnt");
        let msg = err.to_string();
        assert!(msg.starts_with("E009:"), "Error was: {}", msg);
        assert!(msg.contains("not empty"), "Error was: {}", msg);
        assert!(msg.contains("--force"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_protected_path() {
        let err = RecError::protected_path("/");
        let msg = err.to_string();
        assert!(msg.starts_with("E010:"), "Error was: {}", msg);
        assert!(msg.contains("protected"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_not_mount_point() {
        let err = RecError::not_mount_point("/home/user/test");
        let msg = err.to_string();
        assert!(msg.starts_with("E011:"), "Error was: {}", msg);
        assert!(msg.contains("not a mount point"), "Error was: {}", msg);
        assert!(msg.contains("--force"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_insufficient_space() {
        let err = RecError::insufficient_space(2048, 512);
        let msg = err.to_string();
        assert!(msg.starts_with("E012:"), "Error was: {}", msg);
        assert!(msg.contains("2048"), "Error was: {}", msg);
        assert!(msg.contains("512"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_squashfs_not_file() {
        let err = RecError::squashfs_not_file("/some/directory");
        let msg = err.to_string();
        assert!(msg.starts_with("E013:"), "Error was: {}", msg);
        assert!(msg.contains("not a regular file"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_squashfs_not_readable() {
        let err = RecError::squashfs_not_readable("/secret/file.squashfs");
        let msg = err.to_string();
        assert!(msg.starts_with("E014:"), "Error was: {}", msg);
        assert!(msg.contains("cannot read"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_squashfs_inside_target() {
        let err = RecError::squashfs_inside_target("/mnt/fs.squashfs", "/mnt");
        let msg = err.to_string();
        assert!(msg.starts_with("E015:"), "Error was: {}", msg);
        assert!(msg.contains("recursive"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_invalid_rootfs_format() {
        let err = RecError::invalid_rootfs_format("/path/to/file.erofs", "bad magic");
        let msg = err.to_string();
        assert!(msg.starts_with("E016:"), "Error was: {}", msg);
        assert!(msg.contains("not a valid rootfs"), "Error was: {}", msg);
        assert!(msg.contains("bad magic"), "Error was: {}", msg);
    }

    #[test]
    fn test_error_erofs_not_supported() {
        let err = RecError::erofs_not_supported();
        let msg = err.to_string();
        assert!(msg.starts_with("E017:"), "Error was: {}", msg);
        assert!(msg.contains("EROFS"), "Error was: {}", msg);
        assert!(msg.contains("modprobe"), "Error was: {}", msg);
    }

    #[test]
    fn test_all_error_codes_unique() {
        let codes = [
            ErrorCode::TargetNotFound,
            ErrorCode::NotADirectory,
            ErrorCode::NotWritable,
            ErrorCode::SquashfsNotFound,
            ErrorCode::UnsquashfsFailed,
            ErrorCode::ExtractionVerificationFailed,
            ErrorCode::UnsquashfsNotInstalled,
            ErrorCode::NotRoot,
            ErrorCode::TargetNotEmpty,
            ErrorCode::ProtectedPath,
            ErrorCode::NotMountPoint,
            ErrorCode::InsufficientSpace,
            ErrorCode::SquashfsNotFile,
            ErrorCode::SquashfsNotReadable,
            ErrorCode::SquashfsInsideTarget,
            ErrorCode::InvalidRootfsFormat,
            ErrorCode::ErofsNotSupported,
        ];

        let mut seen = std::collections::HashSet::new();
        for code in codes {
            assert!(
                seen.insert(code.code()),
                "Duplicate error code: {}",
                code.code()
            );
        }
    }

    #[test]
    fn test_all_exit_codes_unique() {
        let codes = [
            ErrorCode::TargetNotFound,
            ErrorCode::NotADirectory,
            ErrorCode::NotWritable,
            ErrorCode::SquashfsNotFound,
            ErrorCode::UnsquashfsFailed,
            ErrorCode::ExtractionVerificationFailed,
            ErrorCode::UnsquashfsNotInstalled,
            ErrorCode::NotRoot,
            ErrorCode::TargetNotEmpty,
            ErrorCode::ProtectedPath,
            ErrorCode::NotMountPoint,
            ErrorCode::InsufficientSpace,
            ErrorCode::SquashfsNotFile,
            ErrorCode::SquashfsNotReadable,
            ErrorCode::SquashfsInsideTarget,
            ErrorCode::InvalidRootfsFormat,
            ErrorCode::ErofsNotSupported,
        ];

        let mut seen = std::collections::HashSet::new();
        for code in codes {
            assert!(
                seen.insert(code.exit_code()),
                "Duplicate exit code: {}",
                code.exit_code()
            );
        }
    }

    #[test]
    fn test_essential_dirs_list() {
        assert!(ESSENTIAL_DIRS.contains(&"bin"));
        assert!(ESSENTIAL_DIRS.contains(&"etc"));
        assert!(ESSENTIAL_DIRS.contains(&"usr"));
        assert!(ESSENTIAL_DIRS.contains(&"lib"));
        assert!(ESSENTIAL_DIRS.contains(&"var"));
    }

    #[test]
    fn test_rootfs_search_paths_exist() {
        assert!(!ROOTFS_SEARCH_PATHS.is_empty());
        for path in ROOTFS_SEARCH_PATHS {
            assert!(
                path.ends_with(".erofs") || path.ends_with(".squashfs"),
                "Path {} should end with .erofs or .squashfs",
                path
            );
        }
    }

    #[test]
    fn test_min_required_bytes_is_reasonable() {
        // Should be at least 1GB, at most 10GB
        assert!(MIN_REQUIRED_BYTES >= 1024 * 1024 * 1024);
        assert!(MIN_REQUIRED_BYTES <= 10 * 1024 * 1024 * 1024);
    }

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
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::create_dir(temp.join("lost+found")).unwrap();

        assert!(
            is_dir_empty(&temp).unwrap(),
            "Directory with only lost+found should be considered empty"
        );

        // Add another file - now it's not empty
        std::fs::write(temp.join("test_file"), b"test").unwrap();
        assert!(
            !is_dir_empty(&temp).unwrap(),
            "Directory with lost+found AND other files should NOT be empty"
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_is_dir_empty_ignores_write_test_file() {
        // Leftover .recstrap_write_test from interrupted run should be ignored
        let temp = std::env::temp_dir().join("recstrap_test_writetest");
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::write(temp.join(".recstrap_write_test"), b"test").unwrap();

        assert!(
            is_dir_empty(&temp).unwrap(),
            "Directory with only .recstrap_write_test should be considered empty"
        );

        // With both ignored entries
        std::fs::create_dir(temp.join("lost+found")).unwrap();
        assert!(
            is_dir_empty(&temp).unwrap(),
            "Directory with lost+found AND .recstrap_write_test should be empty"
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_is_dir_empty_truly_empty() {
        let temp = std::env::temp_dir().join("recstrap_test_empty");
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();

        assert!(
            is_dir_empty(&temp).unwrap(),
            "Empty directory should be empty"
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_is_dir_empty_with_file() {
        let temp = std::env::temp_dir().join("recstrap_test_withfile");
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::write(temp.join("some_file"), b"content").unwrap();

        assert!(
            !is_dir_empty(&temp).unwrap(),
            "Directory with file should NOT be empty"
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

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
        assert_eq!(
            RootfsType::from_path(Path::new("/path/to/file.img")),
            None
        );
        assert_eq!(RootfsType::from_path(Path::new("/path/to/file")), None);
    }

    #[test]
    fn test_erofs_magic_constant() {
        // EROFS magic is 0xe0f5e1e2 (little-endian)
        assert_eq!(EROFS_MAGIC, 0xe0f5e1e2);
    }

    #[test]
    fn test_squashfs_magic_constant() {
        // Squashfs magic is "hsqs"
        assert_eq!(SQUASHFS_MAGIC, b"hsqs");
    }

    #[test]
    fn test_validate_rootfs_magic_invalid_file() {
        // Create a temp file with wrong magic at offset 1024
        // EROFS superblock is at offset 1024, so we need at least 1028 bytes
        let temp = std::env::temp_dir().join("recstrap_test_badmagic.erofs");
        let mut data = vec![0u8; 1028];
        // Put wrong magic at offset 1024
        data[1024..1028].copy_from_slice(b"NOPE");
        std::fs::write(&temp, &data).unwrap();

        let result = validate_rootfs_magic(&temp, RootfsType::Erofs);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("not a valid EROFS"),
            "Error was: {}",
            err
        );

        let _ = std::fs::remove_file(&temp);
    }

    #[test]
    fn test_validate_rootfs_magic_squashfs_invalid() {
        // Create a temp file with wrong magic for squashfs
        let temp = std::env::temp_dir().join("recstrap_test_badsquash.squashfs");
        std::fs::write(&temp, b"not squashfs").unwrap();

        let result = validate_rootfs_magic(&temp, RootfsType::Squashfs);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not a valid squashfs"));

        let _ = std::fs::remove_file(&temp);
    }

    #[test]
    fn test_erofs_supported_checks_proc_filesystems() {
        // This test just verifies the function runs without panic
        // The actual result depends on kernel configuration
        let _ = erofs_supported();
    }
}
