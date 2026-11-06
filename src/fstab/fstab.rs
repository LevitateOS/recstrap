//! Fstab entry formatting and output.

/// Escape special characters for fstab output.
///
/// fstab(5) requires special characters to be octal-escaped because the line
/// is split on whitespace before parsing. Characters that need escaping:
/// - Space (\040), Tab (\011), Newline (\012), CR (\015) - field separators
/// - Backslash (\134) - escape character itself
/// - Hash (\043) - comment character at start of field
pub fn escape_fstab(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2); // Worst case: all chars escaped
    for c in s.chars() {
        match c {
            '\\' => result.push_str("\\134"),
            ' ' => result.push_str("\\040"),
            '\t' => result.push_str("\\011"),
            '\n' => result.push_str("\\012"),
            '\r' => result.push_str("\\015"), // Carriage return
            '#' => result.push_str("\\043"),
            _ => result.push(c),
        }
    }
    result
}

/// Convert an absolute mount target to a path relative to the root.
///
/// Returns "/" for the root mount, or the relative path for submounts.
/// Handles edge cases like empty strings gracefully.
pub fn make_fstab_target(target: &str, root_str: &str) -> String {
    // Handle empty input
    if target.is_empty() {
        return "/".to_string();
    }

    if target == root_str {
        "/".to_string()
    } else {
        let stripped = target.strip_prefix(root_str).unwrap_or(target);
        if stripped.is_empty() || stripped.starts_with('/') {
            if stripped.is_empty() {
                "/".to_string()
            } else {
                stripped.to_string()
            }
        } else {
            format!("/{}", stripped)
        }
    }
}

/// Determine the fsck pass number for a filesystem.
///
/// - Pass 1: Root filesystem (checked first)
/// - Pass 2: Other filesystems that support fsck
/// - Pass 0: Filesystems that don't need/support fsck
pub fn determine_pass_number(fstab_target: &str, fstype: &str) -> u8 {
    if fstab_target == "/" {
        1
    } else if needs_fsck(fstype) {
        2
    } else {
        0
    }
}

/// Check if a filesystem type supports/needs fsck at boot.
///
/// Note: vfat is excluded because EFI system partitions don't need fsck
/// (they're rarely written to after initial setup, and dosfsck has issues).
/// Note: btrfs is excluded because it doesn't use traditional fsck - it uses
/// `btrfs check` which should only be run manually, not at boot.
pub fn needs_fsck(fstype: &str) -> bool {
    matches!(fstype, "ext2" | "ext3" | "ext4" | "xfs" | "f2fs")
}
