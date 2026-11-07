//! Safety checks the install tools run before touching a target, and the
//! mount table they read.

/// Protected system paths that should never be extraction/chroot targets.
///
/// These are critical system directories that would be destroyed if used
/// as installation targets. Tools like recstrap and recchroot use this list
/// to prevent catastrophic user errors.
pub const PROTECTED_PATHS: &[&str] = &[
    "/",
    "/bin",
    "/boot",
    "/boot/efi",
    "/dev",
    "/efi",
    "/etc",
    "/home",
    "/lib",
    "/lib32",
    "/lib64",
    "/opt",
    "/proc",
    "/root",
    "/run",
    "/sbin",
    "/srv",
    "/sys",
    "/tmp",
    "/usr",
    "/usr/local",
    "/var",
];

/// Check if a path is a protected system path.
///
/// The path is resolved first (symlinks via `canonicalize` when it exists,
/// otherwise `.`/`..` components lexically), so `/mnt/../usr` or a symlink
/// pointing at `/etc` is caught. Only the resolved path itself is compared;
/// children such as `/mnt/usr` are NOT protected.
pub fn is_protected_path(path: &std::path::Path) -> bool {
    let resolved = resolve_for_protection(path);
    PROTECTED_PATHS
        .iter()
        .any(|protected| resolved == std::path::Path::new(protected))
}

fn resolve_for_protection(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::{Component, PathBuf};

    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }

    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push("/");
    }
    out
}

/// Whether this process runs as root.
pub fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

/// One mount, from `/proc/self/mountinfo`.
#[derive(Debug, Clone)]
pub struct Mount {
    pub id: u64,
    /// The filesystem's device number (major, minor). Every mount of one
    /// filesystem has the same number, all subvolumes of a btrfs filesystem
    /// included (unlike `st_dev`, which differs per subvolume).
    pub dev: (u32, u32),
    pub mount_point: std::path::PathBuf,
    /// What was mounted: a device path for block filesystems.
    pub source: String,
}

/// This process's mount table, in mount order (a later mount on the same
/// point covers an earlier one).
pub fn mount_table() -> crate::error::Result<Vec<Mount>> {
    const MOUNTINFO: &str = "/proc/self/mountinfo";
    let text = std::fs::read_to_string(MOUNTINFO)
        .map_err(|e| crate::error::Error::path_unresolvable(MOUNTINFO, e))?;
    Ok(text.lines().filter_map(parse_mountinfo_line).collect())
}

/// `id parent major:minor root mount-point options [optional...] - fstype source super-options`
fn parse_mountinfo_line(line: &str) -> Option<Mount> {
    use std::os::unix::ffi::OsStringExt;

    let fields: Vec<&str> = line.split(' ').collect();
    let id = fields.first()?.parse().ok()?;
    let (major, minor) = fields.get(2)?.split_once(':')?;
    let separator = fields.iter().skip(6).position(|f| *f == "-")? + 6;
    Some(Mount {
        id,
        dev: (major.parse().ok()?, minor.parse().ok()?),
        mount_point: std::ffi::OsString::from_vec(unescape_mountinfo(fields.get(4)?)).into(),
        source: String::from_utf8_lossy(&unescape_mountinfo(fields.get(separator + 2)?))
            .into_owned(),
    })
}

/// mountinfo writes space, tab, newline and backslash as `\NNN` (octal).
fn unescape_mountinfo(field: &str) -> Vec<u8> {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = bytes.get(i + 1..i + 4).filter(|d| {
            bytes[i] == b'\\' && d.iter().all(|c| (b'0'..=b'7').contains(c))
        });
        match octal {
            Some(d) => {
                out.push(d.iter().fold(0u8, |n, c| n.wrapping_mul(8).wrapping_add(c - b'0')));
                i += 4;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    out
}

/// The mount that holds `path` (an existing, canonical path): the one
/// statx(2) names, or, on kernels without `STATX_MNT_ID`, the topmost mount
/// with the longest mount-point prefix.
pub fn mount_holding<'a>(table: &'a [Mount], path: &std::path::Path) -> Option<&'a Mount> {
    if let Some(id) = mount_id(path) {
        if let Some(mount) = table.iter().find(|m| m.id == id) {
            return Some(mount);
        }
    }
    table
        .iter()
        .filter(|m| path.starts_with(&m.mount_point))
        .max_by_key(|m| m.mount_point.components().count())
}

fn mount_id(path: &std::path::Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;

    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: zeroed statx is a valid out-parameter; `c_path` is NUL-terminated.
    let mut buf: libc::statx = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::statx(libc::AT_FDCWD, c_path.as_ptr(), 0, libc::STATX_MNT_ID, &mut buf)
    };
    (rc == 0 && buf.stx_mask & libc::STATX_MNT_ID != 0).then_some(buf.stx_mnt_id)
}

/// The host system directory (`/` or another of `PROTECTED_PATHS` that
/// exists, such as `/home`, `/usr` or `/boot`) whose filesystem also holds
/// `path`, an existing canonical path. Filesystems are compared by
/// mountinfo's device number, so a separate `/home` partition, a btrfs
/// `/home` subvolume of the root filesystem and a bind mount of any of them
/// are all caught.
pub fn host_system_filesystem_holding(
    path: &std::path::Path,
) -> crate::error::Result<Option<&'static str>> {
    let table = mount_table()?;
    let Some(target) = mount_holding(&table, path) else {
        return Err(crate::error::Error::path_unresolvable(
            &path.to_string_lossy(),
            std::io::Error::other("no mount in /proc/self/mountinfo holds it"),
        ));
    };
    Ok(PROTECTED_PATHS.iter().copied().find(|dir| {
        std::fs::canonicalize(dir)
            .ok()
            .and_then(|dir| mount_holding(&table, &dir))
            .is_some_and(|system| system.dev == target.dev)
    }))
}
