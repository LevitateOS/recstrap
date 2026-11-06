//! recchroot - Enter chroot with proper bind mounts
//!
//! Like arch-chroot for Arch Linux - sets up /dev, /proc, /sys, /run, then chroots.
//! Does ONE thing: enter chroot properly. Cleans up on exit.
//!
//! Usage:
//!   recchroot /mnt              # Interactive shell
//!   recchroot /mnt passwd       # Run single command
//!
//! This is NOT an installer. This enters chroot. That's it.

use clap::Parser;
use install_tools::error::{Error, Result};
use install_tools::session::{resolve_target, Session};
use distro_spec::shared::is_root;
use std::path::Path;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "recchroot")]
#[command(version)]
#[command(about = "Enter chroot with proper bind mounts (like arch-chroot)")]
struct Args {
    /// Chroot directory (e.g., /mnt)
    chroot_dir: String,

    /// Command to run (default: root's login shell in the target)
    #[arg(trailing_var_arg = true)]
    command: Vec<String>,
}

/// Host trees bound into the chroot. `/sys` is bound recursively, so efivarfs
/// (when the host has it) comes along without a mount of its own.
const BIND_DIRS: &[&str] = &["/proc", "/sys", "/dev", "/run"];

use distro_spec::shared::is_protected_path;

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("recchroot: {}", e);
            ExitCode::from(e.code.exit_status())
        }
    }
}

/// The target's own login shell for root, so an interactive chroot works on
/// any distro (Acorn ships ash, not bash). Falls back to POSIX `/bin/sh`.
fn target_root_shell(root: &Path) -> String {
    std::fs::read_to_string(root.join("etc/passwd"))
        .ok()
        .and_then(|passwd| {
            passwd
                .lines()
                .find(|line| line.starts_with("root:"))
                .and_then(|line| line.split(':').nth(6))
                .filter(|shell| !shell.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "/bin/sh".to_string())
}

fn run() -> Result<u8> {
    let args = Args::parse();
    let root = resolve_target(&args.chroot_dir)?;

    // Checked on the canonical path, so a symlink to a protected path is caught.
    if is_protected_path(&root) {
        return Err(Error::protected_path(&root.to_string_lossy()));
    }
    if !is_root() {
        return Err(Error::not_root());
    }

    let mut session = Session::begin("recchroot");
    for dir in BIND_DIRS {
        session.bind_dir(Path::new(dir), &root.join(dir.trim_start_matches('/')))?;
    }

    // Host DNS for the session only, without writing into the target. A
    // symlinked target resolv.conf is left alone: it resolves inside the
    // chroot (typically into the bound /run). DNS is a convenience, so a
    // failure is a warning.
    if let Ok(host_resolv) = std::fs::canonicalize("/etc/resolv.conf") {
        let etc = root.join("etc");
        let target = etc.join("resolv.conf");
        let target_is_symlink = std::fs::symlink_metadata(&target)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        if etc.is_dir() && !target_is_symlink {
            if let Err(e) = session.bind_file(&host_resolv, &target) {
                eprintln!("recchroot: warning: DNS not configured: {e}");
            }
        }
    }

    let shell = target_root_shell(&root);
    let (cmd, cmd_args) = match args.command.split_first() {
        Some((cmd, rest)) => (cmd.as_str(), rest),
        None => (shell.as_str(), &[][..]),
    };
    let status = session
        .command("chroot")
        .arg(&root)
        .arg(cmd)
        .args(cmd_args)
        .status();
    session.close()?;
    let status = status.map_err(|e| Error::command_failed("chroot", e))?;

    // Pass the command's own status through; a signal death reports 128+N,
    // as shells do.
    use std::os::unix::process::ExitStatusExt;
    let code = match (status.code(), status.signal()) {
        (Some(code), _) => code,
        (None, Some(signal)) => 128 + signal,
        (None, None) => 1,
    };
    Ok(u8::try_from(code).unwrap_or(1))
}
