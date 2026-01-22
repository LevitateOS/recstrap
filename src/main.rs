//! recstrap - LevitateOS system extractor
//!
//! Like pacstrap for Arch Linux - extracts the squashfs to target directory.
//! User does EVERYTHING else manually (partitioning, formatting, fstab, bootloader).
//!
//! Usage:
//!   recstrap /mnt                    # Extract squashfs to /mnt
//!   recstrap /mnt --squashfs /path   # Custom squashfs location
//!
//! This is NOT archinstall. This is pacstrap.
//! After running recstrap, you must manually:
//!   - Generate /etc/fstab
//!   - Install bootloader (bootctl install)
//!   - Set root password (passwd)
//!   - Configure timezone, locale, hostname

use anyhow::{bail, Result};
use clap::Parser;
use std::path::Path;
use std::process::Command;

#[derive(Parser)]
#[command(name = "recstrap")]
#[command(about = "Extract LevitateOS squashfs to target directory (like pacstrap)")]
struct Args {
    /// Target directory (must be mounted, e.g., /mnt)
    target: String,

    /// Squashfs location (default: /media/cdrom/live/filesystem.squashfs)
    #[arg(long)]
    squashfs: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Validate target exists and is a directory
    let target = Path::new(&args.target);
    if !target.exists() {
        bail!("Target directory {} does not exist", args.target);
    }
    if !target.is_dir() {
        bail!("{} is not a directory", args.target);
    }

    // Find squashfs
    let squashfs = args
        .squashfs
        .unwrap_or_else(|| "/media/cdrom/live/filesystem.squashfs".to_string());

    if !Path::new(&squashfs).exists() {
        bail!(
            "Squashfs not found at {}\n\
             Make sure you're running from the live ISO.",
            squashfs
        );
    }

    println!("Extracting {} to {}...", squashfs, args.target);

    // Extract squashfs to target
    let status = Command::new("unsquashfs")
        .args(["-f", "-d", &args.target, &squashfs])
        .status()?;

    if !status.success() {
        bail!("unsquashfs failed");
    }

    println!();
    println!("Done! Now complete the installation manually:");
    println!();
    println!("  # Generate fstab");
    println!("  genfstab -U /mnt >> /mnt/etc/fstab");
    println!();
    println!("  # Chroot into new system");
    println!("  arch-chroot /mnt");
    println!();
    println!("  # Set root password");
    println!("  passwd");
    println!();
    println!("  # Install bootloader");
    println!("  bootctl install");
    println!();
    println!("  # Exit chroot and reboot");
    println!("  exit");
    println!("  reboot");

    Ok(())
}
