//! The target-side primitives of the install tools (recpart, recstrap,
//! recchroot, recfstab, recsetup, recab): each is implemented here once, and
//! no tool keeps its own copy.
//!
//! One probe rule holds throughout: only "not found" means absent. Any other
//! I/O error is returned, naming its path, so a check never passes because it
//! could not look. Errors are `std::io::Error`s (the kind kept, the path in
//! the message), so every tool wraps them in its own error type.

use std::ffi::{CString, OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Output};

/// `err`, with `path` named in its message and its kind kept.
pub fn at(path: &Path, err: io::Error) -> io::Error {
    io::Error::new(err.kind(), format!("'{}': {err}", path.display()))
}

/// The file's text; `None` only when it does not exist.
pub fn read_optional(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(at(path, err)),
    }
}

/// Whether anything (a symlink included) exists at `path`.
pub fn exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
        Err(err) => Err(at(path, err)),
    }
}

/// The names in a directory, sorted; empty only when it does not exist.
pub fn dir_names(dir: &Path) -> io::Result<Vec<OsString>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(at(dir, err)),
    };
    let mut names = entries
        .map(|entry| entry.map(|e| e.file_name()).map_err(|e| at(dir, e)))
        .collect::<io::Result<Vec<_>>>()?;
    names.sort();
    Ok(names)
}

// ---- the protected-path guard ----

/// Host system directories that are never an extraction, chroot or install
/// target.
pub const PROTECTED_PATHS: &[&str] = &[
    "/", "/bin", "/boot", "/boot/efi", "/dev", "/efi", "/etc", "/home", "/lib", "/lib32", "/lib64",
    "/opt", "/proc", "/root", "/run", "/sbin", "/srv", "/sys", "/tmp", "/usr", "/usr/local", "/var",
];

/// Whether `path` is one of [`PROTECTED_PATHS`]. The path is resolved first
/// (symlinks when it exists, else `.`/`..` lexically), so `/mnt/../usr` or a
/// symlink to `/etc` is caught. Children such as `/mnt/usr` are not protected.
pub fn is_protected_path(path: &Path) -> bool {
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| {
        let mut out = PathBuf::from("/");
        for component in path.components() {
            match component {
                Component::ParentDir => {
                    out.pop();
                }
                Component::Normal(part) => out.push(part),
                Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
            }
        }
        out
    });
    PROTECTED_PATHS.iter().any(|protected| resolved == Path::new(protected))
}

/// Whether this process runs as root.
pub fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

// ---- os-release and the account files ----

/// The os-release of the system under `root` (`etc/os-release`, else
/// `usr/lib/os-release`); `None` when it has neither. An absolute symlink is
/// followed inside `root`, never onto the host.
pub fn os_release(root: &Path) -> io::Result<Option<String>> {
    for rel in ["etc/os-release", "usr/lib/os-release"] {
        let mut path = root.join(rel);
        for _ in 0..8 {
            match fs::read_link(&path) {
                Ok(link) if link.is_absolute() => {
                    path = root.join(link.strip_prefix("/").unwrap_or(&link))
                }
                Ok(link) => path = path.parent().map_or(link.clone(), |dir| dir.join(&link)),
                Err(_) => break,
            }
        }
        if let Some(text) = read_optional(&path)? {
            return Ok(Some(text));
        }
    }
    Ok(None)
}

/// The names (first fields) of a colon-separated account file
/// (`passwd`, `group`, `shadow`, `gshadow`).
pub fn account_names(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.split(':').next())
        .filter(|name| !name.is_empty() && !name.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Root's login shell in a `passwd` file's text; `/bin/sh` when it names none.
pub fn root_shell(passwd: &str) -> String {
    passwd
        .lines()
        .find(|line| line.starts_with("root:"))
        .and_then(|line| line.split(':').nth(6))
        .filter(|shell| !shell.is_empty())
        .unwrap_or("/bin/sh")
        .to_string()
}

/// An account file merged by name: `image`'s lines in their order, each
/// replaced by `own`'s line of the same name, then `own`'s other lines.
pub fn merge_accounts(image: &str, own: &str) -> String {
    let name = |line: &str| line.split(':').next().unwrap_or("").to_string();
    let own_lines: Vec<&str> = own.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut used = std::collections::HashSet::new();
    let mut merged: Vec<&str> = Vec::new();
    for line in image.lines().filter(|l| !l.trim().is_empty()) {
        let key = name(line);
        match own_lines.iter().find(|own| name(own) == key) {
            Some(own) => {
                used.insert(key);
                merged.push(own);
            }
            None => merged.push(line),
        }
    }
    merged.extend(own_lines.iter().filter(|line| !used.contains(&name(line))));
    merged.join("\n") + "\n"
}

// ---- running commands ----

/// How a finished process ended: `exit N`, or the signal that killed it.
pub fn status_text(status: ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("exit {code}"),
        (_, Some(signal)) => format!("killed by signal {signal}"),
        _ => status.to_string(),
    }
}

fn command_text(command: &Command) -> String {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(OsStr::to_string_lossy)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Run `command` to completion, capturing its output. A command that cannot
/// start or does not succeed is an error naming it, how it ended and its
/// stderr.
pub fn run(command: &mut Command) -> io::Result<Output> {
    let output = command
        .output()
        .map_err(|err| io::Error::new(err.kind(), format!("running '{}': {err}", command_text(command))))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() { String::new() } else { format!(": {}", stderr.trim()) };
        return Err(io::Error::other(format!(
            "'{}' failed ({}){detail}",
            command_text(command),
            status_text(output.status)
        )));
    }
    Ok(output)
}

// ---- block devices ----

/// A tag blkid reads from `device` (`PARTUUID`, `LABEL`, `UUID`, ...);
/// `None` when the device has no such tag (blkid's exit 2).
pub fn blkid_tag(device: &Path, tag: &str) -> io::Result<Option<String>> {
    let mut command = Command::new("blkid");
    command.args(["-s", tag, "-o", "value"]).arg(device);
    let output = command
        .output()
        .map_err(|err| io::Error::new(err.kind(), format!("running 'blkid': {err}")))?;
    if output.status.code() == Some(2) {
        return Ok(None);
    }
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "'{}' failed ({}): {}",
            command_text(&command),
            status_text(output.status),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!value.is_empty()).then_some(value))
}

/// A partition's PARTUUID, lower-case (the spelling every install tool
/// writes and compares); `None` when it has none (not a GPT partition).
pub fn partuuid(device: &Path) -> io::Result<Option<String>> {
    Ok(blkid_tag(device, "PARTUUID")?.map(|value| value.to_ascii_lowercase()))
}

/// A block device's size in bytes: the kernel's sector count in sysfs (always
/// 512-byte units), else the device's end.
pub fn block_device_bytes(device: &Path) -> io::Result<u64> {
    let canonical = fs::canonicalize(device).map_err(|err| at(device, err))?;
    if let Some(name) = canonical.file_name() {
        let size = Path::new("/sys/class/block").join(name).join("size");
        if let Some(text) = read_optional(&size)? {
            let sectors: u64 = text.trim().parse().map_err(|err| {
                io::Error::new(ErrorKind::InvalidData, format!("'{}': {err}", size.display()))
            })?;
            return Ok(sectors * 512);
        }
    }
    File::open(&canonical)
        .and_then(|mut file| file.seek(SeekFrom::End(0)))
        .map_err(|err| at(&canonical, err))
}

// ---- the mount table ----

/// One mount, from `/proc/self/mountinfo`.
#[derive(Debug, Clone)]
pub struct Mount {
    pub id: u64,
    /// The filesystem's device number (major, minor). Every mount of one
    /// filesystem has the same number, all subvolumes of a btrfs filesystem
    /// included (unlike `st_dev`, which differs per subvolume).
    pub dev: (u32, u32),
    /// The directory of the filesystem that is mounted (a btrfs subvolume's
    /// path, or `/`).
    pub root: PathBuf,
    pub mount_point: PathBuf,
    pub fstype: String,
    /// What was mounted: a device path for block filesystems.
    pub source: String,
    /// The mount's options and the filesystem's, as one list (`rw` or `ro`
    /// once), as findmnt's OPTIONS shows them.
    pub options: String,
}

/// This process's mount table, in mount order (a later mount on the same
/// point covers an earlier one).
pub fn mount_table() -> io::Result<Vec<Mount>> {
    let path = Path::new("/proc/self/mountinfo");
    let text = fs::read_to_string(path).map_err(|err| at(path, err))?;
    text.lines()
        .map(|line| {
            parse_mountinfo_line(line).ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidData, format!("{}: unreadable line '{line}'", path.display()))
            })
        })
        .collect()
}

/// `id parent major:minor root mount-point options [optional...] - fstype source super-options`
fn parse_mountinfo_line(line: &str) -> Option<Mount> {
    let fields: Vec<&str> = line.split(' ').collect();
    let (major, minor) = fields.get(2)?.split_once(':')?;
    let separator = fields.iter().skip(6).position(|f| *f == "-")? + 6;
    let text = |field: &str| String::from_utf8_lossy(&unescape_octal(field)).into_owned();
    let mut options: Vec<String> = fields.get(5)?.split(',').map(str::to_string).collect();
    for option in fields.get(separator + 3).unwrap_or(&"").split(',') {
        if !option.is_empty() && option != "rw" && option != "ro" && !options.iter().any(|o| o == option) {
            options.push(option.to_string());
        }
    }
    Some(Mount {
        id: fields.first()?.parse().ok()?,
        dev: (major.parse().ok()?, minor.parse().ok()?),
        root: OsString::from_vec(unescape_octal(fields.get(3)?)).into(),
        mount_point: OsString::from_vec(unescape_octal(fields.get(4)?)).into(),
        fstype: text(fields.get(separator + 1)?),
        source: text(fields.get(separator + 2)?),
        options: text(&options.join(",")),
    })
}

/// The kernel's `\NNN` octal escapes (mountinfo, fstab): space, tab, newline
/// and backslash.
pub fn unescape_octal(field: &str) -> Vec<u8> {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = bytes
            .get(i + 1..i + 4)
            .filter(|d| bytes[i] == b'\\' && d.iter().all(|c| (b'0'..=b'7').contains(c)));
        match octal {
            Some(d) => {
                out.push(d.iter().fold(0u8, |n, c| n.wrapping_mul(8).wrapping_add(c - b'0')));
                i += 4;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    out
}

/// The mount that holds `path` (an existing, canonical path): the one
/// statx(2) names, or, on kernels without `STATX_MNT_ID`, the topmost mount
/// with the longest mount-point prefix.
pub fn mount_holding<'a>(table: &'a [Mount], path: &Path) -> io::Result<Option<&'a Mount>> {
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|err| io::Error::new(ErrorKind::InvalidInput, format!("'{}': {err}", path.display())))?;
    // SAFETY: a zeroed statx is a valid out-parameter; `c_path` is NUL-terminated.
    let mut buf: libc::statx = unsafe { std::mem::zeroed() };
    if unsafe { libc::statx(libc::AT_FDCWD, c_path.as_ptr(), 0, libc::STATX_MNT_ID, &mut buf) } != 0 {
        return Err(at(path, io::Error::last_os_error()));
    }
    if buf.stx_mask & libc::STATX_MNT_ID != 0 {
        if let Some(mount) = table.iter().find(|m| m.id == buf.stx_mnt_id) {
            return Ok(Some(mount));
        }
    }
    Ok(table
        .iter()
        .filter(|m| path.starts_with(&m.mount_point))
        .max_by_key(|m| m.mount_point.components().count()))
}

/// Where the block device `device` is mounted, if anywhere: a mount whose
/// source is the same device.
pub fn mounted_at(table: &[Mount], device: &Path) -> io::Result<Option<PathBuf>> {
    let rdev = block_rdev(device)?;
    for mount in table.iter().filter(|m| m.source.starts_with('/')) {
        match block_rdev(Path::new(&mount.source)) {
            Ok(source) if source == rdev => return Ok(Some(mount.mount_point.clone())),
            Ok(_) => {}
            // A source that no longer exists (a removed loop device) or is no
            // block device is not this device.
            Err(err) if matches!(err.kind(), ErrorKind::NotFound | ErrorKind::InvalidInput) => {}
            Err(err) => return Err(err),
        }
    }
    Ok(None)
}

/// A block device's device number; `InvalidInput` when `path` is not one.
pub fn block_rdev(path: &Path) -> io::Result<u64> {
    let meta = fs::metadata(path).map_err(|err| at(path, err))?;
    if !meta.file_type().is_block_device() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("'{}' is not a block device", path.display()),
        ));
    }
    Ok(meta.rdev())
}

// ---- durable writes ----

/// Replace `path` so that after a crash or power loss it holds either the
/// old or the new content: write a sibling temp file (with `mode`, when
/// given), flush it, rename it over `path`, and flush the directory so the
/// rename persists. (On FAT, as on the ESP, the rename itself is not
/// guaranteed atomic.)
pub fn write_durable(path: &Path, content: &[u8], mode: Option<u32>) -> io::Result<()> {
    let parent = parent_of(path)?;
    fs::create_dir_all(parent).map_err(|err| at(parent, err))?;
    let tmp = temp_sibling(path);
    let written = (|| {
        let mut file = File::create(&tmp)?;
        file.write_all(content)?;
        if let Some(mode) = mode {
            file.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        file.sync_all()
    })();
    match written {
        Ok(()) => commit_temp(&tmp, path),
        Err(err) => Err(discard_temp(&tmp, at(&tmp, err))),
    }
}

/// Copy `source` over `dest` the same way as [`write_durable`].
pub fn copy_durable(source: &Path, dest: &Path) -> io::Result<()> {
    let tmp = temp_sibling(dest);
    match fs::copy(source, &tmp).and_then(|_| File::open(&tmp)?.sync_all()) {
        Ok(()) => commit_temp(&tmp, dest),
        Err(err) => Err(discard_temp(
            &tmp,
            io::Error::new(err.kind(), format!("copying '{}' to '{}': {err}", source.display(), tmp.display())),
        )),
    }
}

/// Flush a directory's entries. Filesystems that cannot fsync a directory
/// (EINVAL) persist renames on their own schedule; that is all they offer.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    match File::open(dir).and_then(|d| d.sync_all()) {
        Err(err) if err.raw_os_error() != Some(libc::EINVAL) => Err(at(dir, err)),
        _ => Ok(()),
    }
}

fn parent_of(path: &Path) -> io::Result<&Path> {
    path.parent().filter(|p| !p.as_os_str().is_empty()).ok_or_else(|| {
        io::Error::new(ErrorKind::InvalidInput, format!("'{}' has no parent directory", path.display()))
    })
}

fn temp_sibling(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    path.with_file_name(format!(".{name}.tmp-{}", std::process::id()))
}

fn commit_temp(tmp: &Path, path: &Path) -> io::Result<()> {
    if let Err(err) = fs::rename(tmp, path) {
        let err = io::Error::new(err.kind(), format!("renaming '{}' to '{}': {err}", tmp.display(), path.display()));
        return Err(discard_temp(tmp, err));
    }
    sync_dir(parent_of(path)?)
}

/// `err`, after removing the temp file; a temp file that cannot be removed
/// is named in the error.
fn discard_temp(tmp: &Path, err: io::Error) -> io::Error {
    match fs::remove_file(tmp) {
        Err(remove) if remove.kind() != ErrorKind::NotFound => io::Error::new(
            err.kind(),
            format!("{err} (and the temp file '{}' was left behind: {remove})", tmp.display()),
        ),
        _ => err,
    }
}

// ---- the rootfs image ----

/// The extension a rootfs image file carries.
pub const EROFS_EXTENSION: &str = "erofs";
/// Where an EROFS image's superblock starts, and its magic number there.
pub const EROFS_SUPERBLOCK_OFFSET: u64 = 1024;
pub const EROFS_MAGIC: u32 = 0xE0F5_E1E2;
/// Block sizes (as a bit shift) an EROFS superblock can declare.
const EROFS_BLOCK_SIZE_BITS: std::ops::RangeInclusive<u8> = 9..=16;

/// A rootfs image every tool accepts: a `.erofs` file, or a block device
/// holding the image.
#[derive(Debug, Clone)]
pub struct ErofsImage {
    pub path: PathBuf,
    /// The file's length, or the image size a partition's superblock declares.
    pub bytes: u64,
    /// The device number, when the image is a block device.
    pub rdev: Option<u64>,
}

/// Check `path` as a rootfs image. The outer error is an I/O failure; the
/// inner `Err` says why a readable `path` is not an EROFS image.
pub fn erofs_image(path: &Path) -> io::Result<Result<ErofsImage, String>> {
    let canonical = fs::canonicalize(path).map_err(|err| at(path, err))?;
    let meta = fs::metadata(&canonical).map_err(|err| at(&canonical, err))?;
    let block_device = meta.file_type().is_block_device();
    if !meta.is_file() && !block_device {
        return Ok(Err("not a file or block device".to_string()));
    }
    if meta.is_file() && canonical.extension() != Some(OsStr::new(EROFS_EXTENSION)) {
        return Ok(Err(format!("an image file must be named .{EROFS_EXTENSION}")));
    }
    let mut file = File::open(&canonical).map_err(|err| at(&canonical, err))?;
    file.seek(SeekFrom::Start(EROFS_SUPERBLOCK_OFFSET)).map_err(|err| at(&canonical, err))?;
    // magic (u32), checksum (u32), feature_compat (u32), blkszbits (u8),
    // sb_extslots (u8), root nid (u16), inos (u64), build time (u64, u32),
    // blocks (u32): all little-endian.
    let mut superblock = [0u8; 40];
    match file.read_exact(&mut superblock) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::UnexpectedEof => {
            return Ok(Err("too short to hold an EROFS superblock".to_string()))
        }
        Err(err) => return Err(at(&canonical, err)),
    }
    let u32_at = |at: usize| u32::from_le_bytes([superblock[at], superblock[at + 1], superblock[at + 2], superblock[at + 3]]);
    let magic = u32_at(0);
    if magic != EROFS_MAGIC {
        return Ok(Err(format!(
            "no EROFS superblock (magic 0x{magic:08x} at byte {EROFS_SUPERBLOCK_OFFSET}, expected 0x{EROFS_MAGIC:08x})"
        )));
    }
    let block_bits = superblock[12];
    if !EROFS_BLOCK_SIZE_BITS.contains(&block_bits) {
        return Ok(Err(format!("the EROFS superblock declares an invalid block size (2^{block_bits})")));
    }
    Ok(Ok(ErofsImage {
        bytes: if block_device { u64::from(u32_at(36)) << block_bits } else { meta.len() },
        rdev: block_device.then(|| meta.rdev()),
        path: canonical,
    }))
}
