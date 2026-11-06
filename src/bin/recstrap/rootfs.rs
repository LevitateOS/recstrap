//! Rootfs extraction.

use install_tools::error::{Error, Result};
use install_tools::session::Session;
use std::path::Path;

/// Extract an EROFS image: mount it read-only in a private directory and copy
/// everything with `cp -aT` (always available, unlike rsync). The session
/// unmounts and removes the directory on every exit path, interrupts included.
pub fn extract_erofs(rootfs: &Path, target: &Path) -> Result<()> {
    let mut session = Session::begin("recstrap");
    let mount_point = session.private_dir("recstrap-erofs")?;

    session.mount_image(rootfs, "erofs", &mount_point)?;

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

    session.close()
}
