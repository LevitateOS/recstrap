//! The plan: a request, checked against the target, as the ordered steps
//! apply runs. Each step carries the exact command it stands for, so the
//! plan is also the manual install for anyone who would rather type it.

use crate::request::{check_hostname, check_username, invalid, BootMode, SetupRequest};
use crate::target::{self, BootloaderMethod, InitSystem, LocaleMethod, Root, Target};
use install_tools::error::{Code, Error, Result};
use install_tools::guard::mount_table;
use recinit::live::LIVE_MEDIA_DIR;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const PLAN_SCHEMA_VERSION: u32 = 1;
const SETUP_PROGRAM: &str = "/usr/libexec/recipe-system-setup";
/// The installed system's initramfs on the live medium, next to the live one
/// (`boot/initramfs.img`); the release build puts it there for systemd variants.
const INSTALLED_INITRAMFS: &str = "initramfs-installed.img";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Timezone,
    Locale,
    Keymap,
    Hostname,
    Users,
    Passwords,
    Bootloader,
    BootEntries,
    MachineId,
    Services,
}

/// What a step does. Paths are target paths ("/etc/hostname"), resolved
/// inside the root when the step runs.
#[derive(Debug, Clone)]
pub enum Action {
    Write {
        path: String,
        content: String,
        mode: u32,
    },
    /// Copy a file from the host (or, for `from_target`, from the target) into the target.
    Copy {
        from: PathBuf,
        from_target: bool,
        to: String,
    },
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
    /// The full text of a file the step writes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
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

/// `default`-style systemd-boot loader.conf keys (`key value`).
fn set_loader_key(existing: Option<&str>, key: &str, value: &str) -> String {
    let mut out = String::new();
    let mut replaced = false;
    for line in existing.unwrap_or("").lines() {
        if line.split_whitespace().next() == Some(key) {
            if !replaced {
                out.push_str(&format!("{key} {value}\n"));
                replaced = true;
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !replaced {
        out.push_str(&format!("{key} {value}\n"));
    }
    out
}

struct Builder<'a> {
    root: &'a Root,
    steps: Vec<Step>,
}

impl Builder<'_> {
    fn push(&mut self, section: Section, description: impl Into<String>, action: Action) {
        let root = self.root;
        let (command, content) = match &action {
            Action::Write {
                path,
                content,
                mode,
            } => {
                let shown = root.display(path);
                let mkdir = if root.is_dir(&parent_of(path)) {
                    String::new()
                } else {
                    format!("mkdir -p {} && ", shell_quote(&parent_of(&shown)))
                };
                let mut command = format!(
                    "{mkdir}cat > {} <<'EOF'\n{}EOF",
                    shell_quote(&shown),
                    content
                );
                if *mode != 0o644 {
                    command.push_str(&format!("\nchmod {mode:04o} {}", shell_quote(&shown)));
                }
                (command, Some(content.clone()))
            }
            Action::Copy {
                from,
                from_target,
                to,
            } => {
                let from = if *from_target {
                    root.display(&from.to_string_lossy())
                } else {
                    from.display().to_string()
                };
                (
                    format!(
                        "install -D -m 0644 {} {}",
                        shell_quote(&from),
                        shell_quote(&root.display(to))
                    ),
                    None,
                )
            }
            Action::Chroot { argv, .. } => (
                format!(
                    "recchroot {} -- {}",
                    shell_quote(&root.path().to_string_lossy()),
                    shell_line(argv)
                ),
                None,
            ),
            Action::Host { argv } => (shell_line(argv), None),
        };
        self.steps.push(Step {
            index: self.steps.len() + 1,
            section,
            description: description.into(),
            command,
            content,
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

    fn chroot(&mut self, section: Section, description: impl Into<String>, argv: &[&str]) {
        let argv = argv.iter().map(|s| s.to_string()).collect();
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

fn parent_of(path: &str) -> String {
    Path::new(path)
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "/".to_string())
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
        let locales = target::locales(&root, &target);
        let Some((choice, source)) = locales.iter().find(|(c, _)| &c.name == locale) else {
            return Err(unavailable(
                format!("locale '{locale}' is not available in the target"),
                inspect_hint("locales"),
            ));
        };
        let mut args = vec!["locale", locale.as_str(), "precompiled"];
        if !choice.compiled {
            let source = source.as_ref().expect("an uncompiled locale has a source");
            match target.locale_method {
                LocaleMethod::LocaleGen => {
                    args = vec!["locale", locale, "locale_gen", &source.line]
                }
                LocaleMethod::Localedef => {
                    args = vec![
                        "locale",
                        locale,
                        "localedef",
                        &source.input,
                        &source.charset,
                    ]
                }
                LocaleMethod::Precompiled | LocaleMethod::Musl => {
                    unreachable!("these list compiled locales only")
                }
            }
        }
        b.setup(
            Section::Locale,
            format!("Set the system locale to {locale}"),
            &args,
        );
    }
    if let Some(keymap) = &request.keymap {
        if !target::keymaps(&root, target.init).contains(keymap) {
            return Err(unavailable(
                format!("keymap '{keymap}' is not in the target"),
                inspect_hint("keymaps"),
            ));
        }
        if target.init == InitSystem::Openrc
            && !root.is_file("etc/init.d/keymaps")
            && !root.is_file("etc/init.d/loadkeys")
            && !root.is_file("etc/init.d/loadkmap")
        {
            warnings.push("the target has no keymaps, loadkeys or loadkmap service, so nothing loads the keymap at boot".into());
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
        if target.account_tool.is_none() {
            return Err(unavailable(
                "the target has neither useradd nor adduser",
                "deploy a complete system first",
            ));
        }
        let groups = target::account_names(&root, "etc/group");
        let existing_users = target::account_names(&root, "etc/passwd");
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
                user_groups.push(target.admin.group.clone());
                if target.admin.tool.is_none() {
                    warnings.push("the target ships neither sudo nor doas; the admin group cannot become root".into());
                } else if !target.admin.rule_present {
                    return Err(unavailable(
                        "the target lacks its recipe-owned administrator rule",
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
        let live = |file: &str| PathBuf::from(format!("{LIVE_MEDIA_DIR}/boot/{file}"));
        let kernel = boot.kernel.clone().unwrap_or_else(|| live("vmlinuz"));
        // The live medium's own initramfs (boot/initramfs.img) only finds the live
        // medium; the installed system boots through the one built from its rootfs,
        // which the ISO carries beside it.
        let live_initramfs = live("initramfs.img");
        let initrd = match &boot.initrd {
            Some(path) if path.as_os_str().is_empty() => None,
            Some(path) => Some(path.clone()),
            None if target.init == InitSystem::Systemd => Some(live(INSTALLED_INITRAMFS)),
            None => None,
        };
        if let Some(initrd) = &initrd {
            let same = |a: &Path, b: &Path| match (a.canonicalize(), b.canonicalize()) {
                (Ok(a), Ok(b)) => a == b,
                _ => a == b,
            };
            if same(initrd, &live_initramfs) {
                return Err(invalid(
                    format!(
                        "'{}' is the live medium's initramfs, which looks for the live medium and ignores root=: the installed system would not boot from its disk",
                        initrd.display()
                    ),
                    format!(
                        "pass the installed system's initramfs ({}), or leave --initrd out to use it",
                        live(INSTALLED_INITRAMFS).display()
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
                    .fix("boot a live ISO that carries boot/initramfs-installed.img, or pass the installed system's initramfs with --initrd")
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

        match target.bootloader {
            Some(BootloaderMethod::Bootctl) => b.chroot(
                Section::Bootloader,
                "Install systemd-boot on the ESP (firmware variables are left to recab and the first boot)",
                &["bootctl", "install", "--esp-path=/boot", "--no-variables"],
            ),
            Some(BootloaderMethod::Copy) => {
                let efi = PathBuf::from(target.systemd_boot_efi.clone().expect("Copy is chosen only with the EFI binary"));
                let name = efi.file_name().unwrap().to_string_lossy().into_owned();
                let fallback = if name.contains("aa64") { "BOOTAA64.EFI" } else { "BOOTX64.EFI" };
                b.push(
                    Section::Bootloader,
                    "Install systemd-boot as the firmware's default loader",
                    Action::Copy { from: efi.clone(), from_target: true, to: format!("/boot/EFI/BOOT/{fallback}") },
                );
                b.push(
                    Section::Bootloader,
                    "Install systemd-boot under its own name",
                    Action::Copy { from: efi, from_target: true, to: format!("/boot/EFI/systemd/{name}") },
                );
            }
            None => {
                return Err(Error::new(Code::BootFileMissing, "the target has no systemd-boot (no bootctl and no systemd-boot EFI binary)")
                    .fix("extract a complete system (recstrap) first"))
            }
        }

        let options = boot.options.clone().unwrap_or_default();
        match boot.mode {
            BootMode::Ab => {
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
                let mut argv: Vec<String> = ["recab", "init", "--root", &r, "--slot-a"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                argv.push(a.display().to_string());
                argv.push("--slot-b".into());
                argv.push(b_dev.display().to_string());
                argv.push("--kernel".into());
                argv.push(kernel.display().to_string());
                if let Some(initrd) = &initrd {
                    argv.push("--initrd".into());
                    argv.push(initrd.display().to_string());
                }
                if !options.trim().is_empty() {
                    argv.push("--options".into());
                    argv.push(options.trim().to_string());
                }
                b.push(
                    Section::BootEntries,
                    "Write both slots' boot entries, slot A's kernel and recab's state, and enable the trial service",
                    Action::Host { argv },
                );
            }
            BootMode::Mutable => {
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
                let partuuid = partuuid(device)?;
                b.push(
                    Section::BootEntries,
                    "Install the kernel on the ESP",
                    Action::Copy {
                        from: kernel,
                        from_target: false,
                        to: "/boot/vmlinuz".into(),
                    },
                );
                if let Some(initrd) = &initrd {
                    b.push(
                        Section::BootEntries,
                        "Install the initramfs on the ESP",
                        Action::Copy {
                            from: initrd.clone(),
                            from_target: false,
                            to: "/boot/initramfs.img".into(),
                        },
                    );
                }
                let mut entry = format!("title {}\nlinux /vmlinuz\n", target.os_name);
                if initrd.is_some() {
                    entry.push_str("initrd /initramfs.img\n");
                }
                let extra = if options.trim().is_empty() {
                    String::new()
                } else {
                    format!(" {}", options.trim())
                };
                entry.push_str(&format!(
                    "options root=PARTUUID={partuuid} rw rootwait{extra}\n"
                ));
                let entry_name = format!("{}.conf", target.os_id);
                b.push(
                    Section::BootEntries,
                    format!(
                        "Write the boot entry, naming {} by its PARTUUID",
                        device.display()
                    ),
                    Action::Write {
                        path: format!("/boot/loader/entries/{entry_name}"),
                        content: entry,
                        mode: 0o644,
                    },
                );
                let existing = root.read("boot/loader/loader.conf");
                let mut loader = set_loader_key(existing.as_deref(), "default", &entry_name);
                if !loader
                    .lines()
                    .any(|l| l.split_whitespace().next() == Some("timeout"))
                {
                    loader = set_loader_key(Some(&loader), "timeout", "5");
                }
                b.push(
                    Section::BootEntries,
                    "Make it the default entry",
                    Action::Write {
                        path: "/boot/loader/loader.conf".into(),
                        content: loader,
                        mode: 0o644,
                    },
                );
            }
        }
    }

    if request.machine_id {
        b.setup(
            Section::MachineId,
            "Initialize this machine's identity",
            &["machine-id"],
        );
    }
    if !request.services.is_empty() {
        let available = target::services(&root, target.init);
        for service in &request.services {
            let name = if target.init == InitSystem::Systemd && !service.contains('.') {
                format!("{service}.service")
            } else {
                service.clone()
            };
            if !available.iter().any(|s| s.name == name) {
                return Err(unavailable(
                    format!("service '{service}' is not in the target"),
                    inspect_hint("services"),
                ));
            }
            b.setup(
                Section::Services,
                format!("Start {name} at boot"),
                &["service", &name],
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
    let device_text = device.display().to_string();
    let out = Command::new("blkid")
        .args(["-s", "PARTUUID", "-o", "value"])
        .arg(device)
        .output()
        .map_err(|e| Error::command_failed("blkid", e))?;
    let value = String::from_utf8_lossy(&out.stdout)
        .trim()
        .to_ascii_lowercase();
    if !out.status.success() || value.is_empty() {
        return Err(Error::identifier_unavailable(&device_text, "PARTUUID").fix(
            "pass the root partition of a GPT disk (recpart apply reports it), and run as root",
        ));
    }
    Ok(value)
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
        let source_dev = (mount.dev.0 == 0)
            .then(|| block_device("mount source", Path::new(&mount.source)).ok())
            .flatten();
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
