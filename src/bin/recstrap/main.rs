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
//! This is NOT archinstall. This is pacstrap: it extracts files and stops;
//! the live install docs give the rest of the installation.
//!
//! Pre-flight checks live in `guards.rs`, one entry per check, in run order.

mod guards;
mod rootfs;

use clap::Parser;
use std::process::ExitCode;

use guards::{Inputs, Waiver, FORCE_WAIVES, GUARDS};
use install_tools::error::Result;
use install_tools::session::resolve_target;
use rootfs::extract_erofs;

#[derive(Parser)]
#[command(name = "recstrap")]
#[command(version)]
#[command(about = "Extract LevitateOS rootfs to target directory (like pacstrap)")]
#[command(
    long_about = "Extracts the LevitateOS EROFS rootfs image to a target directory. \
    This is the pacstrap equivalent for LevitateOS - it only extracts files; \
    the live install docs describe the rest of the installation."
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

    /// Allow extracting onto a filesystem the running system has mounted at /
    /// or another system directory such as /home (never implied by --force)
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
    let rootfs = guards::run_all(&inputs)?;
    let target = &inputs.target;
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

    if !args.quiet {
        eprintln!("Extracting {} to {}...", rootfs_str, target_str);
    }
    extract_erofs(rootfs, target)?;

    guards::verify_extraction(target)?;

    if !args.quiet {
        eprintln!("Extracted {} to {}.", rootfs_str, target_str);
    }

    Ok(())
}
