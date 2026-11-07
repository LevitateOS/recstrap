//! The JSON the install tools print with `--json`: recstrap's progress events,
//! recfstab's entries, and the one error shape every tool writes to stderr.
//! The TUI apps read it through the TypeScript `cargo xtask gen` writes from
//! these types (feature `contracts`).

use crate::error::Error;
use serde::Serialize;

pub const ERROR_SCHEMA_VERSION: u32 = 1;
pub const STRAP_SCHEMA_VERSION: u32 = 1;
pub const FSTAB_SCHEMA_VERSION: u32 = 1;

/// What a failed run prints to stderr with `--json`.
#[cfg_attr(feature = "contracts", derive(schemars::JsonSchema, ts_rs::TS))]
#[derive(Debug, Clone, Serialize)]
pub struct ErrorPayload {
    pub schema_version: u32,
    /// `E0NN`; its number is the exit status.
    pub code: String,
    pub message: String,
    /// What to do about it, when the message alone does not say.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "contracts", ts(optional))]
    pub remediation: Option<String>,
}

impl ErrorPayload {
    pub fn of(err: &Error) -> Self {
        Self {
            schema_version: ERROR_SCHEMA_VERSION,
            code: err.code.to_string(),
            message: err.message.clone(),
            remediation: err.remediation.clone(),
        }
    }
}

pub fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("install tool output types serialize to JSON")
}

/// One event per line, for output read as it comes.
pub fn to_json_line<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("install tool output types serialize to JSON")
}

/// One line of `recstrap --json`, in order: `start`, `progress` (repeated),
/// `verify`, `done`. `recstrap --check --json` prints one `checked` instead.
#[cfg_attr(feature = "contracts", derive(schemars::JsonSchema, ts_rs::TS))]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum StrapEvent {
    /// Every check passed; nothing was copied (`--check`).
    Checked {
        schema_version: u32,
        target: String,
        rootfs: String,
        /// The image file's size.
        rootfs_bytes: u64,
        /// Free space on the target's filesystem.
        free_bytes: u64,
        /// How many checks ran (waived ones are not counted).
        checks: u32,
    },
    /// The copy begins: the image is mounted and its entries counted.
    Start {
        schema_version: u32,
        target: String,
        rootfs: String,
        /// Files, directories and links in the image, below its top directory.
        total_entries: u64,
        /// The size of its regular files.
        total_bytes: u64,
    },
    /// Copied so far: entries counted from the copy, bytes from the target's
    /// filesystem (allocated space, so it can run past `total_bytes`).
    Progress { entries: u64, bytes: u64 },
    /// The copy finished; the extracted system is being checked.
    Verify,
    Done {
        schema_version: u32,
        entries: u64,
        bytes: u64,
        duration_ms: u64,
    },
}

/// One line `recfstab` writes, and the mount it describes.
#[cfg_attr(feature = "contracts", derive(schemars::JsonSchema, ts_rs::TS))]
#[derive(Debug, Clone, Serialize)]
pub struct FstabEntry {
    /// What is mounted (the device, or the swap file's path on the live system).
    pub origin: String,
    pub spec: String,
    /// The mount point, relative to the root; `none` for swap.
    pub file: String,
    pub fstype: String,
    pub options: String,
    pub pass: u8,
    /// The entry as written, its comment line included.
    pub text: String,
}

/// An active line already in the target's fstab.
#[cfg_attr(feature = "contracts", derive(schemars::JsonSchema, ts_rs::TS))]
#[derive(Debug, Clone, Serialize)]
pub struct ExistingEntry {
    /// 1-based.
    pub line: u32,
    pub spec: String,
    pub file: String,
    pub fstype: String,
    /// Describes a mount point (or swap) that a generated entry also describes.
    pub conflicts: bool,
}

/// `recfstab --json`: the entries for what is mounted under the root, what the
/// target's fstab holds now, and (with `--write`) whether they were written.
#[cfg_attr(feature = "contracts", derive(schemars::JsonSchema, ts_rs::TS))]
#[derive(Debug, Clone, Serialize)]
pub struct FstabResult {
    pub schema_version: u32,
    pub root: String,
    /// `<root>/etc/fstab`.
    pub fstab_path: String,
    pub entries: Vec<FstabEntry>,
    /// The fstab's active lines; empty when it has none or does not exist.
    pub existing: Vec<ExistingEntry>,
    /// Things to look at before booting the target; none stops `--write`.
    pub warnings: Vec<String>,
    /// `--write` appended the entries (replacing conflicting lines with `--replace`).
    pub written: bool,
}
