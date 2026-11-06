//! Mount point parsing from findmnt output.

use super::error::{RecfstabError, Result};
use std::process::Command;

/// Represents a single mount point from findmnt output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountInfo {
    pub target: String,
    pub source: String,
    pub fstype: String,
    pub options: String,
}

/// Get all current mounts from the system using findmnt.
pub fn get_mounts() -> Result<Vec<MountInfo>> {
    let output = Command::new("findmnt")
        .args(["-rn", "-o", "TARGET,SOURCE,FSTYPE,OPTIONS"])
        .output()
        .map_err(RecfstabError::findmnt_not_found)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RecfstabError::findmnt_failed(&stderr));
    }

    let mounts_str = String::from_utf8_lossy(&output.stdout);
    let mut mounts = Vec::new();

    for line in mounts_str.lines() {
        if let Some(mount) = parse_mount_line(line) {
            mounts.push(mount);
        }
    }

    Ok(mounts)
}

/// Parse a single line of findmnt output into a MountInfo struct.
///
/// Returns None if the line is malformed or has empty required fields.
pub fn parse_mount_line(line: &str) -> Option<MountInfo> {
    // Skip empty or whitespace-only lines
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let parts: Vec<&str> = line.splitn(4, ' ').collect();
    if parts.len() < 4 {
        return None;
    }

    let target = unescape_findmnt(parts[0]);
    let source = unescape_findmnt(parts[1]);
    let fstype = parts[2].to_string();
    let options = parts[3].to_string();

    // Validate required fields are not empty after unescaping
    if target.is_empty() || fstype.is_empty() {
        return None;
    }

    Some(MountInfo {
        target,
        source,
        fstype,
        options,
    })
}

/// Unescape special characters in findmnt -r output.
///
/// findmnt -r escapes spaces as \x20, tabs as \x09, newlines as \x0a, backslashes as \x5c.
/// Order matters: backslash must be unescaped LAST to avoid double-unescaping.
pub fn unescape_findmnt(s: &str) -> String {
    // Process backslash LAST to avoid turning \\x5cx20 into \ x20 then into \<space>
    s.replace("\\x20", " ")
        .replace("\\x09", "\t")
        .replace("\\x0a", "\n")
        .replace("\\x0d", "\r") // Carriage return (rare but possible)
        .replace("\\x5c", "\\") // Backslash MUST be last
}
