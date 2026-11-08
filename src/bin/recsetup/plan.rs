//! The plan: a request, checked against the target, as the ordered steps
//! apply runs. Each step carries the exact command it stands for, so the
//! plan is also the manual install for anyone who would rather type it.

use crate::request::{check_hostname, check_username, invalid, SetupRequest};
use crate::target::{self, Root, Target};
use distro_profile::InitSystem;
use install_tools::error::{Code, Error, Result};
use target_root::mount_table;
use recinit::live::{
    INITRAMFS_INSTALLED_ISO_PATH, INITRAMFS_LIVE_ISO_PATH, KERNEL_ISO_PATH, LIVE_MEDIA_DIR,
};
use recpart::types::InstallMode;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const PLAN_SCHEMA_VERSION: u32 = 1;
const SETUP_PROGRAM: &str = "/usr/libexec/recipe-system-setup";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Timezone,
    Locale,
    Keymap,
    Hostname,
    Users,
    Passwords,
    /// systemd-boot, the boot entries and the installed kernel (recab init).
    Boot,
    MachineId,
    Services,
}

/// What a step does.
#[derive(Debug, Clone)]
pub enum Action {
    /// Run in the target through recchroot; `passwords` feeds it the
    /// `name:password` lines read from stdin.
    Chroot { argv: Vec<String>, passwords: bool },
    /// Run on the live system.
    Host { argv: Vec<String> },
}

#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub index: usize,
    pub section: Section,
    pub description: String,
    /// The shell command this step is.
    pub command: String,
    #[serde(skip)]
    pub action: Action,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub schema_version: u32,
    pub target: Target,
    pub request: SetupRequest,
    pub steps: Vec<Step>,
    /// Things that do not stop the plan but are worth a look first.
    pub warnings: Vec<String>,
    /// The accounts whose passwords apply reads from stdin, in order.
    pub password_users: Vec<String>,
    #[serde(skip)]
    pub root: Root,
}

/// Quote one word for a POSIX shell, leaving plain words bare.
pub fn shell_quote(word: &str) -> String {
    if !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c))
    {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\"'\"'"))
    }
}

fn shell_line(argv: &[String]) -> String {
    argv.iter()
        .map(|w| shell_quote(w))
        .collect::<Vec<_>>()
        .join(" ")
}

struct Builder<'a> {
    root: &'a Root,
    steps: Vec<Step>,
}

impl Builder<'_> {
    fn push(&mut self, section: Section, description: impl Into<String>, action: Action) {
        let root = self.root;
        let command = match &action {
            Action::Chroot { argv, .. } => format!(
                "recchroot {} -- {}",
                shell_quote(&root.path().to_string_lossy()),
                shell_line(argv)
            ),
            Action::Host { argv } => shell_line(argv),
        };
        self.steps.push(Step {
            index: self.steps.len() + 1,
            section,
            description: description.into(),
            command,
            action,
        });
    }

    fn setup(&mut self, section: Section, description: impl Into<String>, args: &[&str]) {
        let mut argv = vec![SETUP_PROGRAM.to_string()];
        argv.extend(args.iter().map(|arg| arg.to_string()));
        self.push(
            section,
            description,
            Action::Chroot {
                argv,
                passwords: false,
            },
        );
    }
}

fn unavailable(message: impl Into<String>, fix: impl Into<String>) -> Error {
    Error::new(Code::ChoiceUnavailable, message).fix(fix)
}

pub fn build(root: Root, target: Target, request: SetupRequest) -> Result<Plan> {
    let mut b = Builder {
        root: &root,
        steps: Vec::new(),
    };
    let mut warnings = Vec::new();
    let r = root.path().display().to_string();
    let inspect_hint =
        |what: &str| format!("choose one that `recsetup inspect --root {r}` lists under {what}");
    let personalization = request.timezone.is_some()
        || request.locale.is_some()
        || request.keymap.is_some()
        || request.hostname.is_some()
        || !request.users.is_empty()
        || request.machine_id
        || !request.services.is_empty();
    if personalization && !root.is_file(SETUP_PROGRAM) {
        return Err(unavailable(
            "the target lacks Recipe's system setup program",
            "deploy a complete system produced by its current system recipe",
        ));
    }
    if let Some(tz) = &request.timezone {
        if !target::timezones(&root).iter().any(|z| &z.name == tz) {
            return Err(unavailable(
                format!("time zone '{tz}' is not in the target's zoneinfo"),
                inspect_hint("timezones"),
            ));
        }
        b.setup(
            Section::Timezone,
            format!("Set the time zone to {tz}"),
            &["timezone", tz],
        );
    }
    if let Some(locale) = &request.locale {
        if !target::locales(&root, target.profile.libc)?.iter().any(|c| &c.name == locale) {
            return Err(unavailable(
                format!("locale '{locale}' is not available in the target"),
                inspect_hint("locales"),
            ));
        }
        b.setup(
            Section::Locale,
            format!("Set the system locale to {locale}"),
            &["locale", locale],
        );
    }
    if let Some(keymap) = &request.keymap {
        let init = target.profile.init;
        if !target::keymaps(&root, init).contains(keymap) {
            return Err(unavailable(
                format!("keymap '{keymap}' is not in the target"),
                inspect_hint("keymaps"),
            ));
        }
        let loader = target::keymap_loader(init);
        if !root.is_file(loader) {
            warnings.push(format!(
                "the target has no /{loader}, so nothing loads the keymap at boot"
            ));
        }
        b.setup(
            Section::Keymap,
            format!("Set the console keymap to {keymap}"),
            &["keymap", keymap],
        );
    }
    if let Some(hostname) = &request.hostname {
        check_hostname(hostname)?;
        b.setup(
            Section::Hostname,
            format!("Set the hostname and hosts mapping to {hostname}"),
            &["hostname", hostname],
        );
    }

    let mut password_users = Vec::new();
    if request.root_password {
        password_users.push("root".to_string());
    }
    if !request.users.is_empty() {
        let groups = target::account_names(&root, "etc/group")?;
        let existing_users = target::account_names(&root, "etc/passwd")?;
        let mut seen = Vec::new();
        for user in &request.users {
            check_username(&user.name)?;
            if seen.contains(&user.name) {
                return Err(invalid(
                    format!("user '{}' is listed twice", user.name),
                    "list each user once",
                ));
            }
            seen.push(user.name.clone());
            let shell = user
                .shell
                .clone()
                .unwrap_or_else(|| target.default_shell.clone());
            if !shell.starts_with('/') || !root.is_file(&shell) {
                return Err(unavailable(
                    format!("shell '{shell}' is not in the target"),
                    inspect_hint("shells"),
                ));
            }
            let mut user_groups = Vec::new();
            if user.admin {
                let admin = &target.admin;
                user_groups.push(admin.group.clone());
                if !root.has_program(admin.tool.as_str()) {
                    warnings.push(format!(
                        "the target's profile names {}, which it does not ship; the admin group cannot become root",
                        admin.tool
                    ));
                } else if !admin.rule_present {
                    return Err(unavailable(
                        format!(
                            "the target lacks its recipe-owned administrator rule ('{}' in {})",
                            admin.tool.admin_rule(),
                            admin.rule_file
                        ),
                        "fix the system recipe; recsetup does not supply missing package policy",
                    ));
                }
            }
            for g in &user.groups {
                if !user_groups.contains(g) {
                    user_groups.push(g.clone());
                }
            }
            for g in &user_groups {
                if !groups.contains(g) {
                    return Err(unavailable(
                        format!("group '{g}' does not exist in the target"),
                        inspect_hint("groups"),
                    ));
                }
            }
            if user.password {
                password_users.push(user.name.clone());
            }
            if existing_users.contains(&user.name) {
                warnings.push(format!(
                    "user '{}' already exists in the target; it is left as it is",
                    user.name
                ));
                continue;
            }
            b.setup(
                Section::Users,
                format!("Create user {}", user.name),
                &["user", &user.name, &shell, &user_groups.join(",")],
            );
        }
    }
    if !password_users.is_empty() {
        if !root.has_program("chpasswd") {
            return Err(unavailable(
                "the target has no chpasswd",
                "deploy a complete system first",
            ));
        }
        b.push(
            Section::Passwords,
            format!(
                "Set the password of {} (read from stdin)",
                password_users.join(", ")
            ),
            Action::Chroot {
                argv: vec!["chpasswd".into()],
                passwords: true,
            },
        );
    }

    if let Some(boot) = &request.boot {
        let live = |path: &str| PathBuf::from(format!("{LIVE_MEDIA_DIR}/{path}"));
        let kernel = boot.kernel.clone().unwrap_or_else(|| live(KERNEL_ISO_PATH));
        // The live medium's own initramfs (boot/initramfs.img) only finds the live
        // medium; the installed system boots through the one built from its rootfs,
        // which the ISO carries beside it.
        let live_initramfs = live(INITRAMFS_LIVE_ISO_PATH);
        let initrd = match &boot.initrd {
            Some(path) if path.as_os_str().is_empty() => None,
            Some(path) => Some(path.clone()),
            None if target.profile.init == InitSystem::Systemd => Some(live(INITRAMFS_INSTALLED_ISO_PATH)),
            None => None,
        };
        if let Some(initrd) = &initrd {
            // A path that does not exist is compared as it is written.
            let canonical = |path: &Path| match path.canonicalize() {
                Ok(path) => Ok(path),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(path.to_path_buf()),
                Err(e) => Err(Error::path_unresolvable(&path.to_string_lossy(), e)),
            };
            if canonical(initrd)? == canonical(&live_initramfs)? {
                return Err(invalid(
                    format!(
                        "'{}' is the live medium's initramfs, which looks for the live medium and ignores root=: the installed system would not boot from its disk",
                        initrd.display()
                    ),
                    format!(
                        "pass the installed system's initramfs ({}), or leave --initrd out to use it",
                        live(INITRAMFS_INSTALLED_ISO_PATH).display()
                    ),
                ));
            }
        }
        let missing = |what: &str, path: &Path| {
            Error::new(
                Code::BootFileMissing,
                format!("{what} '{}' does not exist", path.display()),
            )
            .fix(format!(
                "pass the {what}'s path with --{}",
                what.split(' ').next().unwrap_or(what)
            ))
        };
        if !kernel.is_file() {
            return Err(missing("kernel", &kernel));
        }
        if let Some(initrd) = &initrd {
            if !initrd.is_file() {
                return Err(if boot.initrd.is_none() {
                    Error::new(
                        Code::BootFileMissing,
                        format!(
                            "this live medium has no installed-system initramfs ('{}')",
                            initrd.display()
                        ),
                    )
                    .fix(format!("boot a live ISO that carries {INITRAMFS_INSTALLED_ISO_PATH}, or pass the installed system's initramfs with --initrd"))
                } else {
                    missing("initrd", initrd)
                });
            }
        }
        if !root.is_dir("boot") {
            return Err(Error::new(
                Code::BootFileMissing,
                format!("'{}' does not exist", root.display("/boot")),
            )
            .fix("mount the EFI system partition at the target's /boot"));
        }
        if !target.boot_is_mountpoint {
            warnings.push(format!(
                "{} is not a mount point; the EFI system partition belongs there",
                root.display("/boot")
            ));
        }

        let options = boot.options.clone().unwrap_or_default();
        let mut argv: Vec<String> =
            ["recab", "init", "--root", &r, "--mode", &boot.mode.to_string()].iter().map(|s| s.to_string()).collect();
        match boot.mode {
            InstallMode::Ab => {
                if boot.root_device.is_some() {
                    return Err(invalid(
                        "root_device is for the mutable layout",
                        "give slot_a and slot_b for --boot ab",
                    ));
                }
                let (Some(a), Some(b_dev)) = (&boot.slot_a, &boot.slot_b) else {
                    return Err(invalid(
                        "the A/B layout needs both slot devices",
                        "pass --slot-a and --slot-b (recpart apply reports them)",
                    ));
                };
                if a == b_dev {
                    return Err(invalid(
                        "slot A and slot B are the same device",
                        "pass two different partitions",
                    ));
                }
                let mounted = RootMount::of(root.path())?;
                mounted.require("--slot-a", a)?;
                if mounted.holds(block_device("--slot-b", b_dev)?) {
                    return Err(Error::new(
                        Code::BootDeviceMismatch,
                        format!(
                            "--slot-b '{}' is the partition mounted at '{r}'; that is slot A",
                            b_dev.display()
                        ),
                    )
                    .fix("pass the other slot's partition as --slot-b (recpart apply reports both)"));
                }
                argv.extend(["--slot-a".into(), a.display().to_string()]);
                argv.extend(["--slot-b".into(), b_dev.display().to_string()]);
            }
            InstallMode::Mutable => {
                if boot.slot_a.is_some() || boot.slot_b.is_some() {
                    return Err(invalid(
                        "slot devices are for the A/B layout",
                        "give root_device for --boot mutable",
                    ));
                }
                let Some(device) = &boot.root_device else {
                    return Err(invalid(
                        "the mutable layout needs the root partition",
                        "pass --root-device",
                    ));
                };
                RootMount::of(root.path())?.require("--root-device", device)?;
                partuuid(device)?;
                argv.extend(["--root-device".into(), device.display().to_string()]);
            }
        }
        argv.extend(["--kernel".into(), kernel.display().to_string()]);
        if let Some(initrd) = &initrd {
            argv.extend(["--initrd".into(), initrd.display().to_string()]);
        }
        if !options.trim().is_empty() {
            argv.extend(["--options".into(), options.trim().to_string()]);
        }
        let what = match boot.mode {
            InstallMode::Ab => "Install systemd-boot, write both slots' boot entries, slot A's kernel and recab's state, and enable the trial service",
            InstallMode::Mutable => "Install systemd-boot, the kernel and the boot entry",
        };
        b.push(Section::Boot, what, Action::Host { argv });
    }

    if request.machine_id {
        b.setup(
            Section::MachineId,
            "Initialize this machine's identity",
            &["machine-id"],
        );
    }
    if !request.services.is_empty() {
        let available = target::services(&root, target.profile.init)?;
        for name in &request.services {
            if !available.iter().any(|s| &s.name == name) {
                return Err(unavailable(
                    format!("service '{name}' is not in the target"),
                    inspect_hint("services"),
                ));
            }
            b.setup(
                Section::Services,
                format!("Start {name} at boot"),
                &["service", name],
            );
        }
    }

    let steps = b.steps;
    Ok(Plan {
        schema_version: PLAN_SCHEMA_VERSION,
        target,
        request,
        steps,
        warnings,
        password_users,
        root,
    })
}

/// The partition's PARTUUID, which names the root in the boot entry.
fn partuuid(device: &Path) -> Result<String> {
    target_root::partuuid(device)
        .map_err(|e| Error::new(Code::IdentifierUnavailable, e.to_string()))?
        .ok_or_else(|| {
            Error::identifier_unavailable(&device.display().to_string(), "PARTUUID").fix(
                "pass the root partition of a GPT disk (recpart apply reports it), and run as root",
            )
        })
}

/// The filesystem mounted exactly at the root, which the boot entries must
/// name: an entry for any other partition boots another disk's system.
struct RootMount {
    root: PathBuf,
    /// mountinfo's device number for it.
    dev: (u32, u32),
    /// The device its mount source names, when the device number is an
    /// anonymous one (major 0, as btrfs reports) rather than the partition's.
    source_dev: Option<(u32, u32)>,
    source: String,
}

impl RootMount {
    fn of(root: &Path) -> Result<Self> {
        let table = mount_table()?;
        // The last mount on a point is the visible one.
        let Some(mount) = table.iter().rev().find(|m| m.mount_point == root) else {
            return Err(Error::new(
                Code::NotMountPoint,
                format!(
                    "'{}' is not a mount point, so the boot devices cannot be checked against the installed system",
                    root.display()
                ),
            )
            .fix("mount the installed system's root partition at --root"));
        };
        // A source that is no block device (a btrfs mounted by UUID) names none.
        let source_dev = match mount.dev.0 == 0 {
            true => match target_root::block_rdev(Path::new(&mount.source)) {
                Ok(rdev) => Some((libc::major(rdev), libc::minor(rdev))),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidInput) => None,
                Err(e) => return Err(Error::io(e)),
            },
            false => None,
        };
        Ok(Self {
            root: root.to_path_buf(),
            dev: mount.dev,
            source_dev,
            source: mount.source.clone(),
        })
    }

    fn holds(&self, dev: (u32, u32)) -> bool {
        self.dev == dev || self.source_dev == Some(dev)
    }

    /// Refuse a `flag` device that is not the partition mounted at the root.
    fn require(&self, flag: &str, device: &Path) -> Result<()> {
        if self.holds(block_device(flag, device)?) {
            return Ok(());
        }
        Err(Error::new(
            Code::BootDeviceMismatch,
            format!(
                "{flag} '{}' is not the partition mounted at '{}' ({})",
                device.display(),
                self.root.display(),
                self.source
            ),
        )
        .fix(format!(
            "pass the partition mounted at --root as {flag}, or mount the installed system's root partition at --root"
        )))
    }
}

/// A block device's number, through symlinks such as /dev/disk/by-uuid names.
fn block_device(flag: &str, device: &Path) -> Result<(u32, u32)> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    let shown = device.display().to_string();
    let meta = device
        .canonicalize()
        .and_then(std::fs::metadata)
        .map_err(|e| Error::path_unresolvable(&shown, e))?;
    if !meta.file_type().is_block_device() {
        return Err(invalid(
            format!("{flag} '{shown}' is not a block device"),
            "pass the partition's device, such as /dev/vda2 (recpart apply reports it)",
        ));
    }
    let rdev = meta.rdev();
    Ok((libc::major(rdev), libc::minor(rdev)))
}
