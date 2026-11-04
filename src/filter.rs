//! Filtering logic for mount entries and options.

/// Pseudo-filesystems and special mounts that should be excluded from fstab.
pub const PSEUDO_FILESYSTEMS: &[&str] = &[
    "autofs",
    "binder",
    "binfmt_misc",
    "bpf",
    "cgroup",
    "cgroup2",
    "configfs",
    "debugfs",
    "devpts",
    "devtmpfs",
    "efivarfs",
    "fuse.gvfsd-fuse",
    "fuse.portal",
    "fusectl",
    "hugetlbfs",
    "mqueue",
    "nsfs",
    "overlay",
    "proc",
    "pstore",
    "ramfs",
    "rpc_pipefs",
    "securityfs",
    "selinuxfs",
    "sysfs",
    "tmpfs",
    "tracefs",
];

/// Mount options that are runtime-only and should not appear in fstab.
pub const RUNTIME_OPTIONS: &[&str] = &["lazytime", "noatime", "relatime", "ro", "rw", "seclabel"];

/// Check if a filesystem type is a pseudo-filesystem that should be excluded.
pub fn is_pseudo_filesystem(fstype: &str) -> bool {
    PSEUDO_FILESYSTEMS.contains(&fstype)
}

/// Filter out runtime-only mount options that shouldn't be in fstab.
///
/// Handles edge cases like leading/trailing commas, empty options, and whitespace.
pub fn filter_options(options: &str) -> String {
    let filtered: Vec<&str> = options
        .split(',')
        .map(|opt| opt.trim()) // Handle whitespace around options
        .filter(|opt| {
            !opt.is_empty() && !RUNTIME_OPTIONS.contains(opt) && !opt.starts_with("subvolid=")
        })
        .collect();

    if filtered.is_empty() {
        "defaults".to_string()
    } else {
        filtered.join(",")
    }
}

/// Check if a filesystem type is empty or whitespace.
pub fn is_valid_fstype(fstype: &str) -> bool {
    !fstype.trim().is_empty()
}

/// Check if a mount target is under the given root path.
pub fn is_under_root(target: &str, root_str: &str) -> bool {
    if root_str == "/" {
        true // Everything is under "/"
    } else {
        target == root_str || target.starts_with(&format!("{}/", root_str))
    }
}
