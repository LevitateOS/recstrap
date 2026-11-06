//! Generate fstab entries describing the system mounted under a root.
//!
//! An entry describes storage the target owns: a filesystem under the root
//! backed by a block device or a network source, a swap file under the root,
//! or a swap device on one of the target's disks. Pseudo filesystems, tmpfs,
//! FUSE daemons and the host's own swap are not the target's, so they are
//! left out. Paths are always written relative to the root.

use crate::error::{Error, Result};
use std::collections::BTreeSet;
use std::fmt;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdType {
    Uuid,
    Label,
    Partuuid,
    Partlabel,
}

impl IdType {
    /// The blkid tag, which is also the fstab `TAG=` prefix.
    fn tag(self) -> &'static str {
        match self {
            IdType::Uuid => "UUID",
            IdType::Label => "LABEL",
            IdType::Partuuid => "PARTUUID",
            IdType::Partlabel => "PARTLABEL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// What was mounted, for the comment line.
    pub origin: String,
    pub spec: String,
    pub file: String,
    pub fstype: String,
    pub options: String,
    pub pass: u8,
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "# {}", self.origin)?;
        writeln!(
            f,
            "{}\t{}\t{}\t{}\t0\t{}",
            escape(&self.spec),
            escape(&self.file),
            self.fstype,
            self.options,
            self.pass
        )
    }
}

/// Entries for everything the target under `root_arg` needs in its fstab.
pub fn generate(root_arg: &str, id_type: IdType) -> Result<Vec<Entry>> {
    let root = crate::session::resolve_target(root_arg)?;
    let root = Root(root.to_string_lossy().into_owned());

    let mut mounts: Vec<Mount> = Vec::new();
    for mount in list_mounts()? {
        if !root.contains(&mount.target) || !mount.is_storage() {
            continue;
        }
        // The last mount on a path is the visible one.
        mounts.retain(|m| m.target != mount.target);
        mounts.push(mount);
    }

    let mut target_disks = BTreeSet::new();
    for mount in &mounts {
        if let Some(device) = mount.device() {
            target_disks.extend(disks_of(device)?);
        }
    }

    let mut entries = Vec::new();
    for mount in &mounts {
        let file = root.relative(&mount.target);
        let pass = if file == "/" {
            1
        } else if boot_fsck_supported(&mount.fstype) {
            2
        } else {
            0
        };
        entries.push(Entry {
            origin: mount.source.clone(),
            spec: match mount.device() {
                Some(device) => identifier(device, id_type)?,
                None => mount.source.clone(),
            },
            file,
            fstype: mount.fstype.clone(),
            options: persistent_options(&mount.options),
            pass,
        });
    }

    for swap in list_swaps()? {
        let entry = if swap.kind == "file" {
            if !root.contains(&swap.name) {
                continue;
            }
            Entry {
                origin: swap.name.clone(),
                spec: root.relative(&swap.name),
                file: "none".into(),
                fstype: "swap".into(),
                options: "defaults".into(),
                pass: 0,
            }
        } else {
            if disks_of(&swap.name)?.is_disjoint(&target_disks) {
                continue;
            }
            Entry {
                origin: swap.name.clone(),
                spec: identifier(&swap.name, id_type)?,
                file: "none".into(),
                fstype: "swap".into(),
                options: "defaults".into(),
                pass: 0,
            }
        };
        entries.push(entry);
    }

    if entries.is_empty() {
        return Err(Error::no_filesystems(&root.0));
    }
    Ok(entries)
}

/// The canonical root, with path membership and re-rooting in one place.
struct Root(String);

impl Root {
    fn contains(&self, path: &str) -> bool {
        self.0 == "/" || path == self.0 || path.starts_with(&format!("{}/", self.0))
    }

    fn relative(&self, path: &str) -> String {
        if self.0 == "/" {
            return path.to_string();
        }
        match path.strip_prefix(&self.0) {
            Some("") => "/".to_string(),
            Some(rest) => rest.to_string(),
            None => path.to_string(),
        }
    }
}

struct Mount {
    target: String,
    source: String,
    fstype: String,
    options: String,
}

impl Mount {
    /// The backing block device, without a btrfs subvolume suffix.
    fn device(&self) -> Option<&str> {
        self.source
            .starts_with("/dev/")
            .then(|| self.source.split('[').next().unwrap_or(&self.source))
    }

    /// Backed by a block device or a network source (`host:/path`, `//host/share`).
    fn is_storage(&self) -> bool {
        self.device().is_some() || self.source.contains(":/") || self.source.starts_with("//")
    }
}

fn list_mounts() -> Result<Vec<Mount>> {
    let rows = util_linux("findmnt", &["-rn", "-o", "TARGET,SOURCE,FSTYPE,OPTIONS"])?;
    Ok(rows
        .iter()
        .filter_map(|fields| match fields.as_slice() {
            [target, source, fstype, options] => Some(Mount {
                target: target.clone(),
                source: source.clone(),
                fstype: fstype.clone(),
                options: options.clone(),
            }),
            _ => None,
        })
        .collect())
}

struct Swap {
    name: String,
    kind: String,
}

fn list_swaps() -> Result<Vec<Swap>> {
    let rows = util_linux("swapon", &["--show=NAME,TYPE", "--raw", "--noheadings"])?;
    Ok(rows
        .into_iter()
        .filter_map(|fields| match <[String; 2]>::try_from(fields) {
            Ok([name, kind]) => Some(Swap { name, kind }),
            Err(_) => None,
        })
        .collect())
}

/// Whole-disk ancestors of a block device (through partitions, LVM, dm-crypt).
fn disks_of(device: &str) -> Result<BTreeSet<String>> {
    let rows = util_linux("lsblk", &["-nrs", "-o", "NAME,TYPE", device])?;
    Ok(rows
        .into_iter()
        .filter_map(|fields| match fields.as_slice() {
            [name, kind] if kind == "disk" || kind == "loop" => Some(name.clone()),
            _ => None,
        })
        .collect())
}

/// Run a util-linux listing in raw mode and split it into unescaped fields.
fn util_linux(tool: &str, args: &[&str]) -> Result<Vec<Vec<String>>> {
    let output = Command::new(tool)
        .args(args)
        .output()
        .map_err(|e| Error::tool_missing(tool, "util-linux", e))?;
    if !output.status.success() {
        return Err(Error::tool_failed(tool, &String::from_utf8_lossy(&output.stderr)));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split(' ').map(unescape_raw).collect())
        .collect())
}

/// util-linux raw output escapes unsafe bytes as `\xNN`.
fn unescape_raw(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && bytes.get(i + 1) == Some(&b'x') {
            if let Some(byte) = field
                .get(i + 2..i + 4)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                out.push(byte);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// fstab(5) splits on whitespace, so those, `\` and a leading-comment `#` are
/// written as octal escapes.
fn escape(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    for c in field.chars() {
        match c {
            ' ' | '\t' | '\n' | '\r' | '\\' | '#' => out.push_str(&format!("\\{:03o}", c as u32)),
            _ => out.push(c),
        }
    }
    out
}

fn identifier(device: &str, id_type: IdType) -> Result<String> {
    let tag = id_type.tag();
    let output = Command::new("blkid")
        .args(["-s", tag, "-o", "value", device])
        .output()
        .map_err(|e| Error::tool_missing("blkid", "util-linux", e))?;
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || value.is_empty() {
        return Err(Error::identifier_unavailable(device, tag));
    }
    Ok(format!("{tag}={value}"))
}

/// Options the kernel adds at mount time, which must not be pinned in fstab:
/// `rw` is the default, `seclabel` is SELinux runtime state, and btrfs
/// `subvolid` changes when a subvolume is recreated (`subvol=` names it).
fn persistent_options(options: &str) -> String {
    let kept: Vec<&str> = options
        .split(',')
        .map(str::trim)
        .filter(|o| !o.is_empty() && *o != "rw" && *o != "seclabel" && !o.starts_with("subvolid="))
        .collect();
    if kept.is_empty() {
        "defaults".to_string()
    } else {
        kept.join(",")
    }
}

/// Filesystems whose fsck is meant to run at boot. vfat (the ESP) and btrfs
/// (`btrfs check` is manual-only) are not.
fn boot_fsck_supported(fstype: &str) -> bool {
    matches!(fstype, "ext2" | "ext3" | "ext4" | "xfs" | "f2fs")
}
