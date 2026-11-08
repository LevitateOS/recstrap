//! What the target root is and what it offers: its distro profile (read from
//! its os-release, never guessed from the files it ships), its admin setup,
//! and the time zones, locales, keymaps, shells, groups and services there are
//! to choose from. Everything is read from files under the root, so `/mnt` and
//! a scratch directory are inspected the same way, and nothing here changes
//! the target.

use distro_profile::{os_release_value, InitSystem, Libc, PrivilegeTool, Profile, ADMIN_GROUP};
use install_tools::error::{Code, Error, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

/// Symlinks followed while resolving one path before it counts as a loop.
const MAX_LINK_HOPS: u32 = 40;

/// The target root. Paths in the target ("/etc/hostname") are resolved the
/// way the target sees them after a chroot: a symlink, absolute or
/// relative, never leads outside the root.
#[derive(Debug, Clone)]
pub struct Root {
    path: PathBuf,
}

impl Root {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How a target path shows on the host, for the printed commands.
    pub fn display(&self, target_path: &str) -> String {
        format!("{}/{}", self.path.display(), target_path.trim_start_matches('/'))
    }

    /// The host path of `target_path`, all symlinks resolved inside the root.
    pub fn resolve(&self, target_path: &str) -> Result<PathBuf> {
        self.walk(target_path, true)
    }

    /// The host path of `target_path` with its parent directories resolved
    /// but its last component left as it is (a symlink there is not
    /// followed), for creating, replacing or removing that entry.
    pub fn resolve_entry(&self, target_path: &str) -> Result<PathBuf> {
        self.walk(target_path, false)
    }

    fn walk(&self, target_path: &str, follow_last: bool) -> Result<PathBuf> {
        let mut queue: VecDeque<OsString> = components(Path::new(target_path));
        let mut current: Vec<OsString> = Vec::new();
        let mut hops = 0;
        while let Some(part) = queue.pop_front() {
            if part == ".." {
                current.pop();
                continue;
            }
            let is_last = queue.is_empty();
            let mut candidate = self.path.clone();
            candidate.extend(&current);
            candidate.push(&part);
            if is_last && !follow_last {
                current.push(part);
                break;
            }
            match fs::symlink_metadata(&candidate) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    hops += 1;
                    if hops > MAX_LINK_HOPS {
                        return Err(Error::new(
                            Code::UnsafeTargetPath,
                            format!("'{}' has a symlink loop", self.display(target_path)),
                        )
                        .fix("remove or repair the looping symlink in the target"));
                    }
                    let link = fs::read_link(&candidate)
                        .map_err(|e| Error::path_unresolvable(&candidate.to_string_lossy(), e))?;
                    if link.is_absolute() {
                        current.clear();
                    }
                    for (i, piece) in components(&link).into_iter().enumerate() {
                        queue.insert(i, piece);
                    }
                }
                _ => current.push(part),
            }
        }
        let mut out = self.path.clone();
        out.extend(&current);
        Ok(out)
    }

    pub fn is_file(&self, target_path: &str) -> bool {
        self.resolve(target_path).is_ok_and(|p| p.is_file())
    }

    pub fn is_dir(&self, target_path: &str) -> bool {
        self.resolve(target_path).is_ok_and(|p| p.is_dir())
    }

    /// The file's text, or `None` when it does not exist or cannot be read.
    pub fn read(&self, target_path: &str) -> Option<String> {
        fs::read_to_string(self.resolve(target_path).ok()?).ok()
    }

    /// The names in a directory of the target, sorted; empty when it is missing.
    pub fn list(&self, target_path: &str) -> Vec<String> {
        let Ok(dir) = self.resolve(target_path) else {
            return Vec::new();
        };
        let mut names: Vec<String> = fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// A program the target can run, looked up in its usual bin directories.
    pub fn has_program(&self, name: &str) -> bool {
        ["usr/bin", "usr/sbin", "bin", "sbin"]
            .iter()
            .any(|dir| self.is_file(&format!("{dir}/{name}")))
    }
}

fn components(path: &Path) -> VecDeque<OsString> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_os_string()),
            Component::ParentDir => Some(OsString::from("..")),
            _ => None,
        })
        .collect()
}

/// How systemd-boot gets onto the ESP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BootloaderMethod {
    /// `bootctl install` in the target (systemd systems).
    Bootctl,
    /// Copy the EFI binary into place (no bootctl without systemd).
    Copy,
}

/// The administrator setup: members of `group` get root through `tool`.
#[derive(Debug, Clone, Serialize)]
pub struct Admin {
    pub group: String,
    pub tool: PrivilegeTool,
    /// The file the system recipe writes the admin rule into.
    pub rule_file: String,
    /// The rule is there, so nothing needs writing.
    pub rule_present: bool,
}

/// The facts about the target that decide which commands set it up.
#[derive(Debug, Clone, Serialize)]
pub struct Target {
    pub root: String,
    pub os_id: String,
    pub os_name: String,
    /// The distro profile from the target's os-release.
    pub profile: Profile,
    pub admin: Admin,
    pub bootloader: Option<BootloaderMethod>,
    /// systemd-boot's EFI binary in the target, when it has one.
    pub systemd_boot_efi: Option<String>,
    /// The time zone database is installed (without it only UTC exists).
    pub has_zoneinfo: bool,
    /// The shell of new users: root's login shell in the target.
    pub default_shell: String,
    /// The ESP is mounted at the target's `/boot`.
    pub boot_is_mountpoint: bool,
}

const ZONEINFO: &str = "usr/share/zoneinfo";

pub fn detect(root: &Root) -> Result<Target> {
    let no_profile = |detail: String| {
        Error::new(Code::NoProfile, format!("'{}': {detail}", root.path().display())).fix(
            "extract a system built by the current builder (its /etc/os-release carries the REC_* fields), and point --root at its top directory",
        )
    };
    let os_release = install_tools::target_system::os_release(root.path())
        .ok_or_else(|| no_profile("no etc/os-release or usr/lib/os-release".to_string()))?;
    let profile = Profile::from_os_release(&os_release).map_err(|e| no_profile(e.to_string()))?;
    let os_id = os_release_value(&os_release, "ID")
        .unwrap_or_else(|| "linux".to_string())
        .to_ascii_lowercase();
    let os_name = os_release_value(&os_release, "NAME").unwrap_or_else(|| os_id.clone());

    let tool = profile.privilege;
    let rule = tool.admin_rule();
    let admin = Admin {
        group: ADMIN_GROUP.to_string(),
        tool,
        rule_file: format!("/{}", tool.admin_rule_file()),
        rule_present: root
            .read(tool.admin_rule_file())
            .is_some_and(|text| text.lines().any(|line| line.trim() == rule)),
    };

    let efi_name = match std::env::consts::ARCH {
        "aarch64" => "systemd-bootaa64.efi",
        _ => "systemd-bootx64.efi",
    };
    let systemd_boot_efi = Some(format!("usr/lib/systemd/boot/efi/{efi_name}")).filter(|p| root.is_file(p));
    let bootloader = if profile.init == InitSystem::Systemd && root.has_program("bootctl") {
        Some(BootloaderMethod::Bootctl)
    } else if systemd_boot_efi.is_some() {
        Some(BootloaderMethod::Copy)
    } else {
        None
    };

    let default_shell = install_tools::target_system::root_shell(&root.read("etc/passwd").unwrap_or_default());

    let boot_is_mountpoint = match (fs::metadata(root.path()), root.resolve("boot").and_then(|b| fs::metadata(&b).map_err(|e| Error::path_unresolvable("boot", e)))) {
        (Ok(top), Ok(boot)) => top.dev() != boot.dev(),
        _ => false,
    };

    Ok(Target {
        root: root.path().display().to_string(),
        os_id,
        os_name,
        profile,
        admin,
        bootloader,
        systemd_boot_efi: systemd_boot_efi.map(|p| format!("/{p}")),
        has_zoneinfo: root.is_dir(ZONEINFO),
        default_shell,
        boot_is_mountpoint,
    })
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimezoneChoice {
    pub name: String,
    /// The part before the first `/` ("Europe"); empty for top-level zones such as UTC.
    pub region: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LocaleChoice {
    /// The `LANG` value, such as `en_US.UTF-8`.
    pub name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServiceChoice {
    pub name: String,
    /// The runlevels (OpenRC) or `.wants` targets (systemd) that already start it.
    pub enabled_in: Vec<String>,
}

/// Everything the target offers to choose from.
#[derive(Debug, Clone, Serialize)]
pub struct Choices {
    pub timezones: Vec<TimezoneChoice>,
    /// Usable as they are: recsetup compiles no locales.
    pub locales: Vec<LocaleChoice>,
    pub keymaps: Vec<String>,
    pub shells: Vec<String>,
    pub groups: Vec<String>,
    pub users: Vec<String>,
    pub services: Vec<ServiceChoice>,
}

pub fn choices(root: &Root, target: &Target) -> Choices {
    Choices {
        timezones: timezones(root),
        locales: locales(root, target.profile.libc),
        keymaps: keymaps(root, target.profile.init),
        shells: shells(root, target),
        groups: account_names(root, "etc/group"),
        users: account_names(root, "etc/passwd"),
        services: services(root, target.profile.init),
    }
}

pub fn timezones(root: &Root) -> Vec<TimezoneChoice> {
    let mut zones = BTreeSet::new();
    zones.insert(TimezoneChoice { name: "UTC".to_string(), region: String::new() });
    if let Ok(base) = root.resolve(ZONEINFO) {
        collect_zones(&base, "", &mut zones);
    }
    zones.into_iter().collect()
}

fn collect_zones(dir: &Path, prefix: &str, out: &mut BTreeSet<TimezoneChoice>) {
    // The `posix/` and `right/` trees repeat every zone; `Factory` is no place.
    const SKIP: &[&str] = &["posix", "right", "posixrules", "localtime", "Factory"];
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if prefix.is_empty() && SKIP.contains(&name.as_str()) {
            continue;
        }
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let path = entry.path();
        // Zones inside zoneinfo are files or links to other zones in it;
        // a link that leaves the tree is skipped rather than followed.
        let Ok(meta) = fs::metadata(&path) else { continue };
        if meta.is_dir() {
            if !entry.file_type().is_ok_and(|t| t.is_symlink()) {
                collect_zones(&path, &rel, out);
            }
        } else if meta.is_file() && is_tzif(&path) {
            let region = rel.split_once('/').map(|(r, _)| r.to_string()).unwrap_or_default();
            out.insert(TimezoneChoice { name: rel, region });
        }
    }
}

fn is_tzif(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && &magic == b"TZif"
}

/// `en_US.utf8` (glibc's directory name) as the usual `LANG` spelling.
fn normalize_locale(name: &str) -> String {
    match name.split_once('.') {
        Some((base, codeset)) => {
            let (codeset, modifier) = match codeset.split_once('@') {
                Some((c, m)) => (c, format!("@{m}")),
                None => (codeset, String::new()),
            };
            let codeset = match codeset.to_ascii_lowercase().as_str() {
                "utf8" | "utf-8" => "UTF-8".to_string(),
                _ => codeset.to_string(),
            };
            format!("{base}.{codeset}{modifier}")
        }
        None => name.to_string(),
    }
}

fn is_locale_name(name: &str) -> bool {
    let base = name.split(['.', '@']).next().unwrap_or("");
    !base.is_empty()
        && base.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && (base == "C" || base.contains('_') || base.len() <= 3)
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "._-@".contains(c))
}

/// Every locale the target can use as it is: glibc's compiled locales, or
/// musl's (musl compiles nothing; its locale files only translate messages).
pub fn locales(root: &Root, libc: Libc) -> Vec<LocaleChoice> {
    let mut found = BTreeSet::new();
    match libc {
        Libc::Musl => {
            found.insert("C.UTF-8".to_string());
            for name in root.list("usr/share/i18n/locales/musl") {
                if is_locale_name(&name) {
                    found.insert(name);
                }
            }
        }
        Libc::Glibc => {
            for name in root.list("usr/lib/locale") {
                if root.is_file(&format!("usr/lib/locale/{name}/LC_CTYPE")) {
                    found.insert(normalize_locale(&name));
                }
            }
        }
    }
    found.into_iter().map(|name| LocaleChoice { name }).collect()
}

/// Where the init system's keymap loader takes its keymaps from: systemd's
/// vconsole setup loads kbd's, busybox's `loadkmap` service (OpenRC) the binary
/// ones of kbd-bkeymaps. The system setup program configures the same loader.
fn keymap_dirs(init: InitSystem) -> &'static [&'static str] {
    match init {
        InitSystem::Systemd => &["usr/lib/kbd/keymaps", "usr/share/kbd/keymaps"],
        InitSystem::Openrc => &["usr/share/bkeymaps"],
    }
}

/// The program that loads the configured keymap at boot, relative to the root.
pub fn keymap_loader(init: InitSystem) -> &'static str {
    match init {
        InitSystem::Systemd => "usr/lib/systemd/systemd-vconsole-setup",
        InitSystem::Openrc => "etc/init.d/loadkmap",
    }
}

pub fn keymaps(root: &Root, init: InitSystem) -> Vec<String> {
    let mut names = BTreeSet::new();
    for dir in keymap_dirs(init) {
        if let Ok(base) = root.resolve(dir) {
            collect_keymaps(&base, &mut names);
        }
    }
    names.into_iter().collect()
}

fn collect_keymaps(dir: &Path, out: &mut BTreeSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            if name != "include" {
                collect_keymaps(&entry.path(), out);
            }
        } else if let Some(stem) = name
            .strip_suffix(".bmap.gz")
            .or_else(|| name.strip_suffix(".map.gz"))
            .or_else(|| name.strip_suffix(".map"))
        {
            out.insert(stem.to_string());
        }
    }
}

pub fn shells(root: &Root, target: &Target) -> Vec<String> {
    let mut shells: BTreeSet<String> = root
        .read("etc/shells")
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('/') && root.is_file(l))
        .map(str::to_string)
        .collect();
    shells.insert(target.default_shell.clone());
    shells.into_iter().collect()
}

pub fn account_names(root: &Root, file: &str) -> Vec<String> {
    root.read(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split(':').next())
        .filter(|n| !n.is_empty() && !n.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// The services that can be started at boot, by their bare names (`sshd`, not
/// `sshd.service`) on every init system. systemd offers the units it can
/// enable (an `[Install]` section, no templates), OpenRC every init script.
pub fn services(root: &Root, init: InitSystem) -> Vec<ServiceChoice> {
    let mut services: BTreeMap<String, Vec<String>> = BTreeMap::new();
    match init {
        InitSystem::Systemd => {
            for dir in ["usr/lib/systemd/system", "etc/systemd/system"] {
                for unit in root.list(dir) {
                    let Some(name) = unit.strip_suffix(".service") else { continue };
                    if name.ends_with('@') {
                        continue;
                    }
                    let text = root.read(&format!("{dir}/{unit}")).unwrap_or_default();
                    if text.lines().any(|l| l.trim() == "[Install]") {
                        services.entry(name.to_string()).or_default();
                    }
                }
            }
            for wants in root.list("etc/systemd/system") {
                let Some(target) = wants.strip_suffix(".wants") else { continue };
                for unit in root.list(&format!("etc/systemd/system/{wants}")) {
                    let name = unit.strip_suffix(".service").unwrap_or(&unit);
                    if let Some(enabled) = services.get_mut(name) {
                        enabled.push(target.to_string());
                    }
                }
            }
        }
        InitSystem::Openrc => {
            for name in root.list("etc/init.d") {
                if root.is_file(&format!("etc/init.d/{name}")) && !name.starts_with('.') && name != "functions.sh" {
                    services.entry(name).or_default();
                }
            }
            for level in root.list("etc/runlevels") {
                for name in root.list(&format!("etc/runlevels/{level}")) {
                    if let Some(enabled) = services.get_mut(&name) {
                        enabled.push(level.clone());
                    }
                }
            }
        }
    }
    services
        .into_iter()
        .map(|(name, enabled_in)| ServiceChoice { name, enabled_in })
        .collect()
}
