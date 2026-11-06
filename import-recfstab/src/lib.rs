//! # recfstab
//!
//! Generate fstab entries from mounted filesystems.
//!
//! `recfstab` is a simple, focused tool that reads the currently mounted filesystems
//! under a specified root directory and outputs properly formatted fstab entries.
//! It's designed to work like Arch Linux's `genfstab` utility.
//!
//! ## Features
//!
//! - Generates fstab entries with UUID identifiers (default) or LABELs
//! - Automatically filters out pseudo-filesystems (proc, sysfs, tmpfs, etc.)
//! - Handles btrfs subvolumes correctly
//! - Outputs standard fstab format compatible with all Linux distributions
//!
//! ## Usage
//!
//! ```bash
//! # Generate fstab for filesystems mounted under /mnt
//! recfstab /mnt >> /mnt/etc/fstab
//!
//! # Use LABELs instead of UUIDs
//! recfstab -L /mnt >> /mnt/etc/fstab
//! ```
//!
//! ## Requirements
//!
//! - Linux system with `findmnt` and `blkid` utilities
//! - Root privileges (for blkid to read device UUIDs)

pub mod device;
pub mod error;
pub mod filter;
pub mod fstab;
pub mod mount;
pub mod swap;

use std::collections::HashSet;
use std::path::Path;

pub use device::{get_device_identifier, IdType};
pub use error::{ErrorCode, RecfstabError, Result};
pub use filter::{filter_options, is_pseudo_filesystem, is_under_root};
pub use fstab::{determine_pass_number, escape_fstab, make_fstab_target};
pub use mount::{get_mounts, MountInfo};
pub use swap::{read_swaps, SwapInfo};

/// Main entry point for the fstab generator.
///
/// Reads mounted filesystems under `root_path` and prints fstab entries to stdout.
///
/// # Arguments
/// * `root_path` - The root directory to scan for mounts
/// * `id_type` - The identifier type to use (UUID, LABEL, PARTUUID, PARTLABEL)
pub fn run(root_path: &str, id_type: IdType) -> Result<()> {
    // Validate input - empty or whitespace-only paths are invalid
    let root_path = root_path.trim();
    if root_path.is_empty() {
        return Err(RecfstabError::root_not_found("(empty path)"));
    }

    let root = Path::new(root_path);

    // Validate root directory
    if !root.exists() {
        return Err(RecfstabError::root_not_found(root_path));
    }
    if !root.is_dir() {
        return Err(RecfstabError::not_a_directory(root_path));
    }

    // Canonicalize the root path to resolve symlinks
    // This ensures we match mount targets correctly even if root is a symlink
    let canonical_root = std::fs::canonicalize(root).map_err(RecfstabError::current_dir_failed)?;
    let root_str = canonical_root.to_string_lossy().to_string();

    // Remove trailing slash for consistent comparison, but keep "/" as-is
    let root_str = if root_str == "/" {
        root_str
    } else {
        root_str.trim_end_matches('/').to_string()
    };

    // Determine the blkid tag to use
    let id_tag = id_type.blkid_tag();

    // Get all mounts using findmnt
    let mounts = get_mounts()?;
    let mut seen_targets: HashSet<String> = HashSet::new();
    let mut found_any = false;

    for mount in mounts {
        // Skip mounts not under our root
        if !is_under_root(&mount.target, &root_str) {
            continue;
        }

        // Skip pseudo-filesystems
        if is_pseudo_filesystem(&mount.fstype) {
            continue;
        }

        // Skip duplicates
        if seen_targets.contains(&mount.target) {
            continue;
        }
        seen_targets.insert(mount.target.clone());

        // Convert absolute target path to path relative to root
        let fstab_target = make_fstab_target(&mount.target, &root_str);

        // Get UUID/LABEL/PARTUUID/PARTLABEL for the device
        let identifier = get_device_identifier(&mount.source, id_tag);

        // Determine fsck pass number
        let pass = determine_pass_number(&fstab_target, &mount.fstype);

        // Filter runtime-only mount options
        let filtered_options = filter_options(&mount.options);

        // Output fstab entry (escape spaces/tabs/newlines per fstab(5))
        println!("# {}", mount.source);
        println!(
            "{}\t{}\t{}\t{}\t0\t{}",
            escape_fstab(&identifier),
            escape_fstab(&fstab_target),
            mount.fstype,
            filtered_options,
            pass
        );
        println!();

        found_any = true;
    }

    // Add swap entries
    if let Ok(swaps) = read_swaps() {
        for swap_entry in &swaps {
            if swap::is_swap_under_root(swap_entry, &root_str) {
                swap::print_swap_entries(std::slice::from_ref(swap_entry), &root_str, id_tag);
                found_any = true;
            }
        }
    }

    if !found_any {
        return Err(RecfstabError::no_filesystems(root_path));
    }

    Ok(())
}
