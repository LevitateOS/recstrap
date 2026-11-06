//! recstrap - LevitateOS system extractor
//!
//! Like pacstrap for Arch Linux - extracts the EROFS rootfs to target directory.
//! User does EVERYTHING else manually (partitioning, formatting, fstab, bootloader).
//!
//! Usage:
//!   recstrap /mnt                    # Extract rootfs to /mnt
//!   recstrap /mnt --rootfs /path     # Custom rootfs location (.erofs)
//!   recstrap /mnt --force            # Allow non-empty / non-mount-point target
//!   recstrap /mnt --quiet            # Scripting mode (minimal output)
//!
//! This is NOT archinstall. This is pacstrap.
//! After running recstrap, you must manually:
//!   - Generate /etc/fstab
//!   - Install bootloader (bootctl install)
//!   - Set root password (passwd)
//!   - Configure timezone, locale, hostname
//!
//! Pre-flight checks live in `guards.rs`, one entry per check, in run order.

mod guards;
mod helpers;
mod rootfs;

use clap::Parser;
use std::io::IsTerminal;
use std::process::ExitCode;

use guards::{Inputs, Waiver, FORCE_WAIVES, GUARDS};
use install_tools::error::Result;
use install_tools::session::resolve_target;
use helpers::{prompt_for_user_creation, regenerate_ssh_host_keys};
use rootfs::extract_erofs;

#[derive(Parser)]
#[command(name = "recstrap")]
#[command(version)]
#[command(about = "Extract LevitateOS rootfs to target directory (like pacstrap)")]
#[command(
    long_about = "Extracts the LevitateOS EROFS rootfs image to a target directory. \
    This is the pacstrap equivalent for LevitateOS - it only extracts files. \
    You must do everything else manually: partitioning, formatting, mounting, \
    fstab generation, bootloader installation, and system configuration."
)]
struct Args {
    /// Target directory (must be mounted, e.g., /mnt)
    target: String,

    /// Rootfs image (auto-detected on the live ISO if not specified)
    #[arg(long)]
    rootfs: Option<String>,

    /// Shorthand for --allow-non-mountpoint --allow-nonempty
    #[arg(short, long)]
    force: bool,

    /// Allow a target directory that is not a mount point
    #[arg(long)]
    allow_non_mountpoint: bool,

    /// Allow a target directory that already contains files
    #[arg(long)]
    allow_nonempty: bool,

    /// Allow extracting onto the filesystem that holds the running system's /
    /// (never implied by --force)
    #[arg(long)]
    allow_host_root_fs: bool,

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
            ExitCode::from(e.code.exit_status())
        }
    }
}

fn run() -> Result<()> {
    let args = Args::parse();

    let mut waived = Vec::new();
    if args.force {
        waived.extend_from_slice(FORCE_WAIVES);
    }
    for (flag, waiver) in [
        (args.allow_non_mountpoint, Waiver::NonMountpoint),
        (args.allow_nonempty, Waiver::Nonempty),
        (args.allow_host_root_fs, Waiver::HostRootFs),
    ] {
        if flag {
            waived.push(waiver);
        }
    }

    let target = resolve_target(&args.target)?;
    let inputs = Inputs::new(target, args.rootfs.as_deref(), waived);
    guards::run_all(&inputs)?;
    let target = &inputs.target;
    let rootfs = inputs.rootfs.as_deref().expect("rootfs_found guard passed");
    let target_str = target.to_string_lossy();
    let rootfs_str = rootfs.to_string_lossy();

    if args.check {
        if !args.quiet {
            eprintln!();
            eprintln!("{}", "=".repeat(70));
            eprintln!("PRE-FLIGHT CHECK PASSED");
            eprintln!("{}", "=".repeat(70));
            eprintln!();
            eprintln!("Target:    {}", target_str);
            eprintln!("Rootfs:    {}", rootfs_str);
            eprintln!();
            eprintln!("All {} validation checks passed.", GUARDS.len());
            eprintln!("Ready to extract. Run without --check to proceed.");
            eprintln!();
        }
        return Ok(());
    }

    // =========================================================================
    // PHASE 5: Extraction
    // =========================================================================

    if !args.quiet {
        eprintln!("Extracting {} to {}...", rootfs_str, target_str);
    }
    extract_erofs(rootfs, target)?;

    // =========================================================================
    // PHASE 6: Post-Extraction Verification
    // =========================================================================

    // Verify extraction produced a valid system
    guards::verify_extraction(target)?;

    // =========================================================================
    // PHASE 7: Security Hardening
    // =========================================================================

    // SECURITY: Regenerate SSH host keys to prevent MITM attacks.
    // The rootfs image contains pre-generated keys shared by all installations.
    // Each installed system needs unique keys.
    if !args.quiet {
        eprintln!("Regenerating SSH host keys...");
    }
    if let Err(e) = regenerate_ssh_host_keys(target, args.quiet) {
        // Not fatal (keys can be regenerated in the chroot), but a security
        // problem, so it is reported even with --quiet.
        eprintln!("recstrap: warning: SSH key regeneration failed: {}", e);
        eprintln!("         Run 'ssh-keygen -A' in chroot to generate keys manually");
    }

    // =========================================================================
    // PHASE 8: Optional User Creation Setup
    // =========================================================================

    // Prompt for initial user creation (Option A: Arch-style)
    // This creates a setup script in /root that user runs in chroot
    if !args.quiet && std::io::stdin().is_terminal() {
        // Only prompt when someone can answer (not with --quiet or piped stdin)
        let _ = prompt_for_user_creation(target);
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
        eprintln!("  # Set up initial user (if you created one above)");
        eprintln!("  bash /root/setup-initial-user.sh");
        eprintln!();
        eprintln!("  # OR: Set root password manually (account is locked by default)");
        eprintln!("  passwd root");
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
