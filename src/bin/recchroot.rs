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
use distro_spec::shared::is_root;
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::sys::signal::{signal, SigHandler, Signal};
use std::fs;
use std::path::{Path, PathBuf};
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};

#[derive(Parser)]
#[command(name = "recchroot")]
#[command(version)]
#[command(about = "Enter chroot with proper bind mounts (like arch-chroot)")]
struct Args {
    /// Chroot directory (e.g., /mnt)
    chroot_dir: String,

    /// Command to run (default: /bin/bash)
    #[arg(trailing_var_arg = true)]
    command: Vec<String>,
}

// =============================================================================
// Constants
// =============================================================================

/// Required mounts (in order)
const BIND_MOUNTS: &[(&str, &str)] = &[
    ("/proc", "proc"),
    ("/sys", "sys"),
    ("/dev", "dev"),
    ("/run", "run"),
];

/// Optional mounts (only if source exists)
const OPTIONAL_MOUNTS: &[&str] = &["/sys/firmware/efi/efivars"];

// Note: PROTECTED_PATHS is now in distro-spec::shared (single source of truth).
// Use is_protected_path() imported below.

// =============================================================================
// Main
// =============================================================================

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("recchroot: {}", e);
            ExitCode::from(e.code.exit_status())
        }
    }
}

/// Helper to clean up mounts on exit or error
fn cleanup_mounts(mounted: &[PathBuf]) {
    for target in mounted.iter().rev() {
        if let Err(e) = umount2(target, MntFlags::MNT_DETACH) {
            eprintln!("recchroot: warning: {}", Error::unmount_failed(target, e));
        }
    }
}

/// Refuse mount targets that are symlinks: joining them onto the chroot root
/// and mounting would follow the link and could land on a host path.
fn refuse_symlink(target: &Path) -> Result<()> {
    match fs::symlink_metadata(target) {
        Ok(meta) if meta.file_type().is_symlink() => Err(Error::unsafe_mount_target(
            target,
            "it is a symlink and would resolve outside the chroot",
        )),
        _ => Ok(()),
    }
}

/// Bind-mount the host's resolv.conf onto `<root>/etc/resolv.conf`.
///
/// Returns the mount target to unmount at cleanup, or `None` when nothing was
/// mounted (no host file, or the target file is a symlink). A placeholder file
/// created for the mount is recorded in `created_files` for removal.
fn bind_host_resolv_conf(root: &Path, created_files: &mut Vec<PathBuf>) -> Result<Option<PathBuf>> {
    let Ok(host_resolv) = fs::canonicalize("/etc/resolv.conf") else {
        return Ok(None);
    };

    let etc = root.join("etc");
    refuse_symlink(&etc)?;
    let target = etc.join("resolv.conf");

    match fs::symlink_metadata(&target) {
        Ok(meta) if meta.file_type().is_symlink() => return Ok(None),
        Ok(meta) if meta.is_file() => {}
        Ok(_) => {
            return Err(Error::unsafe_mount_target(&target, "it is not a regular file"))
        }
        Err(_) => {
            if !etc.is_dir() {
                return Ok(None);
            }
            fs::File::create(&target).map_err(|e| Error::create_failed(&target, e))?;
            created_files.push(target.clone());
        }
    }

    mount(
        Some(&host_resolv),
        &target,
        None::<&str>,
        MsFlags::MS_BIND,
        None::<&str>,
    )
    .map_err(|e| Error::mount_failed("/etc/resolv.conf", &target, e))?;

    Ok(Some(target))
}

// Use is_protected_path from distro-spec (single source of truth)
use distro_spec::shared::is_protected_path;

fn run() -> Result<u8> {
    let args = Args::parse();

    // Validate empty/whitespace-only paths first (no root needed)
    let chroot_dir_str = args.chroot_dir.trim();
    if chroot_dir_str.is_empty() {
        return Err(Error::target_not_found("<empty>"));
    }

    let chroot_dir = Path::new(chroot_dir_str);
    if !chroot_dir.exists() {
        return Err(Error::target_not_found(chroot_dir_str));
    }
    if !chroot_dir.is_dir() {
        return Err(Error::not_a_directory(chroot_dir_str));
    }

    // Canonicalize to resolve symlinks and ..
    let chroot_canonical = chroot_dir
        .canonicalize()
        .map_err(|e| Error::path_unresolvable(chroot_dir_str, e))?;

    // Check protected paths AFTER canonicalization (catches symlinks to protected paths)
    if is_protected_path(&chroot_canonical) {
        return Err(Error::protected_path(
            &chroot_canonical.to_string_lossy(),
        ));
    }

    // Check root privileges (needed for mount operations)
    if !is_root() {
        return Err(Error::not_root());
    }

    let mut mounted: Vec<PathBuf> = Vec::new();

    // Set up signal handlers for cleanup - ignore signals during mount setup
    // so cleanup always happens. We handle SIGINT, SIGTERM, and SIGQUIT.
    unsafe {
        let _ = signal(Signal::SIGINT, SigHandler::SigIgn);
        let _ = signal(Signal::SIGTERM, SigHandler::SigIgn);
        let _ = signal(Signal::SIGQUIT, SigHandler::SigIgn);
    }

    // Setup bind mounts - cleanup on failure
    let mount_result = (|| -> Result<()> {
        for (src, name) in BIND_MOUNTS {
            let target = chroot_canonical.join(name);
            refuse_symlink(&target)?;

            if !target.exists() {
                fs::create_dir_all(&target).map_err(|e| Error::create_failed(&target, e))?;
            }

            mount(
                Some(Path::new(src)),
                &target,
                None::<&str>,
                MsFlags::MS_BIND | MsFlags::MS_REC,
                None::<&str>,
            )
            .map_err(|e| Error::mount_failed(src, &target, e))?;
            mounted.push(target.clone());

            // Recursive binds inherit the host's shared propagation; make them
            // slaves so mounts and lazy unmounts inside the chroot never
            // propagate back to the host (same as `mount --make-rslave`).
            mount(
                None::<&str>,
                &target,
                None::<&str>,
                MsFlags::MS_SLAVE | MsFlags::MS_REC,
                None::<&str>,
            )
            .map_err(|e| Error::mount_failed(src, &target, e))?;
        }
        Ok(())
    })();

    if let Err(e) = mount_result {
        cleanup_mounts(&mounted);
        return Err(e);
    }

    // Optional mounts (like efivars) - warn on failure but continue
    for src in OPTIONAL_MOUNTS {
        let src_path = Path::new(src);
        if !src_path.exists() {
            continue;
        }

        let rel_path = src.strip_prefix('/').unwrap_or(src);
        let target = chroot_canonical.join(rel_path);
        if let Err(e) = refuse_symlink(&target) {
            eprintln!("recchroot: warning: skipping optional mount: {}", e);
            continue;
        }

        if !target.exists() {
            if let Err(e) = fs::create_dir_all(&target) {
                eprintln!(
                    "recchroot: warning: cannot create '{}': {}",
                    target.display(),
                    e
                );
                continue;
            }
        }

        if let Err(e) = mount(
            Some(src_path),
            &target,
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        ) {
            eprintln!(
                "recchroot: warning: cannot mount '{}': {}",
                target.display(),
                e
            );
            continue;
        }

        mounted.push(target);
    }

    // Provide host DNS without writing into the target (or through it into
    // the host): bind-mount the host resolv.conf over the target file for the
    // session only. A symlinked target resolv.conf is left alone; it resolves
    // inside the chroot (typically into the bind-mounted /run).
    let mut created_files: Vec<PathBuf> = Vec::new();
    match bind_host_resolv_conf(&chroot_canonical, &mut created_files) {
        Ok(Some(target)) => mounted.push(target),
        Ok(None) => {}
        Err(e) => eprintln!("recchroot: warning: DNS not configured: {}", e),
    }

    // Determine command to run
    let (cmd, cmd_args): (&str, Vec<&str>) = if args.command.is_empty() {
        ("/bin/bash", vec![])
    } else {
        (
            args.command[0].as_str(),
            args.command[1..].iter().map(|s| s.as_str()).collect(),
        )
    };

    // Run chroot. The parent keeps ignoring SIGINT/SIGTERM/SIGQUIT so it can
    // always clean up, but ignored dispositions survive exec, so restore the
    // defaults in the child or Ctrl-C would not work inside the chroot.
    let mut chroot_cmd = Command::new("chroot");
    chroot_cmd.arg(&chroot_canonical).arg(cmd).args(&cmd_args);
    // SAFETY: only async-signal-safe sigaction calls run between fork and exec.
    unsafe {
        chroot_cmd.pre_exec(|| {
            for sig in [Signal::SIGINT, Signal::SIGTERM, Signal::SIGQUIT] {
                signal(sig, SigHandler::SigDfl).map_err(std::io::Error::from)?;
            }
            Ok(())
        });
    }
    let status = chroot_cmd.status();

    // Always cleanup, even if chroot failed
    cleanup_mounts(&mounted);
    for file in &created_files {
        let _ = fs::remove_file(file);
    }

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

// =============================================================================
// Tests
// =============================================================================
