//! Rootfs extraction.

use install_tools::error::{Error, Result};
use install_tools::output::{to_json_line, StrapEvent, STRAP_SCHEMA_VERSION};
use install_tools::session::Session;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

/// How often `--json` reports progress.
const PROGRESS_EVERY: Duration = Duration::from_millis(200);

/// Extract an EROFS image: mount it read-only in a private directory and copy
/// everything with GNU `cp -aT --preserve=xattr`. Explicit xattr preservation
/// makes missing capability/xattr support an error instead of silently losing
/// privilege metadata. Recipe supplies the deployment tools. The session
/// unmounts and removes the directory on every exit path, interrupts included.
///
/// With `events`, it prints `start` and `progress` events (see `StrapEvent`):
/// the image's entries are counted first, and `cp -v` names each one it copies.
pub fn extract_erofs(rootfs: &Path, target: &Path, events: bool) -> Result<(u64, u64)> {
    let mut session = Session::begin("recstrap")?;
    let mount_point = session.private_dir("recstrap-erofs")?;

    session.mount_image(rootfs, "erofs", &mount_point)?;

    let used_at_start = used_bytes(target)?;
    if events {
        let (total_entries, total_bytes) = count(&mount_point)?;
        println!(
            "{}",
            to_json_line(&StrapEvent::Start {
                schema_version: STRAP_SCHEMA_VERSION,
                target: target.to_string_lossy().into_owned(),
                rootfs: rootfs.to_string_lossy().into_owned(),
                total_entries,
                total_bytes,
            })
        );
    }

    let mut cp = session.command("cp");
    cp.arg(if events { "-aTv" } else { "-aT" })
        .arg("--preserve=xattr")
        .arg(&mount_point)
        .arg(target);
    if events {
        // cp's messages would break the JSON on stderr; they go into the error instead.
        cp.stdout(Stdio::piped()).stderr(Stdio::piped());
    }
    let mut child = cp.spawn().map_err(|e| Error::command_failed("cp", e))?;
    let stderr = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = pipe.read_to_string(&mut text);
            text
        })
    });
    let mut entries = 0u64;
    if let Some(stdout) = child.stdout.take() {
        let mut last = Instant::now();
        for line in BufReader::new(stdout).split(b'\n') {
            if line.is_err() {
                break;
            }
            entries += 1;
            if last.elapsed() >= PROGRESS_EVERY {
                last = Instant::now();
                let bytes = used_bytes(target)?.saturating_sub(used_at_start);
                println!("{}", to_json_line(&StrapEvent::Progress { entries, bytes }));
            }
        }
    }
    let cp_status = child.wait().map_err(|e| Error::command_failed("cp", e))?;
    let cp_errors = stderr.and_then(|t| t.join().ok()).unwrap_or_default();
    if !cp_status.success() {
        let exit = cp_status.code().map_or("a signal".to_string(), |c| c.to_string());
        let detail = cp_errors.lines().take(5).collect::<Vec<_>>().join("; ");
        return Err(Error::extraction_failed(if detail.is_empty() {
            format!("cp exited with {exit}")
        } else {
            format!("cp exited with {exit}: {detail}")
        }));
    }
    let bytes = used_bytes(target)?.saturating_sub(used_at_start);
    if events {
        println!("{}", to_json_line(&StrapEvent::Progress { entries, bytes }));
    }

    session.close()?;
    Ok((entries, bytes))
}

/// Space in use on the filesystem holding `path`.
fn used_bytes(path: &Path) -> Result<u64> {
    let unresolvable = |e| Error::path_unresolvable(&path.to_string_lossy(), e);
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|e| unresolvable(std::io::Error::new(std::io::ErrorKind::InvalidInput, e)))?;
    // SAFETY: zeroed statvfs is a valid out-parameter; `c_path` is NUL-terminated.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
        return Err(unresolvable(std::io::Error::last_os_error()));
    }
    #[allow(clippy::unnecessary_cast)] // field types vary by platform
    Ok((stat.f_blocks as u64).saturating_sub(stat.f_bfree as u64) * stat.f_frsize as u64)
}

/// Entries under `root` (not itself: `cp -T` doesn't name it), and the size of its
/// regular files.
fn count(root: &Path) -> Result<(u64, u64)> {
    let mut entries = 0u64;
    let mut bytes = 0u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        entries += 1;
        let listing = std::fs::read_dir(&dir)
            .map_err(|e| Error::path_unresolvable(&dir.to_string_lossy(), e))?;
        for entry in listing {
            let entry = entry.map_err(|e| Error::path_unresolvable(&dir.to_string_lossy(), e))?;
            let meta = entry
                .metadata()
                .map_err(|e| Error::path_unresolvable(&entry.path().to_string_lossy(), e))?;
            if meta.is_dir() {
                pending.push(entry.path());
            } else {
                entries += 1;
                if meta.is_file() {
                    bytes += meta.len();
                }
            }
        }
    }
    Ok((entries - 1, bytes))
}
