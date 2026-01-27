//! Constants for recstrap.
//!
//! Most constants are now in distro-spec::shared (single source of truth).
//! This module re-exports them for local use.

// Re-export from distro-spec (single source of truth)
pub use distro_spec::shared::{
    EROFS_MAGIC, ESSENTIAL_DIRS, MIN_REQUIRED_BYTES, ROOTFS_SEARCH_PATHS, SQUASHFS_MAGIC,
};

// Note: EROFS_MAGIC_OFFSET is also available from distro_spec::shared if needed.

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
