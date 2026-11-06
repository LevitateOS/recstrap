//! Rootfs type detection, validation, and extraction.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::constants::{EROFS_MAGIC, ESSENTIAL_DIRS};
use install_tools::error::{Error, Result};
use install_tools::session::Session;
use crate::guarded_ensure;

/// Rootfs type detected from file extension
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootfsType {
    Erofs,
}

impl RootfsType {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("erofs") => Some(Self::Erofs),
            _ => None,
        }
    }
}

/// Validate rootfs magic bytes match expected format.
/// Returns Ok(()) or Err if magic doesn't match.
pub fn validate_rootfs_magic(path: &Path, expected: RootfsType) -> std::io::Result<()> {
    let mut f = File::open(path)?;

    if expected == RootfsType::Erofs {
        // EROFS superblock is at offset 1024, magic is first 4 bytes
        f.seek(SeekFrom::Start(1024))?;
        let mut buf = [0u8; 4];
        f.read_exact(&mut buf)?;
        let magic = u32::from_le_bytes(buf);
        if magic != EROFS_MAGIC {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "not a valid EROFS image (magic: 0x{:08x}, expected: 0x{:08x})",
                    magic, EROFS_MAGIC
                ),
            ));
        }
    }

    Ok(())
}

/// Extract an EROFS image: mount it read-only in a private directory and copy
/// everything with `cp -aT` (always available, unlike rsync). The session
/// unmounts and removes the directory on every exit path, interrupts included.
pub fn extract_erofs(rootfs: &Path, target: &Path, quiet: bool) -> Result<()> {
    let mut session = Session::begin("recstrap");
    let mount_point = session.private_dir("recstrap-erofs")?;

    if !quiet {
        eprintln!("Mounting EROFS image...");
    }
    session.mount_image(rootfs, "erofs", &mount_point)?;

    if !quiet {
        eprintln!("Copying files from EROFS to target (this may take a while)...");
    }
    let cp_status = session
        .command("cp")
        .arg("-aT")
        .arg(&mount_point)
        .arg(target)
        .status()
        .map_err(|e| Error::command_failed("cp", e))?;
    if !cp_status.success() {
        let exit = cp_status.code().map_or("a signal".to_string(), |c| c.to_string());
        return Err(Error::extraction_failed(format!("cp exited with {exit}")));
    }

    if !quiet {
        eprintln!("Extraction complete, cleaning up...");
    }
    session.close()
}

/// Verify that essential directories exist after extraction.
/// These directories are required for a functioning Linux system.
///
/// # Cheat Vectors
///
/// - EASY: Reduce ESSENTIAL_DIRS to fewer directories
/// - EASY: Check for files instead of directories
/// - MEDIUM: Only check if path exists (could be file/symlink)
/// - HARD: Remove verification entirely
///
/// # Consequence if Cheated
///
/// System appears to extract successfully but is missing critical directories.
/// User boots into broken system, /bin or /usr missing, nothing works.
pub fn verify_extraction(target: &Path) -> Result<()> {
    let missing: Vec<&str> = ESSENTIAL_DIRS
        .iter()
        .filter(|dir| !target.join(dir).is_dir())
        .copied()
        .collect();

    guarded_ensure!(
        missing.is_empty(),
        Error::extraction_verification_failed(&missing),
        protects = "Extracted system has all essential directories",
        severity = "CRITICAL",
        cheats = [
            "Reduce ESSENTIAL_DIRS list",
            "Move missing dirs to 'optional' list",
            "Check exists() instead of is_dir()",
            "Skip verification entirely",
            "Only check one directory"
        ],
        consequence = "System extracts 'successfully' but is incomplete - /bin, /usr, or /etc missing, unbootable"
    );

    Ok(())
}
