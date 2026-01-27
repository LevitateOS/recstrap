//! Error codes and error handling for recstrap.

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

pub type Result<T> = std::result::Result<T, RecError>;

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
}
