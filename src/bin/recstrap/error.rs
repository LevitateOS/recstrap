//! Error codes and error handling for recstrap.
//!
//! Uses the shared error framework from distro-spec.

use distro_spec::impl_error_code_display;
use distro_spec::shared::error::ToolErrorCode;
use std::fmt;

/// Error codes for recstrap failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// E001: Target directory does not exist
    TargetNotFound = 1,
    /// E002: Target is not a directory
    NotADirectory = 2,
    /// E003: Target directory not writable
    NotWritable = 3,
    /// E004: Rootfs image not found
    RootfsNotFound = 4,
    /// E005: Extraction command failed
    ExtractionFailed = 5,
    /// E006: Extracted system verification failed
    ExtractionVerificationFailed = 6,
    /// E007: Required tool not installed
    #[allow(dead_code)]
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
    /// E013: Rootfs is not a regular file
    RootfsNotFile = 13,
    /// E014: Rootfs is not readable
    RootfsNotReadable = 14,
    /// E015: Rootfs is inside target directory
    RootfsInsideTarget = 15,
    /// E016: Rootfs file has invalid magic bytes (corrupt or wrong format)
    InvalidRootfsFormat = 16,
    /// E017: EROFS kernel module not available
    ErofsNotSupported = 17,
}

impl ToolErrorCode for ErrorCode {
    fn code(&self) -> &'static str {
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

    fn exit_code(&self) -> u8 {
        *self as u8
    }
}

impl_error_code_display!(ErrorCode);

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

    #[allow(dead_code)]
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

    pub fn extraction_verification_failed(missing: &[&str]) -> Self {
        Self::new(
            ErrorCode::ExtractionVerificationFailed,
            format!(
                "extraction verification failed - missing directories: {}",
                missing.join(", ")
            ),
        )
    }

    #[allow(dead_code)]
    pub fn tool_not_installed(tool: &str, package: &str) -> Self {
        Self::new(
            ErrorCode::ToolNotInstalled,
            format!("{} not found in PATH (install {})", tool, package),
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

    pub fn rootfs_not_readable(path: &str) -> Self {
        Self::new(
            ErrorCode::RootfsNotReadable,
            format!("cannot read rootfs '{}' (permission denied?)", path),
        )
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

pub type Result<T> = std::result::Result<T, RecError>;
