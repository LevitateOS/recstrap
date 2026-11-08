//! The host-filesystem guard. The other safety checks (protected paths, the
//! root check) and the mount table are [`target_root`]'s, shared with recpart
//! and recab.

use crate::error::{Error, Result};
use target_root::{mount_holding, mount_table, PROTECTED_PATHS};

/// The host system directory (`/` or another of `PROTECTED_PATHS` that
/// exists, such as `/home`, `/usr` or `/boot`) whose filesystem also holds
/// `path`, an existing canonical path. Filesystems are compared by
/// mountinfo's device number, so a separate `/home` partition, a btrfs
/// `/home` subvolume of the root filesystem and a bind mount of any of them
/// are all caught.
pub fn host_system_filesystem_holding(path: &std::path::Path) -> Result<Option<&'static str>> {
    let table = mount_table()?;
    let Some(target) = mount_holding(&table, path)? else {
        return Err(Error::path_unresolvable(
            &path.to_string_lossy(),
            std::io::Error::other("no mount in /proc/self/mountinfo holds it"),
        ));
    };
    for dir in PROTECTED_PATHS {
        let canonical = match std::fs::canonicalize(dir) {
            Ok(canonical) => canonical,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(Error::path_unresolvable(dir, e)),
        };
        if mount_holding(&table, &canonical)?.is_some_and(|system| system.dev == target.dev) {
            return Ok(Some(dir));
        }
    }
    Ok(None)
}
