//! Constants for recstrap.

// Re-export PROTECTED_PATHS from distro-spec (single source of truth)
pub use distro_spec::shared::PROTECTED_PATHS;

/// Common rootfs locations to search (in order of preference).
/// EROFS paths are listed first as it's the modern format (Fedora 42+, LevitateOS).
pub const ROOTFS_SEARCH_PATHS: &[&str] = &[
    // EROFS (modern - LevitateOS default)
    "/media/cdrom/live/filesystem.erofs",
    "/run/initramfs/live/filesystem.erofs",
    "/run/archiso/bootmnt/live/filesystem.erofs",
    "/mnt/cdrom/live/filesystem.erofs",
    // Squashfs (legacy fallback)
    "/media/cdrom/live/filesystem.squashfs",
    "/run/initramfs/live/filesystem.squashfs",
    "/run/archiso/bootmnt/live/filesystem.squashfs",
    "/mnt/cdrom/live/filesystem.squashfs",
];

/// Essential directories that must exist after extraction
pub const ESSENTIAL_DIRS: &[&str] = &["bin", "etc", "lib", "sbin", "usr", "var"];

/// Minimum required space in bytes (2GB - typical compressed squashfs expands to this)
pub const MIN_REQUIRED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// EROFS magic number (little-endian at offset 1024)
pub const EROFS_MAGIC: u32 = 0xe0f5e1e2;

/// Squashfs magic bytes at offset 0
pub const SQUASHFS_MAGIC: &[u8; 4] = b"hsqs";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_essential_dirs_list() {
        assert!(ESSENTIAL_DIRS.contains(&"bin"));
        assert!(ESSENTIAL_DIRS.contains(&"etc"));
        assert!(ESSENTIAL_DIRS.contains(&"usr"));
        assert!(ESSENTIAL_DIRS.contains(&"lib"));
        assert!(ESSENTIAL_DIRS.contains(&"var"));
    }

    #[test]
    fn test_rootfs_search_paths_exist() {
        assert!(!ROOTFS_SEARCH_PATHS.is_empty());
        for path in ROOTFS_SEARCH_PATHS {
            assert!(
                path.ends_with(".erofs") || path.ends_with(".squashfs"),
                "Path {} should end with .erofs or .squashfs",
                path
            );
        }
    }

    #[test]
    fn test_min_required_bytes_is_reasonable() {
        // Should be at least 1GB, at most 10GB
        assert!(MIN_REQUIRED_BYTES >= 1024 * 1024 * 1024);
        assert!(MIN_REQUIRED_BYTES <= 10 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_erofs_magic_constant() {
        // EROFS magic is 0xe0f5e1e2 (little-endian)
        assert_eq!(EROFS_MAGIC, 0xe0f5e1e2);
    }

    #[test]
    fn test_squashfs_magic_constant() {
        // Squashfs magic is "hsqs"
        assert_eq!(SQUASHFS_MAGIC, b"hsqs");
    }
}
