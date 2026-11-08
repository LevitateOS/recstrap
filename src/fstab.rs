//! Generate fstab entries describing the system mounted under a root.
//!
//! An entry describes storage the target owns: a filesystem under the root
//! backed by a block device or a network source, a swap file under the root,
//! or a swap device on one of the target's disks. Pseudo filesystems, tmpfs,
//! FUSE daemons and the host's own swap are not the target's, so they are
//! left out. Paths are always written relative to the root.

use crate::error::{Error, Result};
use crate::output::{ExistingEntry, FstabEntry, FstabResult, FSTAB_SCHEMA_VERSION};
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
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
    entries_for(&crate::session::resolve_target(root_arg)?, id_type)
}

/// [`generate`] for a resolved (canonical) root.
fn entries_for(root: &Path, id_type: IdType) -> Result<Vec<Entry>> {
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

/// What `--write` does with the target's fstab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Write {
    /// Only report.
    No,
    /// Append the entries; refuse if the fstab already describes one of their mount points.
    Append,
    /// Drop the lines that describe the same mount points (and the comment line recfstab
    /// wrote above each), then append.
    Replace,
}

/// The entries, what `<root>/etc/fstab` holds now, and what to look at before booting;
/// with `write`, the entries appended to that fstab.
pub fn report(root_arg: &str, id_type: IdType, write: Write) -> Result<FstabResult> {
    let root = crate::session::resolve_target(root_arg)?;
    let root_str = root.to_string_lossy().into_owned();
    let entries = entries_for(&root, id_type)?;
    let fstab_path = root.join("etc/fstab");
    let existing = read_existing(&fstab_path)?;
    let existing: Vec<ExistingEntry> = existing
        .into_iter()
        .map(|line| ExistingEntry {
            conflicts: entries.iter().any(|entry| describes_same(entry, &line)),
            ..line
        })
        .collect();

    let mut warnings = Vec::new();
    if !entries.iter().any(|e| e.file == "/") {
        warnings.push(format!(
            "nothing is mounted at {root_str} itself, so the target's / gets no entry"
        ));
    }
    if !entries
        .iter()
        .any(|e| e.fstype == "vfat" && matches!(e.file.as_str(), "/boot" | "/efi" | "/boot/efi"))
    {
        warnings.push(format!(
            "no EFI system partition (vfat) is mounted at {root_str}/boot or {root_str}/efi; \
             the boot step installs systemd-boot there"
        ));
    }
    if !root.join("etc").is_dir() {
        warnings.push(format!("{root_str}/etc doesn't exist yet: run recstrap {root_str} first"));
    }

    let written = match write {
        Write::No => false,
        Write::Append | Write::Replace => {
            let conflicts: Vec<String> = existing
                .iter()
                .filter(|line| line.conflicts)
                .map(|line| if line.file == "none" { line.spec.clone() } else { line.file.clone() })
                .collect();
            if write == Write::Append && !conflicts.is_empty() {
                return Err(Error::fstab_conflict(&fstab_path, &conflicts));
            }
            let dropped: BTreeSet<u32> =
                existing.iter().filter(|line| line.conflicts).map(|line| line.line).collect();
            write_fstab(&fstab_path, &dropped, &entries)?;
            true
        }
    };

    Ok(FstabResult {
        schema_version: FSTAB_SCHEMA_VERSION,
        root: root_str,
        fstab_path: fstab_path.to_string_lossy().into_owned(),
        entries: entries
            .iter()
            .map(|e| FstabEntry {
                origin: e.origin.clone(),
                spec: e.spec.clone(),
                file: e.file.clone(),
                fstype: e.fstype.clone(),
                options: e.options.clone(),
                pass: e.pass,
                text: e.to_string(),
            })
            .collect(),
        existing,
        warnings,
        written,
    })
}

/// An existing line describes the entry's mount point, or for swap, its device or file.
fn describes_same(entry: &Entry, line: &ExistingEntry) -> bool {
    if entry.file == "none" {
        line.fstype == "swap" && line.spec == entry.spec
    } else {
        line.file == entry.file
    }
}

/// The fstab's active lines, unescaped; a missing fstab has none.
fn read_existing(path: &Path) -> Result<Vec<ExistingEntry>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::path_unresolvable(&path.to_string_lossy(), e)),
    };
    Ok(text
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let line_text = line.trim();
            if line_text.is_empty() || line_text.starts_with('#') {
                return None;
            }
            let mut fields = line_text
                .split_whitespace()
                .map(|field| String::from_utf8_lossy(&target_root::unescape_octal(field)).into_owned());
            Some(ExistingEntry {
                line: i as u32 + 1,
                spec: fields.next()?,
                file: fields.next().unwrap_or_default(),
                fstype: fields.next().unwrap_or_default(),
                conflicts: false,
            })
        })
        .collect())
}

/// Rewrites the fstab without the lines in `dropped` (1-based, each taking the
/// `# origin` comment line right above it), then with `entries` appended.
fn write_fstab(path: &Path, dropped: &BTreeSet<u32>, entries: &[Entry]) -> Result<()> {
    let current = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => return Err(Error::path_unresolvable(&path.to_string_lossy(), e)),
    };
    let lines: Vec<&str> = current.lines().collect();
    let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        let number = i as u32 + 1;
        if dropped.contains(&number) {
            continue;
        }
        let comment_of_dropped = line.starts_with("# ") && dropped.contains(&(number + 1));
        if !comment_of_dropped {
            kept.push(line);
        }
    }
    let mut text = kept.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    for entry in entries {
        text.push_str(&entry.to_string());
    }
    // An existing fstab keeps its mode; a new one gets the usual 0644.
    let mode = match fs::metadata(path) {
        Ok(meta) => std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o7777,
        Err(e) if e.kind() == ErrorKind::NotFound => 0o644,
        Err(e) => return Err(Error::path_unresolvable(&path.to_string_lossy(), e)),
    };
    target_root::write_durable(path, text.as_bytes(), Some(mode)).map_err(|e| Error::create_failed(path, e))
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
    Ok(target_root::mount_table()?
        .into_iter()
        .map(|mount| Mount {
            target: mount.mount_point.to_string_lossy().into_owned(),
            source: mount.source,
            fstype: mount.fstype,
            options: mount.options,
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
    let value = match id_type {
        IdType::Partuuid => target_root::partuuid(Path::new(device)),
        _ => target_root::blkid_tag(Path::new(device), tag),
    }
    .map_err(|e| Error::identifier_unavailable(device, tag).fix(e.to_string()))?
    .ok_or_else(|| Error::identifier_unavailable(device, tag))?;
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
