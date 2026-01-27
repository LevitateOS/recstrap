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

mod constants;
mod error;
mod helpers;
mod rootfs;
mod validation;

use clap::Parser;
use distro_spec::shared::error::ToolErrorCode;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use constants::{MIN_REQUIRED_BYTES, ROOTFS_SEARCH_PATHS};
use error::{ErrorCode, RecError, Result};
use helpers::{
    can_read_rootfs, ensure_erofs_module, find_rootfs, get_available_space, is_dir_empty,
    is_mount_point, is_protected_path, is_root, is_rootfs_inside_target, regenerate_ssh_host_keys,
    unsquashfs_available,
};
use rootfs::{extract_erofs, extract_squashfs, validate_rootfs_magic, verify_extraction, RootfsType};

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
                RecError::tool_not_installed("unsquashfs", "squashfs-tools"),
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
    // PHASE 5: Extraction
    // =========================================================================

    if !args.quiet {
        eprintln!(
            "Extracting {} ({:?}) to {}...",
            rootfs_str, rootfs_type, target_str
        );
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
    // PHASE 6: Post-Extraction Verification
    // =========================================================================

    // Verify extraction produced a valid system
    verify_extraction(&target)?;

    // =========================================================================
    // PHASE 7: Security Hardening
    // =========================================================================

    // SECURITY: Regenerate SSH host keys to prevent MITM attacks.
    // The rootfs image contains pre-generated keys shared by all installations.
    // Each installed system needs unique keys.
    if !args.quiet {
        eprintln!("Regenerating SSH host keys...");
    }
    if let Err(e) = regenerate_ssh_host_keys(&target, args.quiet) {
        // Warning only - not fatal since user can regenerate manually
        if !args.quiet {
            eprintln!("recstrap: warning: SSH key regeneration failed: {}", e);
            eprintln!("         Run 'ssh-keygen -A' in chroot to generate keys manually");
        }
    }

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

