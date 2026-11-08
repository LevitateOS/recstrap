//! The one error table for recstrap, recchroot, recfstab and recsetup.
//!
//! Each variant's number is its stable code: it prints as `E0NN` and is the
//! process exit status, so the same failure means the same thing in every
//! tool. Numbers are assigned explicitly and never reused.

use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Code {
    TargetNotFound = 1,
    NotADirectory = 2,
    NotWritable = 3,
    RootfsNotFound = 4,
    ExtractionFailed = 5,
    ExtractionVerificationFailed = 6,
    ToolMissing = 7,
    NotRoot = 8,
    TargetNotEmpty = 9,
    ProtectedPath = 10,
    NotMountPoint = 11,
    InsufficientSpace = 12,
    RootfsNotFile = 13,
    RootfsNotReadable = 14,
    RootfsInsideTarget = 15,
    InvalidRootfsFormat = 16,
    ErofsNotSupported = 17,
    PathUnresolvable = 18,
    CreateFailed = 19,
    MountFailed = 20,
    UnmountFailed = 21,
    CommandFailed = 22,
    ToolFailed = 23,
    NoFilesystems = 24,
    UnsafeMountTarget = 25,
    OnHostRootFilesystem = 26,
    IdentifierUnavailable = 27,
    RemoveFailed = 28,
    /// The target's os-release carries no distro profile (`REC_*` fields).
    NoProfile = 29,
    InvalidRequest = 30,
    ChoiceUnavailable = 31,
    PasswordInput = 32,
    BootFileMissing = 33,
    UnsafeTargetPath = 34,
    BootDeviceMismatch = 35,
    FstabConflict = 36,
}

impl Code {
    pub fn exit_status(self) -> u8 {
        self as u8
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "E{:03}", *self as u8)
    }
}

#[derive(Debug)]
pub struct Error {
    pub code: Code,
    pub message: String,
    /// What to do about it, when the message alone does not say.
    pub remediation: Option<String>,
}

impl Error {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            remediation: None,
        }
    }

    pub fn fix(mut self, remediation: impl Into<String>) -> Self {
        self.remediation = Some(remediation.into());
        self
    }

    pub fn target_not_found(path: &str) -> Self {
        Self::new(
            Code::TargetNotFound,
            format!("target directory '{path}' does not exist"),
        )
    }

    pub fn not_a_directory(path: &str) -> Self {
        Self::new(Code::NotADirectory, format!("'{path}' is not a directory"))
    }

    pub fn not_writable(path: &str) -> Self {
        Self::new(
            Code::NotWritable,
            format!("target directory '{path}' is not writable (are you root?)"),
        )
    }

    pub fn rootfs_not_found(paths_tried: &[&str]) -> Self {
        Self::new(
            Code::RootfsNotFound,
            format!(
                "rootfs not found (tried: {}). Make sure you're running from the live ISO or specify --rootfs",
                paths_tried.join(", ")
            ),
        )
    }

    pub fn extraction_failed(detail: impl fmt::Display) -> Self {
        Self::new(Code::ExtractionFailed, format!("extraction failed: {detail}"))
    }

    pub fn extraction_verification_failed(detail: impl fmt::Display) -> Self {
        Self::new(
            Code::ExtractionVerificationFailed,
            format!("extraction verification failed: {detail}"),
        )
    }

    pub fn tool_missing(tool: &str, package: &str, source: std::io::Error) -> Self {
        Self::new(
            Code::ToolMissing,
            format!("{tool} not found (is {package} installed?): {source}"),
        )
    }

    pub fn not_root() -> Self {
        Self::new(Code::NotRoot, "must run as root")
    }

    pub fn target_not_empty(path: &str) -> Self {
        Self::new(
            Code::TargetNotEmpty,
            format!("target directory '{path}' is not empty (use --force to override)"),
        )
    }

    pub fn protected_path(path: &str) -> Self {
        Self::new(
            Code::ProtectedPath,
            format!("refusing to use protected system path '{path}' - use a mount point like /mnt"),
        )
    }

    pub fn not_mount_point(path: &str) -> Self {
        Self::new(
            Code::NotMountPoint,
            format!(
                "'{path}' is not a mount point - did you forget to mount? (use --force to override)"
            ),
        )
    }

    pub fn insufficient_space(required_mb: u64, available_mb: u64) -> Self {
        Self::new(
            Code::InsufficientSpace,
            format!("insufficient disk space: need ~{required_mb}MB, have {available_mb}MB"),
        )
    }

    pub fn rootfs_not_file(path: &str) -> Self {
        Self::new(Code::RootfsNotFile, format!("'{path}' is not a regular file"))
    }

    pub fn rootfs_not_readable(path: &str) -> Self {
        Self::new(
            Code::RootfsNotReadable,
            format!("cannot read rootfs '{path}' (permission denied?)"),
        )
    }

    pub fn rootfs_inside_target(rootfs: &str, target: &str) -> Self {
        Self::new(
            Code::RootfsInsideTarget,
            format!(
                "rootfs '{rootfs}' is inside target '{target}' - this would cause recursive extraction"
            ),
        )
    }

    pub fn invalid_rootfs_format(path: &str, detail: &str) -> Self {
        Self::new(
            Code::InvalidRootfsFormat,
            format!("'{path}' is not a valid rootfs image: {detail}"),
        )
    }

    pub fn erofs_not_supported() -> Self {
        Self::new(
            Code::ErofsNotSupported,
            "EROFS filesystem not supported by kernel (try: modprobe erofs)",
        )
    }

    pub fn path_unresolvable(path: &str, source: std::io::Error) -> Self {
        Self::new(
            Code::PathUnresolvable,
            format!("cannot resolve '{path}': {source}"),
        )
    }

    pub fn create_failed(path: &Path, source: impl fmt::Display) -> Self {
        Self::new(
            Code::CreateFailed,
            format!("failed to create '{}': {source}", path.display()),
        )
    }

    pub fn mount_failed(source: &str, target: &Path, cause: impl fmt::Display) -> Self {
        Self::new(
            Code::MountFailed,
            format!("failed to mount '{source}' to '{}': {cause}", target.display()),
        )
    }

    pub fn remove_failed(path: &Path, cause: impl fmt::Display) -> Self {
        Self::new(
            Code::RemoveFailed,
            format!("failed to remove '{}': {cause}", path.display()),
        )
    }

    pub fn private_namespace_failed(cause: impl fmt::Display) -> Self {
        Self::new(
            Code::MountFailed,
            format!("cannot give the session a private mount namespace: {cause}"),
        )
    }

    pub fn unmount_failed(target: &Path, cause: impl fmt::Display) -> Self {
        Self::new(
            Code::UnmountFailed,
            format!("failed to unmount '{}': {cause}", target.display()),
        )
    }

    pub fn command_failed(command: &str, source: std::io::Error) -> Self {
        Self::new(
            Code::CommandFailed,
            format!("failed to execute {command}: {source}"),
        )
    }

    pub fn tool_failed(tool: &str, stderr: &str) -> Self {
        let detail = if stderr.trim().is_empty() {
            "unknown error"
        } else {
            stderr.trim()
        };
        Self::new(Code::ToolFailed, format!("{tool} failed: {detail}"))
    }

    pub fn no_filesystems(root: &str) -> Self {
        Self::new(
            Code::NoFilesystems,
            format!(
                "no filesystems found under '{root}' (make sure target filesystems are mounted)"
            ),
        )
    }

    /// `system_dir` is the host directory (`/`, `/home`, ...) whose
    /// filesystem also holds `path`.
    pub fn on_host_root_filesystem(path: &str, system_dir: &str) -> Self {
        let filesystem = if system_dir == "/" {
            "the host's root filesystem".to_string()
        } else {
            format!("the filesystem the host has mounted at '{system_dir}'")
        };
        Self::new(
            Code::OnHostRootFilesystem,
            format!(
                "'{path}' is on {filesystem} (mount the target partition, or pass --allow-host-root-fs)"
            ),
        )
    }

    pub fn identifier_unavailable(device: &str, tag: &str) -> Self {
        Self::new(
            Code::IdentifierUnavailable,
            format!(
                "cannot read {tag} of '{device}' (run as root, or choose another identifier type)"
            ),
        )
    }

    pub fn fstab_conflict(path: &Path, files: &[String]) -> Self {
        Self::new(
            Code::FstabConflict,
            format!("{} already has entries for {}", path.display(), files.join(", ")),
        )
        .fix("check them, then run again with --replace to swap them for the generated entries")
    }

    pub fn unsafe_mount_target(target: &Path, why: &str) -> Self {
        Self::new(
            Code::UnsafeMountTarget,
            format!("refusing to mount over '{}': {why}", target.display()),
        )
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;
        if let Some(remediation) = &self.remediation {
            write!(f, "\n  fix: {remediation}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
