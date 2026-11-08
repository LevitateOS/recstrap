//! What every install tool reads the same way from an extracted system: its
//! os-release (with the distro profile) and root's login shell.

use std::fs;
use std::path::{Path, PathBuf};

/// The target's os-release, `etc/os-release` or else `usr/lib/os-release`. An
/// absolute symlink is followed inside `root`, never onto the host.
pub fn os_release(root: &Path) -> Option<String> {
    ["etc/os-release", "usr/lib/os-release"].iter().find_map(|rel| {
        let mut path = root.join(rel);
        for _ in 0..8 {
            match fs::read_link(&path) {
                Ok(link) if link.is_absolute() => {
                    path = root.join(link.strip_prefix("/").unwrap_or(&link))
                }
                Ok(link) => path = path.parent().map(|p| p.join(&link)).unwrap_or(PathBuf::from(link)),
                Err(_) => break,
            }
        }
        fs::read_to_string(path).ok()
    })
}

/// Root's login shell in the text of a `passwd` file, so every chroot and every
/// new user gets the shell the system itself gives root. Falls back to `/bin/sh`.
pub fn root_shell(passwd: &str) -> String {
    passwd
        .lines()
        .find(|line| line.starts_with("root:"))
        .and_then(|line| line.split(':').nth(6))
        .filter(|shell| !shell.is_empty())
        .unwrap_or("/bin/sh")
        .to_string()
}
