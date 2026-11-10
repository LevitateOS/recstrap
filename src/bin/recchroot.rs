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
use install_tools::error::{Code, Error, Result};
use install_tools::session::{resolve_target, Session};
use std::process::ExitCode;
use target_root::{is_protected_path, is_root};

#[derive(Parser)]
#[command(name = "recchroot")]
#[command(version)]
#[command(about = "Enter chroot with proper bind mounts (like arch-chroot)")]
#[command(after_help = "Examples:\n  recchroot /mnt                 Open the target's root shell\n  recchroot /mnt passwd          Set the target's root password\n  recchroot /mnt /bin/sh -c 'cat /etc/os-release'\n\nRequires root. The target's /dev, /proc, /sys and /run are prepared for the\ncommand and cleaned up when it exits. Type 'exit' to leave the target shell.")]
struct Args {
    /// Chroot directory (e.g., /mnt)
    chroot_dir: String,

    /// Command to run (default: root's login shell in the target)
    #[arg(trailing_var_arg = true)]
    command: Vec<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("recchroot: {}", e);
            ExitCode::from(e.code.exit_status())
        }
    }
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

    // The target's own login shell for root.
    let passwd = target_root::read_optional(&root.join("etc/passwd"))?.unwrap_or_default();
    let command = match args.command.is_empty() {
        true => vec![target_root::root_shell(&passwd)],
        false => args.command,
    };

    let mut session = Session::begin("recchroot")?;
    session.prepare_chroot(&root)?;
    let status = session.chroot_command(&root, &command).status();
    // The command's status is the answer; a cleanup failure is reported
    // beside it, and replaces it only when the command succeeded.
    let closed = session.close();
    let status = status.map_err(|e| Error::command_failed("chroot", e))?;
    use std::os::unix::process::ExitStatusExt;
    // A signal death reports 128+N, as shells do.
    let code = match (status.code(), status.signal()) {
        (Some(code), _) => code,
        (None, Some(signal)) => 128 + signal,
        (None, None) => return Err(Error::new(Code::CommandFailed, format!("chroot ended without a status ({status})"))),
    };
    let code = u8::try_from(code)
        .map_err(|_| Error::new(Code::CommandFailed, format!("chroot ended with status {code}, which is no exit status")))?;
    match closed {
        Err(e) if code == 0 => Err(e),
        Err(e) => {
            eprintln!("recchroot: {e}");
            Ok(code)
        }
        Ok(()) => Ok(code),
    }
}
