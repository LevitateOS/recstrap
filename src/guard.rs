//! Safety checks the install tools run before touching a target.

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
