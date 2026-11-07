//! The plan: a request, checked against the target, as the ordered steps
//! apply runs. Each step carries the exact command it stands for, so the
//! plan is also the manual install for anyone who would rather type it.

use crate::request::{check_hostname, check_username, invalid, BootMode, SetupRequest};
use crate::target::{
    self, admin_rule, AccountTool, AdminTool, BootloaderMethod, InitSystem, LocaleMethod, Root, Target,
};
use install_tools::error::{Code, Error, Result};
use recinit::live::LIVE_MEDIA_DIR;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const PLAN_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Timezone,
    Locale,
    Keymap,
    Hostname,
    Users,
    Admin,
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
    Write { path: String, content: String, mode: u32 },
    Symlink { path: String, target: String },
    Remove { path: String },
    /// Copy a file from the host (or, for `from_target`, from the target) into the target.
    Copy { from: PathBuf, from_target: bool, to: String },
    /// Run in the target through recchroot; `passwords` feeds it the
    /// `name:password` lines read from stdin.
    Chroot { argv: Vec<String>, passwords: bool },
    /// Run on the live system.
    Host { argv: Vec<String> },
    /// Write a new random machine id (systems without systemd-machine-id-setup).
    RandomMachineId,
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
    if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c)) {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\"'\"'"))
    }
}

fn shell_line(argv: &[String]) -> String {
    argv.iter().map(|w| shell_quote(w)).collect::<Vec<_>>().join(" ")
}

/// `KEY=value` files (locale.conf, vconsole.conf, conf.d): set `key`,
/// keeping every other line.
fn set_assignment(existing: Option<&str>, key: &str, value: &str) -> String {
    let prefix = format!("{key}=");
    let mut out = String::new();
    let mut replaced = false;
    for line in existing.unwrap_or("").lines() {
        if line.trim_start().starts_with(&prefix) {
            if !replaced {
                out.push_str(&format!("{key}={value}\n"));
                replaced = true;
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !replaced {
        out.push_str(&format!("{key}={value}\n"));
    }
    out
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
            Action::Write { path, content, mode } => {
                let shown = root.display(path);
                let mkdir = if root.is_dir(&parent_of(path)) {
                    String::new()
                } else {
                    format!("mkdir -p {} && ", shell_quote(&parent_of(&shown)))
                };
                let mut command = format!("{mkdir}cat > {} <<'EOF'\n{}EOF", shell_quote(&shown), content);
                if *mode != 0o644 {
                    command.push_str(&format!("\nchmod {mode:04o} {}", shell_quote(&shown)));
                }
                (command, Some(content.clone()))
            }
            Action::Symlink { path, target } => {
                (format!("ln -sfn {} {}", shell_quote(target), shell_quote(&root.display(path))), None)
            }
            Action::Remove { path } => (format!("rm -f {}", shell_quote(&root.display(path))), None),
            Action::Copy { from, from_target, to } => {
                let from = if *from_target {
                    root.display(&from.to_string_lossy())
                } else {
                    from.display().to_string()
                };
                (format!("install -D -m 0644 {} {}", shell_quote(&from), shell_quote(&root.display(to))), None)
            }
            Action::Chroot { argv, .. } => (
                format!("recchroot {} -- {}", shell_quote(&root.path().to_string_lossy()), shell_line(argv)),
                None,
            ),
            Action::Host { argv } => (shell_line(argv), None),
            Action::RandomMachineId => (
                format!(
                    "od -An -N16 -tx1 /dev/urandom | tr -d ' \\n' > {path} && echo >> {path}",
                    path = shell_quote(&root.display("/etc/machine-id"))
                ),
                None,
            ),
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

    fn chroot(&mut self, section: Section, description: impl Into<String>, argv: &[&str]) {
        let argv = argv.iter().map(|s| s.to_string()).collect();
        self.push(section, description, Action::Chroot { argv, passwords: false });
    }
}

fn parent_of(path: &str) -> String {
    Path::new(path).parent().map(|p| p.display().to_string()).unwrap_or_else(|| "/".to_string())
}

fn unavailable(message: impl Into<String>, fix: impl Into<String>) -> Error {
    Error::new(Code::ChoiceUnavailable, message).fix(fix)
}

pub fn build(root: Root, target: Target, request: SetupRequest) -> Result<Plan> {
    let mut b = Builder { root: &root, steps: Vec::new() };
    let mut warnings = Vec::new();
    let r = root.path().display().to_string();
    let inspect_hint = |what: &str| format!("choose one that `recsetup inspect --root {r}` lists under {what}");

    if let Some(tz) = &request.timezone {
        let known = target::timezones(&root).iter().any(|z| &z.name == tz);
        if !known {
            return Err(unavailable(format!("time zone '{tz}' is not in the target's zoneinfo"), inspect_hint("timezones")));
        }
        if target.has_zoneinfo && root.is_file(&format!("usr/share/zoneinfo/{tz}")) {
            b.push(
                Section::Timezone,
                format!("Set the time zone to {tz}"),
                Action::Symlink { path: "/etc/localtime".into(), target: format!("../usr/share/zoneinfo/{tz}") },
            );
        } else {
            b.push(
                Section::Timezone,
                "Use UTC: the target has no time zone data, and no /etc/localtime means UTC",
                Action::Remove { path: "/etc/localtime".into() },
            );
        }
    }

    if let Some(locale) = &request.locale {
        let locales = target::locales(&root, &target);
        let Some((choice, source)) = locales.iter().find(|(c, _)| &c.name == locale) else {
            let fix = match target.locale_method {
                LocaleMethod::Precompiled => format!(
                    "the target has no locale sources, so only its compiled locales can be used; {}",
                    inspect_hint("locales")
                ),
                _ => inspect_hint("locales"),
            };
            return Err(unavailable(format!("locale '{locale}' is not available in the target"), fix));
        };
        if !choice.compiled {
            let source = source.as_ref().expect("an uncompiled locale has a source");
            match target.locale_method {
                LocaleMethod::LocaleGen => {
                    let existing = root.read("etc/locale.gen").unwrap_or_default();
                    let mut enabled = false;
                    let mut lines: Vec<String> = existing
                        .lines()
                        .map(|line| {
                            let bare = line.trim().trim_start_matches('#').trim();
                            if !enabled && bare.split_whitespace().collect::<Vec<_>>().join(" ") == source.line {
                                enabled = true;
                                source.line.clone()
                            } else {
                                line.to_string()
                            }
                        })
                        .collect();
                    if !enabled {
                        lines.push(source.line.clone());
                    }
                    b.push(
                        Section::Locale,
                        format!("Enable {locale} in /etc/locale.gen"),
                        Action::Write { path: "/etc/locale.gen".into(), content: lines.join("\n") + "\n", mode: 0o644 },
                    );
                    b.chroot(Section::Locale, "Generate the enabled locales", &["locale-gen"]);
                }
                LocaleMethod::Localedef => b.chroot(
                    Section::Locale,
                    format!("Compile {locale}"),
                    &["localedef", "-i", &source.input, "-f", &source.charset, locale],
                ),
                LocaleMethod::Precompiled | LocaleMethod::Musl => unreachable!("these list compiled locales only"),
            }
        }
        b.push(
            Section::Locale,
            format!("Set the system locale to {locale}"),
            Action::Write {
                path: "/etc/locale.conf".into(),
                content: set_assignment(root.read("etc/locale.conf").as_deref(), "LANG", locale),
                mode: 0o644,
            },
        );
        if target.locale_method == LocaleMethod::Musl {
            // musl reads LANG from the environment only; login shells take it from here.
            b.push(
                Section::Locale,
                "Export the locale to login shells (musl has no system locale file of its own)",
                Action::Write {
                    path: "/etc/profile.d/locale.sh".into(),
                    content: "# Written by recsetup: the system locale from /etc/locale.conf.\n\
                              [ -r /etc/locale.conf ] && . /etc/locale.conf && export LANG\n"
                        .into(),
                    mode: 0o644,
                },
            );
        }
    }

    if let Some(keymap) = &request.keymap {
        let keymaps = target::keymaps(&root, target.init);
        if keymaps.is_empty() {
            return Err(unavailable(
                "the target ships no console keymaps",
                "leave the keymap out: the console then uses the kernel's US layout",
            ));
        }
        if !keymaps.contains(keymap) {
            return Err(unavailable(format!("keymap '{keymap}' is not in the target"), inspect_hint("keymaps")));
        }
        match target.init {
            InitSystem::Systemd => b.push(
                Section::Keymap,
                format!("Set the console keymap to {keymap}"),
                Action::Write {
                    path: "/etc/vconsole.conf".into(),
                    content: set_assignment(root.read("etc/vconsole.conf").as_deref(), "KEYMAP", keymap),
                    mode: 0o644,
                },
            ),
            InitSystem::Openrc => {
                // Upstream OpenRC calls the service keymaps, Alpine's kbd
                // loadkeys; either reads keymap= from its own conf.d file.
                let service = ["keymaps", "loadkeys"]
                    .into_iter()
                    .find(|s| root.is_file(&format!("etc/init.d/{s}")));
                let conf = format!("/etc/conf.d/{}", service.unwrap_or("keymaps"));
                b.push(
                    Section::Keymap,
                    format!("Set the console keymap to {keymap}"),
                    Action::Write {
                        content: set_assignment(root.read(&conf).as_deref(), "keymap", &format!("\"{keymap}\"")),
                        path: conf,
                        mode: 0o644,
                    },
                );
                match service {
                    None => warnings.push(
                        "the target has no keymaps or loadkeys service, so nothing loads the keymap at boot".into(),
                    ),
                    Some(service) if !root.exists(&format!("etc/runlevels/boot/{service}")) => {
                        b.chroot(Section::Keymap, "Load the keymap at boot", &["rc-update", "add", service, "boot"])
                    }
                    Some(_) => {}
                }
            }
        }
    }

    if let Some(hostname) = &request.hostname {
        check_hostname(hostname)?;
        b.push(
            Section::Hostname,
            format!("Set the hostname to {hostname}"),
            Action::Write { path: "/etc/hostname".into(), content: format!("{hostname}\n"), mode: 0o644 },
        );
        b.push(
            Section::Hostname,
            "Map the hostname and localhost in /etc/hosts",
            Action::Write { path: "/etc/hosts".into(), content: hosts(root.read("etc/hosts").as_deref(), hostname), mode: 0o644 },
        );
    }

    let mut password_users = Vec::new();
    if request.root_password {
        password_users.push("root".to_string());
    }
    if !request.users.is_empty() {
        let tool = target.account_tool.ok_or_else(|| {
            unavailable("the target has neither useradd nor adduser", "extract a complete system (recstrap) first")
        })?;
        let groups = target::account_names(&root, "etc/group");
        let existing_users = target::account_names(&root, "etc/passwd");
        let mut seen = Vec::new();
        let mut any_admin = false;
        for user in &request.users {
            check_username(&user.name)?;
            if seen.contains(&user.name) {
                return Err(invalid(format!("user '{}' is listed twice", user.name), "list each user once"));
            }
            seen.push(user.name.clone());
            let shell = user.shell.clone().unwrap_or_else(|| target.default_shell.clone());
            if !shell.starts_with('/') || !root.is_file(&shell) {
                return Err(unavailable(format!("shell '{shell}' is not in the target"), inspect_hint("shells")));
            }
            let mut user_groups: Vec<String> = Vec::new();
            if user.admin {
                any_admin = true;
                user_groups.push(target.admin.group.clone());
            }
            for g in &user.groups {
                if !user_groups.contains(g) {
                    user_groups.push(g.clone());
                }
            }
            for g in &user_groups {
                if !groups.contains(g) {
                    return Err(unavailable(format!("group '{g}' does not exist in the target"), inspect_hint("groups")));
                }
            }
            if user.password {
                password_users.push(user.name.clone());
            }
            if existing_users.contains(&user.name) {
                warnings.push(format!("user '{}' already exists in the target; it is left as it is", user.name));
                continue;
            }
            match tool {
                AccountTool::Shadow => {
                    let mut argv = vec!["useradd".to_string(), "-m".into(), "-s".into(), shell.clone()];
                    if !user_groups.is_empty() {
                        argv.push("-G".into());
                        argv.push(user_groups.join(","));
                    }
                    argv.push(user.name.clone());
                    b.push(Section::Users, format!("Create user {}", user.name), Action::Chroot { argv, passwords: false });
                }
                AccountTool::Busybox => {
                    b.chroot(Section::Users, format!("Create user {}", user.name), &["adduser", "-D", "-s", &shell, &user.name]);
                    for g in &user_groups {
                        b.chroot(Section::Users, format!("Add {} to {g}", user.name), &["addgroup", &user.name, g]);
                    }
                }
            }
        }
        if any_admin {
            match target.admin.tool {
                None => warnings.push(format!(
                    "the target ships neither sudo nor doas: members of {} get no command to become root",
                    target.admin.group
                )),
                Some(_) if target.admin.rule_present => {}
                Some(tool) => {
                    let (path, rule, mode) = admin_rule(tool);
                    let content = match tool {
                        AdminTool::Sudo => rule.to_string(),
                        AdminTool::Doas => format!("{}{rule}", root.read(path).unwrap_or_default()),
                    };
                    let name = match tool {
                        AdminTool::Sudo => "sudo",
                        AdminTool::Doas => "doas",
                    };
                    b.push(
                        Section::Admin,
                        format!("Let the {} group use {name}", target.admin.group),
                        Action::Write { path: path.into(), content, mode },
                    );
                }
            }
        }
    }
    if !password_users.is_empty() {
        if !root.has_program("chpasswd") {
            return Err(unavailable("the target has no chpasswd", "extract a complete system (recstrap) first"));
        }
        b.push(
            Section::Passwords,
            format!("Set the password of {} (read from stdin)", password_users.join(", ")),
            Action::Chroot { argv: vec!["chpasswd".into()], passwords: true },
        );
    }

    if let Some(boot) = &request.boot {
        let live = |file: &str| PathBuf::from(format!("{LIVE_MEDIA_DIR}/boot/{file}"));
        let kernel = boot.kernel.clone().unwrap_or_else(|| live("vmlinuz"));
        let initrd = match &boot.initrd {
            Some(path) if path.as_os_str().is_empty() => None,
            Some(path) => Some(path.clone()),
            None if target.init == InitSystem::Systemd => Some(live("initramfs.img")),
            None => None,
        };
        let missing = |what: &str, path: &Path| {
            Error::new(Code::BootFileMissing, format!("{what} '{}' does not exist", path.display()))
                .fix(format!("pass the {what}'s path with --{}", what.split(' ').next().unwrap_or(what)))
        };
        if !kernel.is_file() {
            return Err(missing("kernel", &kernel));
        }
        if let Some(initrd) = &initrd {
            if !initrd.is_file() {
                return Err(missing("initrd", initrd));
            }
        }
        if !root.is_dir("boot") {
            return Err(Error::new(Code::BootFileMissing, format!("'{}' does not exist", root.display("/boot")))
                .fix("mount the EFI system partition at the target's /boot"));
        }
        if !target.boot_is_mountpoint {
            warnings.push(format!("{} is not a mount point; the EFI system partition belongs there", root.display("/boot")));
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
                    return Err(invalid("root_device is for the mutable layout", "give slot_a and slot_b for --boot ab"));
                }
                let (Some(a), Some(b_dev)) = (&boot.slot_a, &boot.slot_b) else {
                    return Err(invalid(
                        "the A/B layout needs both slot devices",
                        "pass --slot-a and --slot-b (recpart apply reports them)",
                    ));
                };
                if a == b_dev {
                    return Err(invalid("slot A and slot B are the same device", "pass two different partitions"));
                }
                let mut argv: Vec<String> = ["recab", "init", "--root", &r, "--slot-a"].iter().map(|s| s.to_string()).collect();
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
                    return Err(invalid("slot devices are for the A/B layout", "give root_device for --boot mutable"));
                }
                let Some(device) = &boot.root_device else {
                    return Err(invalid("the mutable layout needs the root partition", "pass --root-device"));
                };
                let partuuid = partuuid(device)?;
                b.push(
                    Section::BootEntries,
                    "Install the kernel on the ESP",
                    Action::Copy { from: kernel, from_target: false, to: "/boot/vmlinuz".into() },
                );
                if let Some(initrd) = &initrd {
                    b.push(
                        Section::BootEntries,
                        "Install the initramfs on the ESP",
                        Action::Copy { from: initrd.clone(), from_target: false, to: "/boot/initramfs.img".into() },
                    );
                }
                let mut entry = format!("title {}\nlinux /vmlinuz\n", target.os_name);
                if initrd.is_some() {
                    entry.push_str("initrd /initramfs.img\n");
                }
                let extra = if options.trim().is_empty() { String::new() } else { format!(" {}", options.trim()) };
                entry.push_str(&format!("options root=PARTUUID={partuuid} rw rootwait{extra}\n"));
                let entry_name = format!("{}.conf", target.os_id);
                b.push(
                    Section::BootEntries,
                    format!("Write the boot entry, naming {} by its PARTUUID", device.display()),
                    Action::Write { path: format!("/boot/loader/entries/{entry_name}"), content: entry, mode: 0o644 },
                );
                let existing = root.read("boot/loader/loader.conf");
                let mut loader = set_loader_key(existing.as_deref(), "default", &entry_name);
                if !loader.lines().any(|l| l.split_whitespace().next() == Some("timeout")) {
                    loader = set_loader_key(Some(&loader), "timeout", "5");
                }
                b.push(
                    Section::BootEntries,
                    "Make it the default entry",
                    Action::Write { path: "/boot/loader/loader.conf".into(), content: loader, mode: 0o644 },
                );
            }
        }
    }

    if request.machine_id {
        if root.has_program("systemd-machine-id-setup") {
            b.chroot(Section::MachineId, "Give the system its machine id", &["systemd-machine-id-setup"]);
        } else if root.read("etc/machine-id").is_some_and(|id| id.trim().len() == 32) {
            warnings.push("the target already has a machine id; it is kept".into());
        } else {
            b.push(Section::MachineId, "Give the system a random machine id", Action::RandomMachineId);
        }
    }

    if !request.services.is_empty() {
        let available = target::services(&root, target.init);
        for service in &request.services {
            match target.init {
                InitSystem::Systemd => {
                    let unit = if service.contains('.') { service.clone() } else { format!("{service}.service") };
                    if !available.iter().any(|s| s.name == unit)
                        && !root.is_file(&format!("usr/lib/systemd/system/{unit}"))
                        && !root.is_file(&format!("etc/systemd/system/{unit}"))
                    {
                        return Err(unavailable(format!("service '{service}' is not in the target"), inspect_hint("services")));
                    }
                    // Offline: in a chroot there is no service manager to tell.
                    b.chroot(
                        Section::Services,
                        format!("Start {unit} at boot"),
                        &["env", "SYSTEMD_OFFLINE=1", "systemctl", "enable", &unit],
                    );
                }
                InitSystem::Openrc => {
                    if !available.iter().any(|s| &s.name == service) {
                        return Err(unavailable(format!("service '{service}' is not in the target"), inspect_hint("services")));
                    }
                    b.chroot(Section::Services, format!("Start {service} at boot"), &["rc-update", "add", service, "default"]);
                }
            }
        }
    }

    let steps = b.steps;
    Ok(Plan { schema_version: PLAN_SCHEMA_VERSION, target, request, steps, warnings, password_users, root })
}

/// The hosts file with localhost and `hostname` mapped, other lines kept.
fn hosts(existing: Option<&str>, hostname: &str) -> String {
    let mut lines: Vec<String> = existing
        .unwrap_or("")
        .lines()
        .filter(|l| l.split_whitespace().next() != Some("127.0.1.1"))
        .map(str::to_string)
        .collect();
    let maps = |lines: &[String], addr: &str| {
        lines.iter().any(|l| {
            let mut words = l.split_whitespace();
            words.next() == Some(addr) && words.any(|w| w == "localhost")
        })
    };
    let mut head = Vec::new();
    if !maps(&lines, "127.0.0.1") {
        head.push("127.0.0.1   localhost".to_string());
    }
    if !maps(&lines, "::1") {
        head.push("::1         localhost".to_string());
    }
    head.append(&mut lines);
    head.push(format!("127.0.1.1   {hostname}.localdomain {hostname}"));
    head.join("\n") + "\n"
}

/// The partition's PARTUUID, which names the root in the boot entry.
fn partuuid(device: &Path) -> Result<String> {
    let device_text = device.display().to_string();
    let out = Command::new("blkid")
        .args(["-s", "PARTUUID", "-o", "value"])
        .arg(device)
        .output()
        .map_err(|e| Error::command_failed("blkid", e))?;
    let value = String::from_utf8_lossy(&out.stdout).trim().to_ascii_lowercase();
    if !out.status.success() || value.is_empty() {
        return Err(Error::identifier_unavailable(&device_text, "PARTUUID")
            .fix("pass the root partition of a GPT disk (recpart apply reports it), and run as root"));
    }
    Ok(value)
}
