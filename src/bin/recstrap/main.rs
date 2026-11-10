//! recstrap - system extractor
//!
//! Like pacstrap for Arch Linux - extracts the EROFS rootfs to target directory.
//! User does EVERYTHING else manually (partitioning, formatting, fstab, bootloader).
//!
//! Usage:
//!   recstrap /mnt                    # Extract rootfs to /mnt
//!   recstrap /mnt --rootfs /path     # Custom rootfs location (.erofs)
//!   recstrap /mnt --force            # Allow non-empty / non-mount-point target
//!   recstrap /mnt --quiet            # Scripting mode (minimal output)
//!   recstrap /mnt --json             # Progress as JSON lines (for the base-system app)
//!
//! This is NOT archinstall. This is pacstrap: it extracts files and stops;
//! the live install docs give the rest of the installation.
//!
//! Pre-flight checks live in `guards.rs`, one entry per check, in run order.

mod guards;
mod rootfs;

use clap::Parser;
use std::process::ExitCode;
use std::time::Instant;

use guards::{Inputs, Waiver, FORCE_WAIVES, GUARDS};
use install_tools::error::Result;
use install_tools::output::{to_json, to_json_line, ErrorPayload, StrapEvent, STRAP_SCHEMA_VERSION};
use install_tools::session::resolve_target;
use rootfs::extract_erofs;

#[derive(Parser)]
#[command(name = "recstrap")]
#[command(version)]
#[command(about = "Extract the system rootfs to target directory (like pacstrap)")]
#[command(
    long_about = "Extracts the EROFS rootfs image to a target directory. \
    This is the pacstrap equivalent - it only extracts files; \
    the live install docs describe the rest of the installation."
)]
#[command(after_help = "Examples:\n  recstrap /mnt --check       Check the target without extracting\n  recstrap /mnt               Extract the live ISO's system image\n  recstrap /mnt --rootfs /path/to/filesystem.erofs\n\nAfter extraction: preview fstab with 'recfstab /mnt', write it with\n'recfstab /mnt --write', then inspect configuration with 'recsetup inspect --root /mnt'.")]
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

    /// Print progress as JSON lines on stdout (one event per line: start,
    /// progress, verify, done; with --check, one checked); errors go to stderr
    /// as JSON
    #[arg(long)]
    json: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let json = args.json;
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if json {
                eprintln!("{}", to_json(&ErrorPayload::of(&e)));
            } else {
                eprintln!("recstrap: {}", e);
            }
            ExitCode::from(e.code.exit_status())
        }
    }
}

fn run(args: Args) -> Result<()> {
    let started = Instant::now();
    let quiet = args.quiet || args.json;

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

    // The checks that ran: every guard not waived.
    let checks = GUARDS
        .iter()
        .filter(|g| !g.waiver.is_some_and(|w| inputs.waived.contains(&w)))
        .count() as u32;
    if args.check && args.json {
        let event = StrapEvent::Checked {
            schema_version: STRAP_SCHEMA_VERSION,
            target: target_str.into_owned(),
            rootfs: rootfs_str.into_owned(),
            rootfs_bytes: target_root::erofs_image(rootfs)?
                .map_err(|why| install_tools::error::Error::invalid_rootfs_format(&rootfs.to_string_lossy(), &why))?
                .bytes,
            free_bytes: guards::free_bytes(target)?,
            checks,
        };
        println!("{}", to_json_line(&event));
        return Ok(());
    }

    if args.check {
        if !quiet {
            eprintln!();
            eprintln!("{}", "=".repeat(70));
            eprintln!("PRE-FLIGHT CHECK PASSED");
            eprintln!("{}", "=".repeat(70));
            eprintln!();
            eprintln!("Target:    {}", target_str);
            eprintln!("Rootfs:    {}", rootfs_str);
            eprintln!();
            eprintln!("All {checks} validation checks passed.");
            eprintln!("Ready to extract. Run without --check to proceed.");
            eprintln!();
        }
        return Ok(());
    }

    if !quiet {
        eprintln!("Extracting {} to {}...", rootfs_str, target_str);
    }
    let (entries, bytes) = extract_erofs(rootfs, target, args.json)?;

    if args.json {
        println!("{}", to_json_line(&StrapEvent::Verify));
    }
    guards::verify_extraction(target)?;

    if args.json {
        let event = StrapEvent::Done {
            schema_version: STRAP_SCHEMA_VERSION,
            entries,
            bytes,
            duration_ms: started.elapsed().as_millis() as u64,
        };
        println!("{}", to_json_line(&event));
    }
    if !quiet {
        eprintln!("Extracted {} to {}.", rootfs_str, target_str);
        eprintln!("Next: run recfstab on the target to review its mounts, then recsetup to configure the system.");
    }

    Ok(())
}
