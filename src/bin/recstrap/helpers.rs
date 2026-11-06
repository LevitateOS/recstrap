//! Utility functions for recstrap.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

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
