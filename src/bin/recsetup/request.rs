//! What to set up: a JSON request (`--request`), the flags that fill or
//! override it, and the checks a request must pass before anything is
//! planned. Every section is optional; a request with only `boot` and
//! `services` is the boot step, one with the rest is the configuration step.
//!
//! Passwords are never part of the request: apply reads them from stdin.

use install_tools::error::{Code, Error, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupRequest {
    /// A zone from the target's zoneinfo, such as `Europe/Amsterdam`, or `UTC`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// The system `LANG`, such as `en_US.UTF-8`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// The console keymap, such as `de` or `us`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keymap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// Set root's password (read from stdin at apply).
    #[serde(default)]
    pub root_password: bool,
    #[serde(default)]
    pub users: Vec<UserRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot: Option<BootRequest>,
    /// Give the system its machine id.
    #[serde(default)]
    pub machine_id: bool,
    /// Services to start at boot, by bare name as `inspect` lists them (`sshd`).
    #[serde(default)]
    pub services: Vec<String>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserRequest {
    pub name: String,
    /// Extra groups, on top of the admin group when `admin` is set.
    #[serde(default)]
    pub groups: Vec<String>,
    /// Login shell; root's login shell in the target when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// Member of the admin group (wheel), with the privilege tool of the target's profile.
    #[serde(default = "yes")]
    pub admin: bool,
    /// Set the user's password (read from stdin at apply).
    #[serde(default = "yes")]
    pub password: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum BootMode {
    /// Two root slots managed by recab.
    Ab,
    /// One root and one boot entry.
    Mutable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootRequest {
    pub mode: BootMode,
    /// A/B: slot A's partition (the installed system) and slot B's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_a: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_b: Option<PathBuf>,
    /// Mutable: the root partition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_device: Option<PathBuf>,
    /// Kernel to install; the live media's when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel: Option<PathBuf>,
    /// Initramfs to install; unset takes the installed system's initramfs the
    /// live media carries (`boot/initramfs-installed.img`) on systemd systems
    /// and none on OpenRC ones (they boot without one). An empty string means
    /// none. The live media's own `boot/initramfs.img` is refused: it boots
    /// only the live medium.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initrd: Option<PathBuf>,
    /// Extra kernel options for the entries (consoles, for example).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<String>,
}

/// The request flags shared by `plan` and `apply`. They override or add to
/// what `--request` says.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct RequestFlags {
    /// JSON request file; the flags below override or add to it
    #[arg(long, value_name = "FILE")]
    pub request: Option<PathBuf>,
    /// Time zone, such as Europe/Amsterdam (`inspect` lists them)
    #[arg(long)]
    pub timezone: Option<String>,
    /// System locale (LANG), such as en_US.UTF-8
    #[arg(long)]
    pub locale: Option<String>,
    /// Console keymap, such as de
    #[arg(long)]
    pub keymap: Option<String>,
    /// Hostname, such as levitate-pc
    #[arg(long)]
    pub hostname: Option<String>,
    /// Set root's password, read from stdin at apply
    #[arg(long)]
    pub root_password: bool,
    /// Create a user in the admin group with the default shell; its password is read from stdin at apply (repeatable)
    #[arg(long = "user", value_name = "NAME")]
    pub users: Vec<String>,
    /// Install systemd-boot and write the boot entries for this layout
    #[arg(long, value_enum)]
    pub boot: Option<BootMode>,
    /// A/B slot A partition
    #[arg(long, value_name = "DEVICE")]
    pub slot_a: Option<PathBuf>,
    /// A/B slot B partition
    #[arg(long, value_name = "DEVICE")]
    pub slot_b: Option<PathBuf>,
    /// Mutable root partition
    #[arg(long, value_name = "DEVICE")]
    pub root_device: Option<PathBuf>,
    /// Kernel image (default: the live media's)
    #[arg(long)]
    pub kernel: Option<PathBuf>,
    /// Initramfs image (default: the live media's boot/initramfs-installed.img on systemd systems, none on OpenRC)
    #[arg(long, conflicts_with = "no_initrd")]
    pub initrd: Option<PathBuf>,
    /// Boot without an initramfs
    #[arg(long)]
    pub no_initrd: bool,
    /// Extra kernel options for the boot entries
    #[arg(long)]
    pub options: Option<String>,
    /// Give the system its machine id
    #[arg(long)]
    pub machine_id: bool,
    /// Service to start at boot, by bare name (repeatable)
    #[arg(long = "service", value_name = "NAME")]
    pub services: Vec<String>,
}

impl RequestFlags {
    pub fn into_request(self) -> Result<SetupRequest> {
        let mut request = match &self.request {
            Some(path) => {
                let text = std::fs::read_to_string(path).map_err(|e| {
                    Error::new(Code::InvalidRequest, format!("cannot read request '{}': {e}", path.display()))
                        .fix("pass the path of a readable JSON request file")
                })?;
                serde_json::from_str(&text).map_err(|e| {
                    Error::new(Code::InvalidRequest, format!("request '{}' is not valid: {e}", path.display()))
                        .fix("see the recsetup docs page for the request fields")
                })?
            }
            None => SetupRequest::default(),
        };
        let set = |slot: &mut Option<String>, value: Option<String>| {
            if value.is_some() {
                *slot = value;
            }
        };
        set(&mut request.timezone, self.timezone);
        set(&mut request.locale, self.locale);
        set(&mut request.keymap, self.keymap);
        set(&mut request.hostname, self.hostname);
        request.root_password |= self.root_password;
        request.machine_id |= self.machine_id;
        for name in self.users {
            request.users.push(UserRequest { name, groups: Vec::new(), shell: None, admin: true, password: true });
        }
        request.services.extend(self.services);

        if let Some(mode) = self.boot {
            match &mut request.boot {
                Some(boot) => boot.mode = mode,
                None => {
                    request.boot = Some(BootRequest {
                        mode,
                        slot_a: None,
                        slot_b: None,
                        root_device: None,
                        kernel: None,
                        initrd: None,
                        options: None,
                    })
                }
            }
        }
        let boot_flags = self.slot_a.is_some()
            || self.slot_b.is_some()
            || self.root_device.is_some()
            || self.kernel.is_some()
            || self.initrd.is_some()
            || self.no_initrd
            || self.options.is_some();
        match &mut request.boot {
            Some(boot) => {
                let set_path = |slot: &mut Option<PathBuf>, value: Option<PathBuf>| {
                    if value.is_some() {
                        *slot = value;
                    }
                };
                set_path(&mut boot.slot_a, self.slot_a);
                set_path(&mut boot.slot_b, self.slot_b);
                set_path(&mut boot.root_device, self.root_device);
                set_path(&mut boot.kernel, self.kernel);
                set_path(&mut boot.initrd, self.initrd);
                if self.no_initrd {
                    boot.initrd = Some(PathBuf::new());
                }
                if self.options.is_some() {
                    boot.options = self.options;
                }
            }
            None if boot_flags => {
                return Err(Error::new(Code::InvalidRequest, "boot options were given without a boot layout")
                    .fix("add --boot ab or --boot mutable (or a \"boot\" section in the request)"));
            }
            None => {}
        }
        Ok(request)
    }
}

pub fn invalid(message: impl Into<String>, fix: impl Into<String>) -> Error {
    Error::new(Code::InvalidRequest, message).fix(fix)
}

/// A hostname of RFC 1123 labels: letters, digits and inner hyphens, at
/// most 63 characters a label and 64 in all.
pub fn check_hostname(name: &str) -> Result<()> {
    let label_ok = |label: &str| {
        !label.is_empty()
            && label.len() <= 63
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    };
    if name.is_empty() || name.len() > 64 || !name.split('.').all(label_ok) {
        return Err(invalid(
            format!("'{name}' is not a valid hostname"),
            "use letters, digits and hyphens (not first or last), at most 64 characters, such as levitate-pc",
        ));
    }
    Ok(())
}

/// A user name `useradd` and busybox `adduser` both take.
pub fn check_username(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_');
    let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if !first_ok || !rest_ok || name.len() > 32 {
        return Err(invalid(
            format!("'{name}' is not a valid user name"),
            "start with a lowercase letter or _, then lowercase letters, digits, _ or -, at most 32 characters",
        ));
    }
    if name == "root" {
        return Err(invalid(
            "'root' cannot be created as a user",
            "set root's password with --root-password (\"root_password\": true) instead",
        ));
    }
    Ok(())
}
