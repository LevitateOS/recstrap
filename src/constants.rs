//! Constants for recstrap.
//!
//! Most constants are now in distro-spec::shared (single source of truth).
//! This module re-exports them for local use.

// Re-export from distro-spec (single source of truth)
pub use distro_spec::shared::{
    EROFS_MAGIC, ESSENTIAL_DIRS, MIN_REQUIRED_BYTES, ROOTFS_SEARCH_PATHS,
};

// Note: EROFS_MAGIC_OFFSET is also available from distro_spec::shared if needed.
