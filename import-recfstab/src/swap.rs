//! Swap partition detection from /proc/swaps.

use crate::device::get_device_identifier;
use crate::error::Result;
use crate::fstab::escape_fstab;
use std::fs;

/// Represents a swap entry from /proc/swaps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapInfo {
    /// The device or file path (e.g., /dev/sda2, /swapfile)
    pub filename: String,
    /// Type: partition or file
    pub swap_type: String,
}

/// Read active swap entries from /proc/swaps.
///
/// Parses the kernel's swap file status and returns entries suitable for fstab.
/// Skips zram devices (dynamically managed by systemd-zram-setup).
pub fn read_swaps() -> Result<Vec<SwapInfo>> {
    let content = match fs::read_to_string("/proc/swaps") {
        Ok(c) => c,
        Err(_) => return Ok(Vec::new()), // No /proc/swaps = no swaps
    };

    let mut swaps = Vec::new();

    for line in content.lines().skip(1) {
        // Skip header line
        if let Some(swap) = parse_swap_line(line) {
            // Skip zram devices - they're dynamically created
            if is_zram(&swap.filename) {
                continue;
            }
            swaps.push(swap);
        }
    }

    Ok(swaps)
}

/// Parse a single line from /proc/swaps.
///
/// Format: Filename Type Size Used Priority
/// Fields are whitespace-separated, filename may contain escaped spaces.
fn parse_swap_line(line: &str) -> Option<SwapInfo> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Split on whitespace - first field is filename, second is type
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }

    let filename = unescape_proc_swaps(parts[0]);
    let swap_type = parts[1].to_string();

    if filename.is_empty() {
        return None;
    }

    Some(SwapInfo {
        filename,
        swap_type,
    })
}

/// Unescape special characters in /proc/swaps filenames.
///
/// /proc/swaps uses octal escaping like fstab: \040 for space, etc.
fn unescape_proc_swaps(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            // Try to parse octal escape
            let mut octal = String::new();
            for _ in 0..3 {
                if let Some(&digit) = chars.peek() {
                    if digit.is_ascii_digit() && digit != '8' && digit != '9' {
                        octal.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
            }
            if octal.len() == 3 {
                if let Ok(byte) = u8::from_str_radix(&octal, 8) {
                    result.push(byte as char);
                    continue;
                }
            }
            // Invalid escape - keep the backslash and octal chars
            result.push('\\');
            result.push_str(&octal);
        } else {
            result.push(c);
        }
    }

    result
}

/// Check if a swap path is a zram device.
///
/// zram devices are dynamically created by systemd-zram-setup and should
/// not appear in fstab (they're configured via /etc/systemd/zram-generator.conf).
pub fn is_zram(path: &str) -> bool {
    path.starts_with("/dev/zram")
}

/// Check if a swap path is a swap file (not a block device).
///
/// Swap files should use their path directly in fstab, not UUID lookup.
pub fn is_swap_file(path: &str) -> bool {
    // Block devices start with /dev/
    // Swap files are regular files in the filesystem
    !path.starts_with("/dev/")
}

/// Check if a swap device is under the target root.
///
/// For swap files, checks if the path is under the root.
/// For block devices, they're always "under" any root (system-wide).
pub fn is_swap_under_root(swap: &SwapInfo, root: &str) -> bool {
    if is_swap_file(&swap.filename) {
        // Swap file must be under the root
        if root == "/" {
            // Everything is under "/"
            true
        } else {
            let canonical_root = root.trim_end_matches('/');
            swap.filename == canonical_root
                || swap.filename.starts_with(&format!("{}/", canonical_root))
        }
    } else {
        // Block device swap partitions are system-wide, always include
        true
    }
}

/// Get the fstab source identifier for a swap entry.
///
/// Uses UUID/LABEL for block devices, path for swap files.
pub fn get_swap_identifier(swap: &SwapInfo, id_type: &str) -> String {
    if is_swap_file(&swap.filename) {
        // Swap files use their path directly
        swap.filename.clone()
    } else {
        // Block devices use UUID/LABEL/PARTUUID/PARTLABEL
        get_device_identifier(&swap.filename, id_type)
    }
}

/// Get the fstab target path for a swap entry under the given root.
///
/// Swap files need their path relative to root, block devices use "none".
pub fn get_swap_target(swap: &SwapInfo, root: &str) -> String {
    if is_swap_file(&swap.filename) {
        // Convert swap file path to relative path
        let canonical_root = if root == "/" {
            root.to_string()
        } else {
            root.trim_end_matches('/').to_string()
        };

        if swap.filename == canonical_root {
            "/".to_string()
        } else if let Some(relative) = swap.filename.strip_prefix(&canonical_root) {
            if relative.is_empty() || relative.starts_with('/') {
                if relative.is_empty() {
                    "/".to_string()
                } else {
                    relative.to_string()
                }
            } else {
                format!("/{}", relative)
            }
        } else {
            swap.filename.clone() // Fallback to absolute path
        }
    } else {
        "none".to_string()
    }
}

/// Print swap entries as fstab lines.
pub fn print_swap_entries(swaps: &[SwapInfo], root: &str, id_type: &str) {
    for swap in swaps {
        if !is_swap_under_root(swap, root) {
            continue;
        }

        let identifier = get_swap_identifier(swap, id_type);
        let target = get_swap_target(swap, root);

        println!("# {}", swap.filename);
        println!(
            "{}\t{}\tswap\tdefaults\t0\t0",
            escape_fstab(&identifier),
            escape_fstab(&target),
        );
        println!();
    }
}
