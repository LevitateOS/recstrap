//! recstrap's pre-flight checks, in the order they run.
//!
//! Each guard is one entry in `GUARDS`: the check and the override (if any)
//! that waives it. A check that cannot be completed (an I/O error) fails, so a
//! guard never passes by accident. The notes on each check say what it
//! protects and how it could be weakened; they are for maintainers and are not
//! printed.

use recinit::live::ROOTFS_SEARCH_PATHS;
use install_tools::guard::host_system_filesystem_holding;
use target_root::{is_protected_path, is_root};
use install_tools::error::{Error, Result};
use std::io::ErrorKind;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

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
    /// The canonical rootfs image, if one was found, or why looking failed.
    rootfs: std::result::Result<Option<PathBuf>, (String, std::io::Error)>,
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
        // The first candidate that exists; one that cannot be looked at stops
        // the search with its error.
        let rootfs = candidates
            .iter()
            .find_map(|candidate| match Path::new(candidate).canonicalize() {
                Ok(path) => Some(Ok(Some(path))),
                Err(e) if e.kind() == ErrorKind::NotFound => None,
                Err(e) => Some(Err((candidate.clone(), e))),
            })
            .unwrap_or(Ok(None));
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
        match &self.rootfs {
            Ok(Some(path)) => Ok(path),
            Ok(None) => Err(Error::rootfs_not_found(&tried)),
            Err((candidate, e)) => Err(Error::path_unresolvable(candidate, std::io::Error::new(e.kind(), e.to_string()))),
        }
    }
}

pub struct Guard {
    pub check: fn(&Inputs) -> Result<()>,
    pub waiver: Option<Waiver>,
}

pub const GUARDS: &[Guard] = &[
    Guard { check: not_protected, waiver: None },
    Guard { check: not_on_host_root_fs, waiver: Some(Waiver::HostRootFs) },
    Guard { check: running_as_root, waiver: None },
    Guard { check: writable, waiver: None },
    Guard { check: mount_point, waiver: Some(Waiver::NonMountpoint) },
    Guard { check: empty, waiver: Some(Waiver::Nonempty) },
    Guard { check: enough_space, waiver: None },
    Guard { check: rootfs_found, waiver: None },
    Guard { check: rootfs_is_image, waiver: None },
    Guard { check: rootfs_outside_target, waiver: None },
    Guard { check: rootfs_is_erofs, waiver: None },
    Guard { check: kernel_has_erofs, waiver: None },
];

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

/// Protects the running system's filesystems from `--force`: the target must
/// not be on the filesystem mounted at `/` or at any other host system
/// directory (`/home`, `/usr`, `/var`, `/boot`, ...). Decided by the mount
/// table's device numbers, not `st_dev`, which differs per btrfs subvolume
/// (Fedora's `/home`) and so let `--force ~/x` through. Needs no privilege,
/// so it runs before the root check. Weakened by: letting `--force` imply the
/// waiver, or comparing `st_dev` with `/` only. Consequence: a rootfs copied
/// over live host directories.
fn not_on_host_root_fs(inputs: &Inputs) -> Result<()> {
    if let Some(system_dir) = host_system_filesystem_holding(&inputs.target)? {
        return Err(Error::on_host_root_filesystem(&inputs.target_str(), system_dir));
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
/// ignored: a freshly formatted ext4 filesystem always has it; so are empty
/// filesystems mounted inside the target (the layout's /boot, /var). Weakened
/// by: ignoring hidden files, or a mount that holds files. Consequence: unrecoverable data loss.
fn empty(inputs: &Inputs) -> Result<()> {
    if holds_files(&inputs.target)? {
        return Err(Error::target_not_empty(&inputs.target_str()));
    }
    Ok(())
}

/// Whether `dir` holds anything but `lost+found` and empty filesystems mounted
/// on it: the layout recpart (or the guide) mounts at /mnt, /mnt/boot and
/// /mnt/var before extraction counts as empty.
fn holds_files(dir: &Path) -> Result<bool> {
    let unresolvable = |e| Error::path_unresolvable(&dir.to_string_lossy(), e);
    let device = std::fs::metadata(dir).map_err(unresolvable)?.dev();
    for entry in std::fs::read_dir(dir).map_err(unresolvable)? {
        let entry = entry.map_err(unresolvable)?;
        if entry.file_name() == "lost+found" {
            continue;
        }
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(unresolvable)?;
        let mounted_here = meta.is_dir() && meta.dev() != device;
        if !mounted_here || holds_files(&path)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Protects against running out of space half-way through. Weakened by:
/// lowering the floor or only warning. Consequence: a corrupt partial system.
fn enough_space(inputs: &Inputs) -> Result<()> {
    let available = free_bytes(&inputs.target)?;
    if available < MIN_FREE_BYTES {
        return Err(Error::insufficient_space(
            MIN_FREE_BYTES / (1024 * 1024),
            available / (1024 * 1024),
        ));
    }
    Ok(())
}

/// Space an unprivileged write could use on the filesystem holding `target`.
pub fn free_bytes(target: &Path) -> Result<u64> {
    let target_str = target.to_string_lossy();
    let path = std::ffi::CString::new(target.as_os_str().as_bytes())
        .map_err(|_| Error::not_writable(&target_str))?;
    // SAFETY: zeroed statvfs is a valid out-parameter; `path` is NUL-terminated.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return Err(Error::path_unresolvable(&target_str, std::io::Error::last_os_error()));
    }
    #[allow(clippy::unnecessary_cast)] // field types vary by platform
    Ok(stat.f_bavail as u64 * stat.f_frsize as u64)
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
    let meta = std::fs::metadata(rootfs)
        .map_err(|e| Error::path_unresolvable(&rootfs.to_string_lossy(), e))?;
    if !meta.is_file() && !meta.file_type().is_block_device() {
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
/// an EROFS superblock (the rule recab's stage applies too).
fn rootfs_is_erofs(inputs: &Inputs) -> Result<()> {
    let rootfs = inputs.rootfs()?;
    let shown = rootfs.to_string_lossy();
    match target_root::erofs_image(rootfs) {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(why)) => Err(Error::invalid_rootfs_format(&shown, &why)),
        Err(e) if e.kind() == ErrorKind::PermissionDenied => Err(Error::rootfs_not_readable(&shown)),
        Err(e) => Err(Error::invalid_rootfs_format(&shown, &e.to_string())),
    }
}

/// Protects against a cryptic "unknown filesystem type" at mount time. The
/// live protocol requires EROFS built into the kernel (the live root is
/// EROFS), so the kernel lists it in /proc/filesystems; nothing is loaded.
fn kernel_has_erofs(_: &Inputs) -> Result<()> {
    let built_in = std::fs::read_to_string("/proc/filesystems")
        .map_err(|e| Error::path_unresolvable("/proc/filesystems", e))?
        .lines()
        .any(|line| line.split_whitespace().last() == Some("erofs"));
    if built_in {
        Ok(())
    } else {
        Err(Error::erofs_not_supported())
    }
}

/// Protects against reporting success for an incomplete system: an extracted
/// system describes itself in its os-release, distro profile included, which
/// recsetup and recab read from it.
pub fn verify_extraction(target: &Path) -> Result<()> {
    let os_release = target_root::os_release(target)?.ok_or_else(|| {
        Error::extraction_verification_failed(format!(
            "'{}' has no etc/os-release or usr/lib/os-release",
            target.display()
        ))
    })?;
    distro_profile::Profile::from_os_release(&os_release).map_err(|e| {
        Error::extraction_verification_failed(format!("'{}': {e}", target.display()))
    })?;
    Ok(())
}
