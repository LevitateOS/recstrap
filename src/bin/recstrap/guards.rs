//! recstrap's pre-flight checks, in the order they run.
//!
//! Each guard is one entry in `GUARDS`: the check and the override (if any)
//! that waives it. A check that cannot be completed (an I/O error) fails, so a
//! guard never passes by accident. The notes on each check say what it
//! protects and how it could be weakened; they are for maintainers and are not
//! printed.

use recinit::live::ROOTFS_SEARCH_PATHS;
use install_tools::guard::{is_protected_path, is_root};
use install_tools::error::{Error, Result};
use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The rootfs image format recstrap extracts.
const ROOTFS_EXTENSION: &str = "erofs";
const EROFS_MAGIC: u32 = 0xe0f5_e1e2;
const EROFS_MAGIC_OFFSET: u64 = 1024;

/// A rootfs expands to roughly this much; checked up front so a full disk
/// fails before extraction starts rather than half-way through. The exact
/// size is unknown until the image is mounted.
const MIN_FREE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiver {
    NonMountpoint,
    Nonempty,
    HostRootFs,
}

/// What `--force` waives. The host-root-fs guard is deliberately not here: it
/// must be asked for by name, or `--force /home/user` would copy a rootfs over
/// live data.
pub const FORCE_WAIVES: &[Waiver] = &[Waiver::NonMountpoint, Waiver::Nonempty];

pub struct Inputs {
    pub target: PathBuf,
    /// The canonical rootfs image, if one was found.
    rootfs: Option<PathBuf>,
    /// Where the rootfs was looked for, for the not-found message.
    pub rootfs_tried: Vec<String>,
    pub waived: Vec<Waiver>,
}

impl Inputs {
    pub fn new(target: PathBuf, rootfs_arg: Option<&str>, waived: Vec<Waiver>) -> Self {
        let candidates: Vec<String> = match rootfs_arg {
            Some(path) => vec![path.to_string()],
            None => ROOTFS_SEARCH_PATHS.iter().map(|p| p.to_string()).collect(),
        };
        let rootfs = candidates
            .iter()
            .find_map(|candidate| Path::new(candidate).canonicalize().ok());
        Self {
            target,
            rootfs,
            rootfs_tried: candidates,
            waived,
        }
    }

    fn target_str(&self) -> String {
        self.target.to_string_lossy().into_owned()
    }

    fn rootfs(&self) -> Result<&Path> {
        let tried: Vec<&str> = self.rootfs_tried.iter().map(String::as_str).collect();
        self.rootfs
            .as_deref()
            .ok_or_else(|| Error::rootfs_not_found(&tried))
    }
}

pub struct Guard {
    pub check: fn(&Inputs) -> Result<()>,
    pub waiver: Option<Waiver>,
}

pub const GUARDS: &[Guard] = &[
    Guard { check: not_protected, waiver: None },
    Guard { check: running_as_root, waiver: None },
    Guard { check: writable, waiver: None },
    Guard { check: not_on_host_root_fs, waiver: Some(Waiver::HostRootFs) },
    Guard { check: mount_point, waiver: Some(Waiver::NonMountpoint) },
    Guard { check: empty, waiver: Some(Waiver::Nonempty) },
    Guard { check: enough_space, waiver: None },
    Guard { check: rootfs_found, waiver: None },
    Guard { check: rootfs_is_image, waiver: None },
    Guard { check: rootfs_outside_target, waiver: None },
    Guard { check: rootfs_is_erofs, waiver: None },
    Guard { check: kernel_has_erofs, waiver: None },
];

/// Run every guard in order; the first failure stops the run.
/// Run every check that is not waived; returns the rootfs image they found.
pub fn run_all(inputs: &Inputs) -> Result<&Path> {
    for guard in GUARDS {
        if guard.waiver.is_some_and(|w| inputs.waived.contains(&w)) {
            continue;
        }
        (guard.check)(inputs)?;
    }
    inputs.rootfs()
}

/// Protects critical system directories from being overwritten. Never
/// waivable; checked on the canonical path so a symlink cannot bypass it.
/// Weakened by: shrinking the protected list, adding an override, or checking
/// before canonicalisation. Consequence: `/` or `/usr` overwritten.
fn not_protected(inputs: &Inputs) -> Result<()> {
    if is_protected_path(&inputs.target) {
        return Err(Error::protected_path(&inputs.target_str()));
    }
    Ok(())
}

/// Protects against starting an extraction that fails on the first file.
/// Weakened by: skipping it or assuming sudo. Consequence: permission errors
/// mid-extraction.
fn running_as_root(_: &Inputs) -> Result<()> {
    if !is_root() {
        return Err(Error::not_root());
    }
    Ok(())
}

/// Protects against starting on a read-only or unwritable target (access(2)
/// also reports read-only filesystems to root). Weakened by: assuming root can
/// write anywhere. Consequence: a partial, corrupt extraction.
fn writable(inputs: &Inputs) -> Result<()> {
    let path = std::ffi::CString::new(inputs.target.as_os_str().as_bytes())
        .map_err(|_| Error::not_writable(&inputs.target_str()))?;
    // SAFETY: `path` is a valid NUL-terminated C string.
    if unsafe { libc::access(path.as_ptr(), libc::W_OK) } != 0 {
        return Err(Error::not_writable(&inputs.target_str()));
    }
    Ok(())
}

/// Protects the running system's root filesystem from `--force`. Compares
/// device IDs, not paths. Weakened by: letting `--force` imply the waiver.
/// Consequence: a rootfs copied over live host directories.
fn not_on_host_root_fs(inputs: &Inputs) -> Result<()> {
    let target = std::fs::metadata(&inputs.target)
        .map_err(|e| Error::path_unresolvable(&inputs.target_str(), e))?;
    let root = std::fs::metadata("/").map_err(|e| Error::path_unresolvable("/", e))?;
    if target.dev() == root.dev() {
        return Err(Error::on_host_root_filesystem(&inputs.target_str()));
    }
    Ok(())
}

/// Protects against installing onto whatever filesystem happens to hold the
/// directory because the user forgot to mount. Consequence: the wrong disk
/// fills up.
fn mount_point(inputs: &Inputs) -> Result<()> {
    let target = std::fs::metadata(&inputs.target)
        .map_err(|e| Error::path_unresolvable(&inputs.target_str(), e))?;
    let parent_path = inputs.target.parent().unwrap_or(Path::new("/"));
    let parent = std::fs::metadata(parent_path)
        .map_err(|e| Error::path_unresolvable(&parent_path.to_string_lossy(), e))?;
    // A mount point is on a different device than its parent, or is the
    // root of its own filesystem (same inode as the parent: `/`).
    if target.dev() == parent.dev() && target.ino() != parent.ino() {
        return Err(Error::not_mount_point(&inputs.target_str()));
    }
    Ok(())
}

/// Protects existing data from being silently overwritten. `lost+found` is
/// ignored: a freshly formatted ext4 filesystem always has it. Weakened by:
/// ignoring hidden files. Consequence: unrecoverable data loss.
fn empty(inputs: &Inputs) -> Result<()> {
    let entries = std::fs::read_dir(&inputs.target)
        .map_err(|e| Error::path_unresolvable(&inputs.target_str(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::path_unresolvable(&inputs.target_str(), e))?;
        if entry.file_name() != "lost+found" {
            return Err(Error::target_not_empty(&inputs.target_str()));
        }
    }
    Ok(())
}

/// Protects against running out of space half-way through. Weakened by:
/// lowering the floor or only warning. Consequence: a corrupt partial system.
fn enough_space(inputs: &Inputs) -> Result<()> {
    let path = std::ffi::CString::new(inputs.target.as_os_str().as_bytes())
        .map_err(|_| Error::not_writable(&inputs.target_str()))?;
    // SAFETY: zeroed statvfs is a valid out-parameter; `path` is NUL-terminated.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return Err(Error::path_unresolvable(
            &inputs.target_str(),
            std::io::Error::last_os_error(),
        ));
    }
    #[allow(clippy::unnecessary_cast)] // field types vary by platform
    let available = stat.f_bavail as u64 * stat.f_frsize as u64;
    if available < MIN_FREE_BYTES {
        return Err(Error::insufficient_space(
            MIN_FREE_BYTES / (1024 * 1024),
            available / (1024 * 1024),
        ));
    }
    Ok(())
}

/// Protects against guessing: the rootfs is the explicit `--rootfs`, or the
/// first live-ISO location that exists. Weakened by: hard-coding a path.
fn rootfs_found(inputs: &Inputs) -> Result<()> {
    inputs.rootfs().map(|_| ())
}

/// Protects against a directory being treated as the image. The live rootfs
/// handle may resolve to the payload partition, so a block device counts.
fn rootfs_is_image(inputs: &Inputs) -> Result<()> {
    use std::os::unix::fs::FileTypeExt;
    let rootfs = inputs.rootfs()?;
    let is_image = std::fs::metadata(rootfs)
        .map(|m| m.is_file() || m.file_type().is_block_device())
        .unwrap_or(false);
    if !is_image {
        return Err(Error::rootfs_not_file(&rootfs.to_string_lossy()));
    }
    Ok(())
}

/// Protects against extracting the image over itself. Checked on canonical
/// paths. Consequence: the source is overwritten mid-extraction.
fn rootfs_outside_target(inputs: &Inputs) -> Result<()> {
    let rootfs = inputs.rootfs()?;
    if rootfs.starts_with(&inputs.target) {
        return Err(Error::rootfs_inside_target(
            &rootfs.to_string_lossy(),
            &inputs.target_str(),
        ));
    }
    Ok(())
}

/// Protects against mounting a corrupt or wrong-format image: an image file
/// must be named `.erofs`, and file or partition must be readable and carry
/// the EROFS superblock magic.
fn rootfs_is_erofs(inputs: &Inputs) -> Result<()> {
    let rootfs = inputs.rootfs()?;
    let shown = rootfs.to_string_lossy();
    if rootfs.is_file() && rootfs.extension().and_then(|e| e.to_str()) != Some(ROOTFS_EXTENSION) {
        return Err(Error::invalid_rootfs_format(
            &shown,
            &format!("expected a .{ROOTFS_EXTENSION} image"),
        ));
    }
    let read_magic = || -> std::io::Result<u32> {
        let mut file = File::open(rootfs)?;
        file.seek(SeekFrom::Start(EROFS_MAGIC_OFFSET))?;
        let mut buf = [0u8; 4];
        file.read_exact(&mut buf)?;
        Ok(u32::from_le_bytes(buf))
    };
    match read_magic() {
        Ok(EROFS_MAGIC) => Ok(()),
        Ok(magic) => Err(Error::invalid_rootfs_format(
            &shown,
            &format!("not a valid EROFS image (magic: 0x{magic:08x}, expected: 0x{EROFS_MAGIC:08x})"),
        )),
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {
            Err(Error::rootfs_not_readable(&shown))
        }
        Err(e) => Err(Error::invalid_rootfs_format(&shown, &e.to_string())),
    }
}

/// Protects against a cryptic "unknown filesystem type" at mount time. EROFS
/// counts as available when the kernel has it or can load it (`modprobe -n`
/// changes nothing; the mount itself loads the module).
fn kernel_has_erofs(_: &Inputs) -> Result<()> {
    let built_in = std::fs::read_to_string("/proc/filesystems")
        .map(|list| list.lines().any(|line| line.split_whitespace().last() == Some("erofs")))
        .unwrap_or(false);
    let loadable = || {
        Command::new("modprobe")
            .args(["-n", "-q", "erofs"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    if built_in || loadable() {
        Ok(())
    } else {
        Err(Error::erofs_not_supported())
    }
}

/// Directories every extracted system must have; a missing one means the image
/// was corrupt or incomplete.
const ESSENTIAL_DIRS: &[&str] = &["bin", "etc", "lib", "sbin", "usr", "var"];

/// Protects against reporting success for an incomplete system.
pub fn verify_extraction(target: &Path) -> Result<()> {
    let missing: Vec<&str> = ESSENTIAL_DIRS
        .iter()
        .filter(|dir| !target.join(dir).is_dir())
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(Error::extraction_verification_failed(&missing));
    }
    Ok(())
}
