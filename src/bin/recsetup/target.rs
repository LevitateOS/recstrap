//! What the target root is and what it offers: its distro profile (read from
//! its os-release, never guessed from the files it ships), its admin setup,
//! and the time zones, locales, keymaps, shells, groups and services there are
//! to choose from. Everything is read from files under the root, so `/mnt` and
//! a scratch directory are inspected the same way, and nothing here changes
//! the target.

use distro_profile::{os_release_value, Flavour, InitSystem, Libc, PrivilegeTool, Profile, ADMIN_GROUP};
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
        self.walk(target_path)
    }

    fn walk(&self, target_path: &str) -> Result<PathBuf> {
        let mut queue: VecDeque<OsString> = components(Path::new(target_path));
        let mut current: Vec<OsString> = Vec::new();
        let mut hops = 0;
        while let Some(part) = queue.pop_front() {
            if part == ".." {
                current.pop();
                continue;
            }
            let mut candidate = self.path.clone();
            candidate.extend(&current);
            candidate.push(&part);
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

    /// The file's text; `None` only when it does not exist.
    pub fn read(&self, target_path: &str) -> Result<Option<String>> {
        Ok(target_root::read_optional(&self.resolve(target_path)?)?)
    }

    /// The names in a directory of the target, sorted; empty only when it is missing.
    pub fn list(&self, target_path: &str) -> Result<Vec<String>> {
        Ok(target_root::dir_names(&self.resolve(target_path)?)?
            .into_iter()
            .map(|name| name.to_string_lossy().into_owned())
            .collect())
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
    /// Which system it is: the base system, or the desktop system (whose users
    /// get the desktop session's groups, an X11 keyboard layout and can log in
    /// by themselves).
    pub flavour: Flavour,
    pub admin: Admin,
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
    let os_release = target_root::os_release(root.path())?
        .ok_or_else(|| no_profile("no etc/os-release or usr/lib/os-release".to_string()))?;
    let profile = Profile::from_os_release(&os_release).map_err(|e| no_profile(e.to_string()))?;
    let flavour = Flavour::from_os_release(&os_release).map_err(|e| no_profile(e.to_string()))?;
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
            .read(tool.admin_rule_file())?
            .is_some_and(|text| text.lines().any(|line| line.trim() == rule)),
    };

    let default_shell = target_root::root_shell(&root.read("etc/passwd")?.unwrap_or_default());

    let top = fs::metadata(root.path()).map_err(|e| Error::path_unresolvable(&root.path().to_string_lossy(), e))?;
    let boot = root.resolve("boot")?;
    let boot_is_mountpoint = match fs::metadata(&boot) {
        Ok(boot) => top.dev() != boot.dev(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(Error::path_unresolvable(&boot.to_string_lossy(), e)),
    };

    Ok(Target {
        root: root.path().display().to_string(),
        os_id,
        os_name,
        profile,
        flavour,
        admin,
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

pub fn choices(root: &Root, target: &Target) -> Result<Choices> {
    Ok(Choices {
        timezones: timezones(root),
        locales: locales(root, target.profile.libc)?,
        keymaps: keymaps(root, target.profile.init),
        shells: shells(root, target)?,
        groups: account_names(root, "etc/group")?,
        users: account_names(root, "etc/passwd")?,
        services: services(root, target.profile.init)?,
    })
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
pub fn locales(root: &Root, libc: Libc) -> Result<Vec<LocaleChoice>> {
    let mut found = BTreeSet::new();
    match libc {
        Libc::Musl => {
            found.insert("C.UTF-8".to_string());
            for name in root.list("usr/share/i18n/locales/musl")? {
                if is_locale_name(&name) {
                    found.insert(name);
                }
            }
        }
        Libc::Glibc => {
            for name in root.list("usr/lib/locale")? {
                if root.is_file(&format!("usr/lib/locale/{name}/LC_CTYPE")) {
                    found.insert(normalize_locale(&name));
                }
            }
        }
    }
    Ok(found.into_iter().map(|name| LocaleChoice { name }).collect())
}

/// Where the init system's keymap loader takes its keymaps from: systemd's
/// vconsole setup and OpenRC's `loadkeys` service both load kbd's. The system
/// setup program configures the same loader.
fn keymap_dirs(init: InitSystem) -> &'static [&'static str] {
    match init {
        InitSystem::Systemd => &["usr/lib/kbd/keymaps", "usr/share/kbd/keymaps"],
        InitSystem::Openrc => &["usr/share/keymaps"],
    }
}

/// The program that loads the configured keymap at boot, relative to the root.
pub fn keymap_loader(init: InitSystem) -> &'static str {
    match init {
        InitSystem::Systemd => "usr/lib/systemd/systemd-vconsole-setup",
        InitSystem::Openrc => "etc/init.d/loadkeys",
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
            .strip_suffix(".map.gz")
            .or_else(|| name.strip_suffix(".map"))
        {
            out.insert(stem.to_string());
        }
    }
}

pub fn shells(root: &Root, target: &Target) -> Result<Vec<String>> {
    let mut shells: BTreeSet<String> = root
        .read("etc/shells")?
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('/') && root.is_file(l))
        .map(str::to_string)
        .collect();
    shells.insert(target.default_shell.clone());
    Ok(shells.into_iter().collect())
}

pub fn account_names(root: &Root, file: &str) -> Result<Vec<String>> {
    Ok(target_root::account_names(&root.read(file)?.unwrap_or_default()))
}

/// The services that can be started at boot, by their bare names (`sshd`, not
/// `sshd.service`) on every init system. systemd offers the units it can
/// enable (an `[Install]` section, no templates), OpenRC every init script.
pub fn services(root: &Root, init: InitSystem) -> Result<Vec<ServiceChoice>> {
    let mut services: BTreeMap<String, Vec<String>> = BTreeMap::new();
    match init {
        InitSystem::Systemd => {
            for dir in ["usr/lib/systemd/system", "etc/systemd/system"] {
                for unit in root.list(dir)? {
                    let Some(name) = unit.strip_suffix(".service") else { continue };
                    if name.ends_with('@') {
                        continue;
                    }
                    let text = root.read(&format!("{dir}/{unit}"))?.unwrap_or_default();
                    if text.lines().any(|l| l.trim() == "[Install]") {
                        services.entry(name.to_string()).or_default();
                    }
                }
            }
            for wants in root.list("etc/systemd/system")? {
                let Some(target) = wants.strip_suffix(".wants") else { continue };
                for unit in root.list(&format!("etc/systemd/system/{wants}"))? {
                    let name = unit.strip_suffix(".service").unwrap_or(&unit);
                    if let Some(enabled) = services.get_mut(name) {
                        enabled.push(target.to_string());
                    }
                }
            }
            // The desktop system starts its display manager through the
            // display-manager.service alias, which the default target wants.
            let link = |path: &str| {
                fs::read_link(root.path().join(path)).ok().and_then(|to| to.file_name().map(|n| n.to_string_lossy().into_owned()))
            };
            if let Some(unit) = link("etc/systemd/system/display-manager.service") {
                let default = link("etc/systemd/system/default.target").unwrap_or_else(|| "graphical.target".to_string());
                if let Some(enabled) = unit.strip_suffix(".service").and_then(|name| services.get_mut(name)) {
                    enabled.push(default);
                }
            }
        }
        InitSystem::Openrc => {
            for name in root.list("etc/init.d")? {
                if root.is_file(&format!("etc/init.d/{name}")) && !name.starts_with('.') && name != "functions.sh" {
                    services.entry(name).or_default();
                }
            }
            for level in root.list("etc/runlevels")? {
                for name in root.list(&format!("etc/runlevels/{level}"))? {
                    if let Some(enabled) = services.get_mut(&name) {
                        enabled.push(level.clone());
                    }
                }
            }
        }
    }
    Ok(services
        .into_iter()
        .map(|(name, enabled_in)| ServiceChoice { name, enabled_in })
        .collect())
}

/// The X11 keyboard layout (and variant) that matches the console keymap
/// `keymap` in the target, for a desktop system's display server, login
/// screen and session; `None` when none matches. systemd's own table of them
/// (`kbd-model-map`) decides where the target has it; otherwise the keymap's
/// name, a few known renames and its base before the first `-`, whichever is
/// a layout of the target's XKB rules.
pub fn x11_layout(root: &Root, keymap: &str) -> Result<Option<(String, Option<String>)>> {
    if let Some(map) = root.read("usr/share/systemd/kbd-model-map")? {
        for line in map.lines().filter(|line| !line.trim_start().starts_with('#')) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() >= 4 && fields[0] == keymap {
                let variant = (fields[3] != "-").then(|| fields[3].to_string());
                return Ok(Some((fields[1].to_string(), variant)));
            }
        }
    }
    let mut rules = None;
    for path in ["usr/share/X11/xkb/rules/base.lst", "usr/share/xkeyboard-config-2/rules/base.lst"] {
        if rules.is_none() {
            rules = root.read(path)?;
        }
    }
    let Some(rules) = rules else { return Ok(None) };
    let mut section = "";
    let mut layouts = Vec::new();
    let mut variants = Vec::new();
    for line in rules.lines() {
        if let Some(name) = line.strip_prefix("! ") {
            section = name.trim();
            continue;
        }
        let mut words = line.split_whitespace();
        match (section, words.next(), words.next()) {
            ("layout", Some(layout), _) => layouts.push(layout.to_string()),
            ("variant", Some(variant), Some(layout)) => {
                variants.push((layout.trim_end_matches(':').to_string(), variant.to_string()))
            }
            _ => {}
        }
    }
    const RENAMES: &[(&str, &str, Option<&str>)] = &[
        ("uk", "gb", None),
        ("sv-latin1", "se", None),
        ("jp106", "jp", None),
        ("slovene", "si", None),
        ("croat", "hr", None),
        ("et", "ee", None),
        ("trq", "tr", None),
        ("dvorak", "us", Some("dvorak")),
        ("colemak", "us", Some("colemak")),
    ];
    let mut candidates: Vec<(String, Option<String>)> = Vec::new();
    if let Some((_, layout, variant)) = RENAMES.iter().find(|(name, _, _)| *name == keymap) {
        candidates.push((layout.to_string(), variant.map(str::to_string)));
    }
    candidates.push((keymap.to_string(), None));
    if let Some((base, _)) = keymap.split_once('-') {
        candidates.push((base.to_string(), None));
    }
    let Some((layout, mut variant)) = candidates.into_iter().find(|(layout, _)| layouts.contains(layout)) else {
        return Ok(None);
    };
    if variant.is_none() && keymap.contains("nodeadkeys") && variants.contains(&(layout.clone(), "nodeadkeys".to_string())) {
        variant = Some("nodeadkeys".to_string());
    }
    Ok(Some((layout, variant)))
}
