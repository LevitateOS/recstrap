//! recchroot - Enter chroot with proper bind mounts
//!
//! Like arch-chroot for Arch Linux - sets up /dev, /proc, /sys, /run, then chroots.
//! Does ONE thing: enter chroot properly. Cleans up on exit.
//!
//! Usage:
//!   recchroot /mnt              # Interactive shell
//!   recchroot /mnt passwd       # Run single command
//!
//! This is NOT an installer. This enters chroot. That's it.
//!
//! ## Error Codes
//!
//! | Code | Exit | Description |
//! |------|------|-------------|
//! | E001 | 1 | Target directory does not exist |
//! | E002 | 2 | Target is not a directory |
//! | E003 | 3 | Failed to create mount point directory |
//! | E004 | 4 | Mount operation failed |
//! | E005 | 5 | Unmount operation failed (warning only) |
//! | E006 | 6 | Command execution failed |
//! | E007 | 7 | Must run as root |
//! | E008 | 8 | Target is a protected system path |

use clap::Parser;
use distro_spec::impl_error_code_display;
use distro_spec::shared::error::ToolErrorCode;
use distro_spec::shared::is_root;
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::sys::signal::{signal, SigHandler, Signal};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};

#[derive(Parser)]
#[command(name = "recchroot")]
#[command(version)]
#[command(about = "Enter chroot with proper bind mounts (like arch-chroot)")]
struct Args {
    /// Chroot directory (e.g., /mnt)
    chroot_dir: String,

    /// Command to run (default: /bin/bash)
    #[arg(trailing_var_arg = true)]
    command: Vec<String>,
}

// =============================================================================
// Error Handling
// =============================================================================

/// Error codes for recchroot failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// E001: Target directory does not exist
    TargetNotFound,
    /// E002: Target is not a directory
    NotADirectory,
    /// E003: Failed to create mount point directory
    MkdirFailed,
    /// E004: Mount operation failed
    MountFailed,
    /// E005: Unmount operation failed
    UnmountFailed,
    /// E006: Command execution failed
    CommandFailed,
    /// E007: Must run as root
    NotRoot,
    /// E008: Target is a protected system path
    ProtectedPath,
}

impl ToolErrorCode for ErrorCode {
    fn code(&self) -> &'static str {
        match self {
            ErrorCode::TargetNotFound => "E001",
            ErrorCode::NotADirectory => "E002",
            ErrorCode::MkdirFailed => "E003",
            ErrorCode::MountFailed => "E004",
            ErrorCode::UnmountFailed => "E005",
            ErrorCode::CommandFailed => "E006",
            ErrorCode::NotRoot => "E007",
            ErrorCode::ProtectedPath => "E008",
        }
    }

    fn exit_code(&self) -> u8 {
        match self {
            ErrorCode::TargetNotFound => 1,
            ErrorCode::NotADirectory => 2,
            ErrorCode::MkdirFailed => 3,
            ErrorCode::MountFailed => 4,
            ErrorCode::UnmountFailed => 5,
            ErrorCode::CommandFailed => 6,
            ErrorCode::NotRoot => 7,
            ErrorCode::ProtectedPath => 8,
        }
    }
}

impl_error_code_display!(ErrorCode);

/// A recchroot error with code and context.
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

    pub fn mkdir_failed(path: &Path, source: std::io::Error) -> Self {
        Self::new(
            ErrorCode::MkdirFailed,
            format!("failed to create '{}': {}", path.display(), source),
        )
    }

    pub fn mount_failed(src: &str, target: &Path, source: nix::Error) -> Self {
        Self::new(
            ErrorCode::MountFailed,
            format!(
                "failed to mount '{}' to '{}': {}",
                src,
                target.display(),
                source
            ),
        )
    }

    pub fn command_failed(source: std::io::Error) -> Self {
        Self::new(
            ErrorCode::CommandFailed,
            format!("failed to execute chroot: {}", source),
        )
    }

    pub fn not_root() -> Self {
        Self::new(ErrorCode::NotRoot, "must run as root")
    }

    pub fn protected_path(path: &str) -> Self {
        Self::new(
            ErrorCode::ProtectedPath,
            format!("'{}' is a protected system path", path),
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

/// Required mounts (in order)
const BIND_MOUNTS: &[(&str, &str)] = &[
    ("/proc", "proc"),
    ("/sys", "sys"),
    ("/dev", "dev"),
    ("/run", "run"),
];

/// Optional mounts (only if source exists)
const OPTIONAL_MOUNTS: &[&str] = &["/sys/firmware/efi/efivars"];

// Note: PROTECTED_PATHS is now in distro-spec::shared (single source of truth).
// Use is_protected_path() imported below.

// =============================================================================
// Main
// =============================================================================

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("recchroot: {}", e);
            ExitCode::from(e.code.exit_code())
        }
    }
}

/// Helper to clean up mounts on exit or error
fn cleanup_mounts(mounted: &[PathBuf]) {
    for target in mounted.iter().rev() {
        if let Err(e) = umount2(target, MntFlags::MNT_DETACH) {
            eprintln!(
                "recchroot: warning: E005: failed to unmount '{}': {}",
                target.display(),
                e
            );
        }
    }
}

/// Refuse mount targets that are symlinks: joining them onto the chroot root
/// and mounting would follow the link and could land on a host path.
fn refuse_symlink(target: &Path) -> Result<()> {
    match fs::symlink_metadata(target) {
        Ok(meta) if meta.file_type().is_symlink() => Err(RecError::new(
            ErrorCode::MountFailed,
            format!(
                "refusing to mount over symlink '{}' (it would resolve outside the chroot)",
                target.display()
            ),
        )),
        _ => Ok(()),
    }
}

/// Bind-mount the host's resolv.conf onto `<root>/etc/resolv.conf`.
///
/// Returns the mount target to unmount at cleanup, or `None` when nothing was
/// mounted (no host file, or the target file is a symlink). A placeholder file
/// created for the mount is recorded in `created_files` for removal.
fn bind_host_resolv_conf(root: &Path, created_files: &mut Vec<PathBuf>) -> Result<Option<PathBuf>> {
    let Ok(host_resolv) = fs::canonicalize("/etc/resolv.conf") else {
        return Ok(None);
    };

    let etc = root.join("etc");
    refuse_symlink(&etc)?;
    let target = etc.join("resolv.conf");

    match fs::symlink_metadata(&target) {
        Ok(meta) if meta.file_type().is_symlink() => return Ok(None),
        Ok(meta) if meta.is_file() => {}
        Ok(_) => {
            return Err(RecError::new(
                ErrorCode::MountFailed,
                format!("'{}' is not a regular file", target.display()),
            ))
        }
        Err(_) => {
            if !etc.is_dir() {
                return Ok(None);
            }
            fs::File::create(&target).map_err(|e| RecError::mkdir_failed(&target, e))?;
            created_files.push(target.clone());
        }
    }

    mount(
        Some(&host_resolv),
        &target,
        None::<&str>,
        MsFlags::MS_BIND,
        None::<&str>,
    )
    .map_err(|e| RecError::mount_failed("/etc/resolv.conf", &target, e))?;

    Ok(Some(target))
}

// Use is_protected_path from distro-spec (single source of truth)
use distro_spec::shared::is_protected_path;

fn run() -> Result<u8> {
    let args = Args::parse();

    // Validate empty/whitespace-only paths first (no root needed)
    let chroot_dir_str = args.chroot_dir.trim();
    if chroot_dir_str.is_empty() {
        return Err(RecError::target_not_found("<empty>"));
    }

    let chroot_dir = Path::new(chroot_dir_str);
    if !chroot_dir.exists() {
        return Err(RecError::target_not_found(chroot_dir_str));
    }
    if !chroot_dir.is_dir() {
        return Err(RecError::not_a_directory(chroot_dir_str));
    }

    // Canonicalize to resolve symlinks and ..
    let chroot_canonical = chroot_dir
        .canonicalize()
        .map_err(|e| RecError::new(ErrorCode::TargetNotFound, e.to_string()))?;

    // Check protected paths AFTER canonicalization (catches symlinks to protected paths)
    if is_protected_path(&chroot_canonical) {
        return Err(RecError::protected_path(
            &chroot_canonical.to_string_lossy(),
        ));
    }

    // Check root privileges (needed for mount operations)
    if !is_root() {
        return Err(RecError::not_root());
    }

    let mut mounted: Vec<PathBuf> = Vec::new();

    // Set up signal handlers for cleanup - ignore signals during mount setup
    // so cleanup always happens. We handle SIGINT, SIGTERM, and SIGQUIT.
    unsafe {
        let _ = signal(Signal::SIGINT, SigHandler::SigIgn);
        let _ = signal(Signal::SIGTERM, SigHandler::SigIgn);
        let _ = signal(Signal::SIGQUIT, SigHandler::SigIgn);
    }

    // Setup bind mounts - cleanup on failure
    let mount_result = (|| -> Result<()> {
        for (src, name) in BIND_MOUNTS {
            let target = chroot_canonical.join(name);
            refuse_symlink(&target)?;

            if !target.exists() {
                fs::create_dir_all(&target).map_err(|e| RecError::mkdir_failed(&target, e))?;
            }

            mount(
                Some(Path::new(src)),
                &target,
                None::<&str>,
                MsFlags::MS_BIND | MsFlags::MS_REC,
                None::<&str>,
            )
            .map_err(|e| RecError::mount_failed(src, &target, e))?;
            mounted.push(target.clone());

            // Recursive binds inherit the host's shared propagation; make them
            // slaves so mounts and lazy unmounts inside the chroot never
            // propagate back to the host (same as `mount --make-rslave`).
            mount(
                None::<&str>,
                &target,
                None::<&str>,
                MsFlags::MS_SLAVE | MsFlags::MS_REC,
                None::<&str>,
            )
            .map_err(|e| RecError::mount_failed(src, &target, e))?;
        }
        Ok(())
    })();

    if let Err(e) = mount_result {
        cleanup_mounts(&mounted);
        return Err(e);
    }

    // Optional mounts (like efivars) - warn on failure but continue
    for src in OPTIONAL_MOUNTS {
        let src_path = Path::new(src);
        if !src_path.exists() {
            continue;
        }

        let rel_path = src.strip_prefix('/').unwrap_or(src);
        let target = chroot_canonical.join(rel_path);
        if let Err(e) = refuse_symlink(&target) {
            eprintln!("recchroot: warning: skipping optional mount: {}", e);
            continue;
        }

        if !target.exists() {
            if let Err(e) = fs::create_dir_all(&target) {
                eprintln!(
                    "recchroot: warning: cannot create '{}': {}",
                    target.display(),
                    e
                );
                continue;
            }
        }

        if let Err(e) = mount(
            Some(src_path),
            &target,
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        ) {
            eprintln!(
                "recchroot: warning: cannot mount '{}': {}",
                target.display(),
                e
            );
            continue;
        }

        mounted.push(target);
    }

    // Provide host DNS without writing into the target (or through it into
    // the host): bind-mount the host resolv.conf over the target file for the
    // session only. A symlinked target resolv.conf is left alone; it resolves
    // inside the chroot (typically into the bind-mounted /run).
    let mut created_files: Vec<PathBuf> = Vec::new();
    match bind_host_resolv_conf(&chroot_canonical, &mut created_files) {
        Ok(Some(target)) => mounted.push(target),
        Ok(None) => {}
        Err(e) => eprintln!("recchroot: warning: DNS not configured: {}", e),
    }

    // Determine command to run
    let (cmd, cmd_args): (&str, Vec<&str>) = if args.command.is_empty() {
        ("/bin/bash", vec![])
    } else {
        (
            args.command[0].as_str(),
            args.command[1..].iter().map(|s| s.as_str()).collect(),
        )
    };

    // Run chroot. The parent keeps ignoring SIGINT/SIGTERM/SIGQUIT so it can
    // always clean up, but ignored dispositions survive exec, so restore the
    // defaults in the child or Ctrl-C would not work inside the chroot.
    let mut chroot_cmd = Command::new("chroot");
    chroot_cmd.arg(&chroot_canonical).arg(cmd).args(&cmd_args);
    // SAFETY: only async-signal-safe sigaction calls run between fork and exec.
    unsafe {
        chroot_cmd.pre_exec(|| {
            for sig in [Signal::SIGINT, Signal::SIGTERM, Signal::SIGQUIT] {
                signal(sig, SigHandler::SigDfl).map_err(std::io::Error::from)?;
            }
            Ok(())
        });
    }
    let status = chroot_cmd.status();

    // Always cleanup, even if chroot failed
    cleanup_mounts(&mounted);
    for file in &created_files {
        let _ = fs::remove_file(file);
    }

    let status = status.map_err(RecError::command_failed)?;

    Ok(status.code().unwrap_or(1) as u8)
}

// =============================================================================
// Tests
// =============================================================================
