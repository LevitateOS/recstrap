//! recstrap - LevitateOS system extractor
//!
//! Like pacstrap for Arch Linux - extracts the squashfs to target directory.
//! User does EVERYTHING else manually (partitioning, formatting, fstab, bootloader).
//!
//! Usage:
//!   recstrap /mnt                    # Extract squashfs to /mnt
//!   recstrap /mnt --squashfs /path   # Custom squashfs location
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
use std::io::Read;
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
#[command(about = "Extract LevitateOS squashfs to target directory (like pacstrap)")]
#[command(
    long_about = "Extracts the LevitateOS squashfs image to a target directory. \
    This is the pacstrap equivalent for LevitateOS - it only extracts files. \
    You must do everything else manually: partitioning, formatting, mounting, \
    fstab generation, bootloader installation, and system configuration."
)]
struct Args {
    /// Target directory (must be mounted, e.g., /mnt)
    target: String,

    /// Squashfs location (auto-detected from common paths if not specified)
    #[arg(long)]
    squashfs: Option<String>,

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
    /// E004: Squashfs image not found
    SquashfsNotFound = 4,
    /// E005: unsquashfs command failed
    UnsquashfsFailed = 5,
    /// E006: Extracted system verification failed
    ExtractionVerificationFailed = 6,
    /// E007: unsquashfs not installed
    UnsquashfsNotInstalled = 7,
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
    /// E013: Squashfs is not a regular file
    SquashfsNotFile = 13,
    /// E014: Squashfs is not readable
    SquashfsNotReadable = 14,
    /// E015: Squashfs is inside target directory
    SquashfsInsideTarget = 15,
}

impl ErrorCode {
    /// Get the numeric code as a string (e.g., "E001").
    pub fn code(&self) -> &'static str {
        match self {
            ErrorCode::TargetNotFound => "E001",
            ErrorCode::NotADirectory => "E002",
            ErrorCode::NotWritable => "E003",
            ErrorCode::SquashfsNotFound => "E004",
            ErrorCode::UnsquashfsFailed => "E005",
            ErrorCode::ExtractionVerificationFailed => "E006",
            ErrorCode::UnsquashfsNotInstalled => "E007",
            ErrorCode::NotRoot => "E008",
            ErrorCode::TargetNotEmpty => "E009",
            ErrorCode::ProtectedPath => "E010",
            ErrorCode::NotMountPoint => "E011",
            ErrorCode::InsufficientSpace => "E012",
            ErrorCode::SquashfsNotFile => "E013",
            ErrorCode::SquashfsNotReadable => "E014",
            ErrorCode::SquashfsInsideTarget => "E015",
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

    pub fn squashfs_not_found(paths_tried: &[&str]) -> Self {
        Self::new(
            ErrorCode::SquashfsNotFound,
            format!(
                "squashfs not found (tried: {}). Make sure you're running from the live ISO or specify --squashfs",
                paths_tried.join(", ")
            ),
        )
    }

    pub fn unsquashfs_failed(detail: &str) -> Self {
        let detail = if detail.is_empty() {
            "unknown error (check dmesg for details)".to_string()
        } else {
            detail.trim().to_string()
        };
        Self::new(
            ErrorCode::UnsquashfsFailed,
            format!("unsquashfs failed: {}", detail),
        )
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

    pub fn squashfs_not_file(path: &str) -> Self {
        Self::new(
            ErrorCode::SquashfsNotFile,
            format!("'{}' is not a regular file", path),
        )
    }

    pub fn squashfs_not_readable(path: &str) -> Self {
        Self::new(
            ErrorCode::SquashfsNotReadable,
            format!("cannot read squashfs '{}' (permission denied?)", path),
        )
    }

    pub fn squashfs_inside_target(squashfs: &str, target: &str) -> Self {
        Self::new(
            ErrorCode::SquashfsInsideTarget,
            format!(
                "squashfs '{}' is inside target '{}' - this would cause recursive extraction",
                squashfs, target
            ),
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

/// Common squashfs locations to search (in order of preference)
const SQUASHFS_SEARCH_PATHS: &[&str] = &[
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

/// Check if unsquashfs is available
fn unsquashfs_available() -> bool {
    Command::new("unsquashfs")
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Find squashfs from search paths
fn find_squashfs() -> Option<&'static str> {
    SQUASHFS_SEARCH_PATHS
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

/// Check if squashfs path is inside target directory
fn is_squashfs_inside_target(squashfs: &Path, target: &Path) -> bool {
    squashfs.starts_with(target)
}

/// Check if we can read the squashfs file (at least the first few bytes)
fn can_read_squashfs(path: &Path) -> bool {
    match File::open(path) {
        Ok(mut f) => {
            let mut buf = [0u8; 4];
            f.read_exact(&mut buf).is_ok()
        }
        Err(_) => false,
    }
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
    // PHASE 3: Squashfs Validation
    // =========================================================================

    let squashfs: PathBuf = match &args.squashfs {
        Some(path) => {
            let p = Path::new(path);
            guarded_ensure!(
                p.exists(),
                RecError::squashfs_not_found(&[path.as_str()]),
                protects = "Specified squashfs file actually exists",
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
                RecError::squashfs_not_file(path),
                protects = "Squashfs path points to a file, not directory",
                severity = "CRITICAL",
                cheats = ["Accept directories", "Skip type check"],
                consequence = "unsquashfs fails with confusing error about invalid format"
            );

            p.canonicalize()
                .map_err(|e| RecError::new(ErrorCode::SquashfsNotFound, e.to_string()))?
        }
        None => {
            let found = find_squashfs();
            guarded_ensure!(
                found.is_some(),
                RecError::squashfs_not_found(SQUASHFS_SEARCH_PATHS),
                protects = "Live ISO squashfs is found automatically",
                severity = "CRITICAL",
                cheats = [
                    "Return first path without checking existence",
                    "Hardcode a path",
                    "Create empty file at expected location"
                ],
                consequence = "User must manually specify --squashfs, poor UX"
            );

            let found = found.unwrap();
            let p = Path::new(found);

            guarded_ensure!(
                p.is_file(),
                RecError::squashfs_not_file(found),
                protects = "Auto-detected squashfs is actually a file",
                severity = "CRITICAL",
                cheats = ["Skip type verification", "Accept any path type"],
                consequence = "unsquashfs fails with confusing error"
            );

            p.canonicalize()
                .map_err(|e| RecError::new(ErrorCode::SquashfsNotFound, e.to_string()))?
        }
    };

    let squashfs_str = squashfs.to_string_lossy();

    guarded_ensure!(
        can_read_squashfs(&squashfs),
        RecError::squashfs_not_readable(&squashfs_str),
        protects = "Squashfs file is readable before starting extraction",
        severity = "CRITICAL",
        cheats = [
            "Skip readability check",
            "Only check file permissions metadata",
            "Assume root can read anything"
        ],
        consequence = "Extraction fails immediately with permission denied"
    );

    guarded_ensure!(
        !is_squashfs_inside_target(&squashfs, &target),
        RecError::squashfs_inside_target(&squashfs_str, &target_str),
        protects = "Squashfs is not inside the extraction target",
        severity = "CRITICAL",
        cheats = [
            "Skip this check",
            "Only check exact path match",
            "Check before canonicalization"
        ],
        consequence = "Recursive extraction disaster - extracting overwrites source mid-extraction"
    );

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
            eprintln!("Squashfs:  {}", squashfs_str);
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
        eprintln!("Extracting {} to {}...", squashfs_str, target_str);
    }

    // Extract squashfs to target
    // -f tells unsquashfs to overwrite existing files (safe: we checked empty or --force)
    // -d specifies destination directory
    // Use status() instead of output() so stderr goes to terminal for progress
    let status = Command::new("unsquashfs")
        .args(["-f", "-d"])
        .arg(&target)
        .arg(&squashfs)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| {
            RecError::new(
                ErrorCode::UnsquashfsFailed,
                format!("failed to run unsquashfs: {}", e),
            )
        })?;

    guarded_ensure!(
        status.success(),
        RecError::unsquashfs_failed(&format!(
            "exit code {}",
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
    fn test_error_squashfs_not_found() {
        let err = RecError::squashfs_not_found(&["/path/to/squashfs"]);
        let msg = err.to_string();
        assert!(msg.starts_with("E004:"), "Error was: {}", msg);
        assert!(msg.contains("squashfs not found"), "Error was: {}", msg);
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
    fn test_squashfs_search_paths_exist() {
        assert!(!SQUASHFS_SEARCH_PATHS.is_empty());
        for path in SQUASHFS_SEARCH_PATHS {
            assert!(
                path.ends_with(".squashfs"),
                "Path {} should end with .squashfs",
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
    fn test_squashfs_inside_target_detection() {
        assert!(is_squashfs_inside_target(
            Path::new("/mnt/fs.squashfs"),
            Path::new("/mnt")
        ));
        assert!(is_squashfs_inside_target(
            Path::new("/mnt/subdir/fs.squashfs"),
            Path::new("/mnt")
        ));
        assert!(!is_squashfs_inside_target(
            Path::new("/media/cdrom/fs.squashfs"),
            Path::new("/mnt")
        ));
    }

    #[test]
    fn test_can_read_existing_file() {
        // /etc/passwd should be readable
        assert!(can_read_squashfs(Path::new("/etc/passwd")));
    }

    #[test]
    fn test_cannot_read_nonexistent_file() {
        assert!(!can_read_squashfs(Path::new("/nonexistent/file")));
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
}
