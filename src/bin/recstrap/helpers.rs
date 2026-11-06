//! Utility functions for recstrap.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::constants::ROOTFS_SEARCH_PATHS;

// Re-export from distro-spec (single source of truth)
pub use distro_spec::shared::{is_mount_point, is_protected_path, is_root};

/// Find rootfs from canonical EROFS search paths.
pub fn find_rootfs() -> Option<&'static str> {
    ROOTFS_SEARCH_PATHS
        .iter()
        .find(|path| Path::new(path).exists())
        .copied()
}

/// Check if directory is empty for extraction purposes.
/// Ignores:
/// - lost+found (auto-created on ext4 mount points)
/// - .recstrap_write_test (leftover from interrupted write permission check)
pub fn is_dir_empty(path: &Path) -> std::io::Result<bool> {
    for entry in path.read_dir()? {
        let entry = entry?;
        let name = entry.file_name();
        // Ignore filesystem artifacts and our own test files
        if name != "lost+found" && name != ".recstrap_write_test" {
            return Ok(false);
        }
    }
    Ok(true)
}

// Note: is_mount_point() is now in distro-spec::shared::system (single source of truth)
// Re-exported above from distro_spec::shared::is_mount_point

/// Convert OsStr to CString for libc calls, preserving non-UTF8 bytes
pub fn path_to_cstring(path: &Path) -> std::io::Result<std::ffi::CString> {
    let bytes = path.as_os_str().as_bytes();
    std::ffi::CString::new(bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
}

/// Get available space on filesystem containing path (in bytes)
#[allow(clippy::unnecessary_cast)] // Cast needed - types vary by platform
pub fn get_available_space(path: &Path) -> std::io::Result<u64> {
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let c_path = path_to_cstring(path)?;

    let ret = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }

    // Available space = f_bavail * f_frsize
    Ok(stat.f_bavail as u64 * stat.f_frsize as u64)
}

/// Check if rootfs path is inside target directory
pub fn is_rootfs_inside_target(rootfs: &Path, target: &Path) -> bool {
    rootfs.starts_with(target)
}

/// Check if we can read the rootfs file (at least the first few bytes)
pub fn can_read_rootfs(path: &Path) -> bool {
    match File::open(path) {
        Ok(mut f) => {
            let mut buf = [0u8; 4];
            f.read_exact(&mut buf).is_ok()
        }
        Err(_) => false,
    }
}

/// Check if EROFS filesystem support is available in the kernel.
/// Checks /proc/filesystems for "erofs" entry.
pub fn erofs_supported() -> bool {
    match fs::read_to_string("/proc/filesystems") {
        Ok(content) => content.lines().any(|line| line.contains("erofs")),
        Err(_) => false,
    }
}

/// Try to load EROFS kernel module if not already loaded.
/// Returns true if EROFS is available after the attempt.
pub fn ensure_erofs_module() -> bool {
    if erofs_supported() {
        return true;
    }

    // Try to load the module (requires root, which we already checked)
    let _ = Command::new("modprobe")
        .arg("erofs")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // Check again
    erofs_supported()
}

/// Check if ssh-keygen is available
pub fn ssh_keygen_available() -> bool {
    Command::new("ssh-keygen")
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Regenerate SSH host keys in the target system.
///
/// SECURITY: The rootfs image contains pre-generated SSH host keys shared by
/// all installations. This function regenerates fresh keys for each installed
/// system to prevent MITM attacks.
///
/// Keys generated:
/// - RSA 3072-bit
/// - ECDSA P-256
/// - Ed25519
///
/// Returns Ok(()) on success, Err on failure.
pub fn regenerate_ssh_host_keys(target: &Path, quiet: bool) -> std::io::Result<()> {
    let ssh_dir = target.join("etc/ssh");

    // Skip if /etc/ssh doesn't exist (unusual, but handle gracefully)
    if !ssh_dir.is_dir() {
        if !quiet {
            eprintln!("recstrap: warning: /etc/ssh not found, skipping SSH key regeneration");
        }
        return Ok(());
    }

    // Check if ssh-keygen is available
    if !ssh_keygen_available() {
        if !quiet {
            eprintln!("recstrap: warning: ssh-keygen not found, skipping SSH key regeneration");
            eprintln!("         (installed system will use shared keys - regenerate manually!)");
        }
        return Ok(());
    }

    let key_types = [("rsa", 3072), ("ecdsa", 256), ("ed25519", 0)];

    for (key_type, bits) in key_types {
        let key_path = ssh_dir.join(format!("ssh_host_{}_key", key_type));
        let pub_key_path = ssh_dir.join(format!("ssh_host_{}_key.pub", key_type));

        // Remove old keys (from shared rootfs image)
        let _ = fs::remove_file(&key_path);
        let _ = fs::remove_file(&pub_key_path);

        // Generate fresh key pair
        let mut cmd = Command::new("ssh-keygen");
        cmd.arg("-t")
            .arg(key_type)
            .arg("-f")
            .arg(&key_path)
            .arg("-N")
            .arg("") // Empty passphrase
            .arg("-q"); // Quiet mode

        if bits > 0 {
            cmd.arg("-b").arg(bits.to_string());
        }

        let status = cmd.status()?;
        if !status.success() {
            return Err(std::io::Error::other(format!(
                "ssh-keygen failed for {} key",
                key_type
            )));
        }

        // Verify both files were created
        if !key_path.exists() || !pub_key_path.exists() {
            return Err(std::io::Error::other(format!(
                "SSH {} key pair not created",
                key_type
            )));
        }
    }

    if !quiet {
        eprintln!("  Generated fresh SSH host keys (rsa, ecdsa, ed25519)");
    }

    Ok(())
}

/// Interactively prompt for creating an initial user account.
///
/// This implements Option A from the installation plan: prompts for initial user
/// creation before chrooting. If accepted, prepares a script that adds the user
/// to wheel for administration through the installed distro's privilege tool.
///
/// Returns Ok if operation completed (user created or skipped), Err if something failed.
/// Portable login name: `[a-z_][a-z0-9_-]*`, at most 32 bytes. This also keeps
/// the name safe to embed in single quotes and never parsed as an option.
fn is_valid_username(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    name.len() <= 32
        && (first.is_ascii_lowercase() || first == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub fn prompt_for_user_creation(target: &Path) -> std::io::Result<()> {
    // Check if we can write to target
    let root_dir = target.join("root");
    if !root_dir.exists() {
        return Ok(()); // root dir doesn't exist yet, skip
    }

    eprintln!();
    eprintln!("LevitateOS: Initial User Setup");
    eprintln!();
    eprintln!("Root account is locked on installed systems for security.");
    eprintln!("You can either:");
    eprintln!("  1. Create an initial user account (recommended)");
    eprintln!("  2. Set root password later in chroot with 'passwd'");
    eprintln!();
    eprint!("Create initial user? [y/N]: ");
    std::io::stderr().flush()?;

    let mut response = String::new();
    std::io::stdin().read_line(&mut response)?;

    if response.trim().to_lowercase() != "y" && response.trim().to_lowercase() != "yes" {
        eprintln!("Skipped. You can set root password in chroot with: passwd");
        return Ok(());
    }

    // Prompt for username
    eprint!("Username: ");
    std::io::stderr().flush()?;
    let mut username = String::new();
    std::io::stdin().read_line(&mut username)?;
    let username = username.trim();

    if username.is_empty() {
        eprintln!("Invalid username. Skipping user creation.");
        return Ok(());
    }

    if !is_valid_username(username) {
        eprintln!(
            "Username must match [a-z_][a-z0-9_-]* (max 32 chars). Skipping user creation."
        );
        return Ok(());
    }

    // The target has no usable /etc/passwd tooling from here, so write a small
    // script for the user to run inside the chroot. It never contains the
    // password: `passwd` prompts for it inside the chroot with echo disabled.
    let script_path = root_dir.join("setup-initial-user.sh");
    let script_content = format!(
        "#!/bin/sh\n\
         set -eu\n\
         echo 'Creating user: {user}'\n\
         useradd -m -s /bin/bash -G wheel -- '{user}'\n\
         echo 'Set the password for {user}:'\n\
         passwd -- '{user}'\n\
         rm -f -- \"$0\"\n\
         echo 'User setup complete. You can now log in as {user}.'\n",
        user = username
    );

    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o700)
            .open(&script_path)?;
        file.write_all(script_content.as_bytes())?;
    }
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o700))?;
    }

    eprintln!();
    eprintln!("User setup script created at: /root/setup-initial-user.sh");
    eprintln!("Run this in chroot: bash /root/setup-initial-user.sh");
    eprintln!();

    Ok(())
}
