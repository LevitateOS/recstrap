//! Mount, run a child, and always clean up.
//!
//! A `Session` is the one owner of everything the install tools mount or
//! create on the host. Beginning one moves the process into a private mount
//! namespace (see `enter_private_mount_namespace`), so whatever it mounts
//! disappears when the process and its children are gone, however they die
//! (SIGKILL included), and never propagates to or from the host. While it
//! lives, SIGHUP/SIGINT/SIGTERM/SIGQUIT are ignored by the tool (children get
//! the default dispositions back), so a hangup or interrupt stops the child
//! and the session still undoes its work. `close` unmounts in reverse order,
//! removes what it created, restores the signal dispositions and reports any
//! failure; dropping an unclosed session does the same and prints the
//! failures.

use crate::error::{Error, Result};
use crate::guard::is_root;
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const HANDLED_SIGNALS: [libc::c_int; 4] =
    [libc::SIGHUP, libc::SIGINT, libc::SIGTERM, libc::SIGQUIT];

pub struct Session {
    tool: &'static str,
    mounts: Vec<PathBuf>,
    created_files: Vec<PathBuf>,
    created_dirs: Vec<PathBuf>,
    previous_signals: Vec<(libc::c_int, libc::sighandler_t)>,
    closed: bool,
}

impl Session {
    pub fn begin(tool: &'static str) -> Result<Self> {
        enter_private_mount_namespace(tool)?;
        let previous_signals = HANDLED_SIGNALS
            .iter()
            // SAFETY: installing SIG_IGN runs no handler code.
            .map(|&sig| (sig, unsafe { libc::signal(sig, libc::SIG_IGN) }))
            .collect();
        Ok(Self {
            tool,
            mounts: Vec::new(),
            created_files: Vec::new(),
            created_dirs: Vec::new(),
            previous_signals,
            closed: false,
        })
    }

    /// A fresh private (0700) directory, removed again when the session ends.
    pub fn private_dir(&mut self, prefix: &str) -> Result<PathBuf> {
        use std::ffi::{CString, OsString};
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let base = std::env::temp_dir();
        let template = base.join(format!("{prefix}.XXXXXX"));
        let template = CString::new(template.as_os_str().as_bytes())
            .map_err(|e| Error::create_failed(&base, e))?;
        let mut raw = template.into_bytes_with_nul();
        // SAFETY: `raw` is a writable NUL-terminated template ending in XXXXXX.
        if unsafe { libc::mkdtemp(raw.as_mut_ptr().cast()) }.is_null() {
            return Err(Error::create_failed(&base, std::io::Error::last_os_error()));
        }
        raw.pop();
        let dir = PathBuf::from(OsString::from_vec(raw));
        self.created_dirs.push(dir.clone());
        Ok(dir)
    }

    /// Bind-mount a directory tree. Recursive binds are made slaves so mounts
    /// and unmounts inside never propagate back to the host (already true in
    /// the session's private namespace; this covers running without one).
    pub fn bind_dir(&mut self, source: &Path, target: &Path) -> Result<()> {
        refuse_symlink(target)?;
        if !target.exists() {
            fs::create_dir_all(target).map_err(|e| Error::create_failed(target, e))?;
        }
        let src = source.display().to_string();
        mount(
            Some(source),
            target,
            None::<&str>,
            MsFlags::MS_BIND | MsFlags::MS_REC,
            None::<&str>,
        )
        .map_err(|e| Error::mount_failed(&src, target, e))?;
        self.mounts.push(target.to_path_buf());
        mount(
            None::<&str>,
            target,
            None::<&str>,
            MsFlags::MS_SLAVE | MsFlags::MS_REC,
            None::<&str>,
        )
        .map_err(|e| Error::mount_failed(&src, target, e))
    }

    /// Bind-mount a single file, creating an empty placeholder (removed again
    /// at the end) when the target file does not exist yet.
    pub fn bind_file(&mut self, source: &Path, target: &Path) -> Result<()> {
        if let Some(parent) = target.parent() {
            refuse_symlink(parent)?;
        }
        match fs::symlink_metadata(target) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => return Err(Error::unsafe_mount_target(target, "it is not a regular file")),
            Err(_) => {
                fs::File::create(target).map_err(|e| Error::create_failed(target, e))?;
                self.created_files.push(target.to_path_buf());
            }
        }
        mount(
            Some(source),
            target,
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        )
        .map_err(|e| Error::mount_failed(&source.display().to_string(), target, e))?;
        self.mounts.push(target.to_path_buf());
        Ok(())
    }

    /// Mount a filesystem image read-only through a loop device. mount(8) sets
    /// up the loop device (auto-cleared on unmount), which the raw syscall
    /// cannot do.
    pub fn mount_image(&mut self, image: &Path, fstype: &str, target: &Path) -> Result<()> {
        let src = image.display().to_string();
        let status = self
            .command("mount")
            .args(["-t", fstype, "-o", "ro,loop"])
            .arg(image)
            .arg(target)
            .status()
            .map_err(|e| Error::command_failed("mount", e))?;
        if !status.success() {
            let exit = status.code().map_or("a signal".to_string(), |c| c.to_string());
            return Err(Error::mount_failed(&src, target, format!("mount exited with {exit}")));
        }
        self.mounts.push(target.to_path_buf());
        Ok(())
    }

    /// A command whose child process gets the default signal dispositions.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut cmd = Command::new(program);
        // SAFETY: only async-signal-safe signal() calls run between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                for sig in HANDLED_SIGNALS {
                    libc::signal(sig, libc::SIG_DFL);
                }
                Ok(())
            });
        }
        cmd
    }

    /// Undo everything, in reverse order, and report the first failure.
    pub fn close(mut self) -> Result<()> {
        let failures = self.undo();
        failures.into_iter().next().map_or(Ok(()), Err)
    }

    fn undo(&mut self) -> Vec<Error> {
        self.closed = true;
        let mut failures = Vec::new();
        let mut still_mounted = Vec::new();
        for target in self.mounts.drain(..).rev() {
            if let Err(e) = umount2(&target, MntFlags::MNT_DETACH) {
                failures.push(Error::unmount_failed(&target, e));
                still_mounted.push(target);
            }
        }
        for file in self.created_files.drain(..) {
            if still_mounted.contains(&file) {
                continue;
            }
            if let Err(e) = fs::remove_file(&file) {
                failures.push(Error::remove_failed(&file, e));
            }
        }
        for dir in self.created_dirs.drain(..).rev() {
            if still_mounted.contains(&dir) {
                continue;
            }
            // remove_dir, not remove_dir_all: anything left inside is kept.
            if let Err(e) = fs::remove_dir(&dir) {
                failures.push(Error::remove_failed(&dir, e));
            }
        }
        for (sig, previous) in self.previous_signals.drain(..) {
            // SAFETY: restores the disposition captured in `begin`.
            unsafe {
                libc::signal(sig, previous);
            }
        }
        failures
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.closed {
            for failure in self.undo() {
                eprintln!("{}: warning: {failure}", self.tool);
            }
        }
    }
}

/// Give this process a mount namespace of its own: unshare(CLONE_NEWNS), then
/// make every mount in it private. The namespace starts as a copy of the
/// host's, so mounts the host already has (the target at /mnt, its /boot)
/// stay visible and writes to them reach the disk; but mounts made here are
/// not seen by the host, and they are torn down by the kernel when the last
/// process in the namespace exits. Children (chroot, mount, cp) inherit it, so
/// a mount lives exactly as long as the tool or the command it runs. Nothing
/// mounted inside a session outlives it, a mount made by the chrooted command
/// included.
///
/// Without the privilege (EPERM: not root, or a container that forbids it)
/// the session goes on in the host's namespace, as before: mounting needs the
/// same privilege, so it fails later with its own error. Any other failure
/// stops the session before it mounts anything; a namespace whose mounts are
/// still shared with the host would make the binds leak into it.
fn enter_private_mount_namespace(tool: &str) -> Result<()> {
    // SAFETY: unshare has no memory-safety preconditions; the tools are
    // single-threaded when a session begins.
    if unsafe { libc::unshare(libc::CLONE_NEWNS) } != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EPERM) {
            if is_root() {
                eprintln!(
                    "{tool}: warning: no private mount namespace ({err}); if {tool} is killed, its mounts stay behind"
                );
            }
            return Ok(());
        }
        return Err(Error::private_namespace_failed(err));
    }
    mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .map_err(Error::private_namespace_failed)
}

/// Refuse mount targets that are symlinks: mounting would follow the link and
/// could land on a host path.
fn refuse_symlink(target: &Path) -> Result<()> {
    match fs::symlink_metadata(target) {
        Ok(meta) if meta.file_type().is_symlink() => Err(Error::unsafe_mount_target(
            target,
            "it is a symlink and could resolve outside the target",
        )),
        _ => Ok(()),
    }
}

/// Resolve a target directory argument: reject empty input, require an
/// existing directory, and return its canonical path (symlinks and `..`
/// resolved) for every later check.
pub fn resolve_target(arg: &str) -> Result<PathBuf> {
    let trimmed = arg.trim();
    if trimmed.is_empty() {
        return Err(Error::target_not_found("<empty>"));
    }
    let path = Path::new(trimmed);
    if !path.exists() {
        return Err(Error::target_not_found(trimmed));
    }
    if !path.is_dir() {
        return Err(Error::not_a_directory(trimmed));
    }
    path.canonicalize()
        .map_err(|e| Error::path_unresolvable(trimmed, e))
}
