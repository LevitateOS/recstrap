//! Device identifier lookup (UUID/LABEL/PARTUUID/PARTLABEL).

use std::process::Command;

/// Device identifier type for fstab entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IdType {
    /// Use filesystem UUID (default)
    #[default]
    Uuid,
    /// Use filesystem LABEL
    Label,
    /// Use partition UUID (GPT PARTUUID)
    Partuuid,
    /// Use partition LABEL (GPT PARTLABEL)
    Partlabel,
}

impl IdType {
    /// Get the blkid tag name for this identifier type.
    pub fn blkid_tag(&self) -> &'static str {
        match self {
            IdType::Uuid => "UUID",
            IdType::Label => "LABEL",
            IdType::Partuuid => "PARTUUID",
            IdType::Partlabel => "PARTLABEL",
        }
    }

    /// Get the fstab prefix for this identifier type.
    pub fn fstab_prefix(&self) -> &'static str {
        match self {
            IdType::Uuid => "UUID",
            IdType::Label => "LABEL",
            IdType::Partuuid => "PARTUUID",
            IdType::Partlabel => "PARTLABEL",
        }
    }
}

/// Extract the base device path from a source string.
///
/// Handles btrfs subvolume notation like `/dev/sda1[/subvol]`.
/// Returns empty string if source is empty or starts with '['.
pub fn extract_device_path(source: &str) -> &str {
    if source.is_empty() {
        return "";
    }
    if let Some(bracket_pos) = source.find('[') {
        // Handle edge case where source starts with '['
        if bracket_pos == 0 {
            return "";
        }
        &source[..bracket_pos]
    } else {
        source
    }
}

/// Get the device identifier (UUID/LABEL/PARTUUID/PARTLABEL) for a source device.
///
/// Falls back to the device path if identifier lookup fails.
/// Preserves existing identifiers (UUID=, LABEL=, PARTUUID=, PARTLABEL=).
///
/// # Arguments
/// * `source` - The device source string (e.g., "/dev/sda1", "/dev/sda1[/subvol]")
/// * `id_type` - The identifier type to use ("UUID", "LABEL", "PARTUUID", "PARTLABEL")
pub fn get_device_identifier(source: &str, id_type: &str) -> String {
    // Handle empty source gracefully
    if source.is_empty() {
        return "none".to_string();
    }

    // Already has an identifier - preserve it
    if source.starts_with("UUID=")
        || source.starts_with("LABEL=")
        || source.starts_with("PARTUUID=")
        || source.starts_with("PARTLABEL=")
    {
        return source.to_string();
    }

    let device = extract_device_path(source);

    // Handle empty device after extraction
    if device.is_empty() {
        return source.to_string();
    }

    // Look up identifier for block devices
    if device.starts_with("/dev/") {
        if let Some(id) = lookup_device_id(device, id_type) {
            return id;
        }
        // Fall back to device path if no identifier found
        return device.to_string();
    }

    // For other sources (bind mounts, network mounts), use as-is
    source.to_string()
}

/// Look up an identifier for a device using blkid.
///
/// # Arguments
/// * `device` - The device path (e.g., "/dev/sda1")
/// * `tag` - The blkid tag to look up ("UUID", "LABEL", "PARTUUID", "PARTLABEL")
///
/// Returns None silently on failure - Unix philosophy: avoid unnecessary output.
/// The caller handles fallback to device path.
pub fn lookup_device_id(device: &str, tag: &str) -> Option<String> {
    let output = Command::new("blkid")
        .args(["-s", tag, "-o", "value", device])
        .output()
        .ok()?; // Silent failure - falls back to device path

    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !value.is_empty() {
            return Some(format!("{}={}", tag, value));
        }
    }

    None
}
