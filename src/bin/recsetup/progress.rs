//! Durable progress for one personalization request. Secrets are never stored.
use crate::plan::{Plan, Step};
use install_tools::error::{Code, Error, Result};
use serde::{Deserialize, Serialize};
use std::os::{
    fd::AsRawFd,
    unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::PathBuf,
};

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    filesystem: String,
    request: serde_json::Value,
    completed: Vec<String>,
    finished: bool,
}
pub struct Progress {
    file: PathBuf,
    saved: Saved,
    _lock: File,
}
fn fail(message: impl Into<String>) -> Error {
    Error::new(Code::UnsafeTargetPath,message).fix("resume the original installation request on its original target; do not reset an installed system")
}
// Media/device paths may change when resuming the same target. Recab owns
// validation of their actual partition identities and frozen artifact hashes.
fn identity(mut request: serde_json::Value) -> serde_json::Value {
    if let Some(boot) = request.get_mut("boot").and_then(|b| b.as_object_mut()) {
        for path in ["kernel", "initrd", "slot_a", "slot_b", "root_device"] {
            boot.remove(path);
        }
    }
    request
}
impl Progress {
    pub fn open(plan: &Plan) -> Result<Self> {
        // Do not follow image-supplied directory links into the live host.
        let mut dir = plan.root.path().to_path_buf();
        for name in ["var", "lib", "recsetup"] {
            dir.push(name);
            match fs::symlink_metadata(&dir) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir(&dir)?;
                    fs::set_permissions(
                        &dir,
                        fs::Permissions::from_mode(if name == "recsetup" { 0o700 } else { 0o755 }),
                    )?;
                }
                Ok(m) if m.is_dir() && m.uid() == 0 && m.mode() & 0o022 == 0 => {}
                Ok(_) => {
                    return Err(fail(
                        "installation progress needs real root-owned state directories",
                    ))
                }
                Err(e) => return Err(e.into()),
            }
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(dir.join("apply.lock"))?;
        if !lock.metadata()?.is_file()
            || lock.metadata()?.uid() != 0
            || lock.metadata()?.nlink() != 1
        {
            return Err(fail("unsafe installation progress lock"));
        }
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(fail("another recsetup apply owns this installation"));
        }
        let table = target_root::mount_table()?;
        let mount = target_root::mount_holding(&table, plan.root.path())?
            .ok_or_else(|| fail("installation target has no filesystem"))?;
        let filesystem = match target_root::blkid_tag(std::path::Path::new(&mount.source), "UUID")?
        {
            Some(uuid) => format!("UUID={uuid}:{}", mount.root.display()),
            None => format!("{}:{}", mount.source, plan.root.path().display()),
        };
        let file = dir.join("apply.json");
        let request = serde_json::to_value(&plan.request).map_err(|e| fail(e.to_string()))?;
        let saved = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&file)
        {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
            Ok(input) => {
                let m = input.metadata()?;
                if !m.is_file()
                    || m.uid() != 0
                    || m.mode() & 0o077 != 0
                    || m.len() > 4 * 1024 * 1024
                {
                    return Err(fail("unsafe/oversized installation progress"));
                }
                let mut text = String::new();
                input.take(4 * 1024 * 1024 + 1).read_to_string(&mut text)?;
                Some(serde_json::from_str::<Saved>(&text).map_err(|e| fail(e.to_string()))?)
            }
        };
        let saved = match saved {
            Some(s) if s.version != 1 || s.filesystem != filesystem => {
                return Err(fail(
                    "installation progress belongs to a different filesystem/version",
                ));
            }
            Some(mut s) if identity(s.request.clone()) == identity(request.clone()) => {
                s.request = request;
                s
            }
            Some(s) if !s.finished && !s.completed.is_empty() => {
                return Err(fail("an unfinished personalization request must be resumed before changing its settings"));
            }
            _ => {
                if plan
                    .root
                    .read("/var/lib/recab/installation.json")?
                    .is_some()
                    || plan.root.read("/var/lib/recab/state.json")?.is_some()
                {
                    return Err(fail("personalization cannot reset an initialized system"));
                }
                Saved {
                    version: 1,
                    filesystem,
                    request,
                    completed: vec![],
                    finished: false,
                }
            }
        };
        let progress = Self {
            file,
            saved,
            _lock: lock,
        };
        progress.save()?;
        Ok(progress)
    }
    pub fn done(&self, step: &Step) -> bool {
        // Even after a lost success response, recab must revalidate both roots
        // and entries before the installer can claim it is ready to reboot.
        if matches!(&step.action,crate::plan::Action::Host{argv} if argv.first().is_some_and(|p|p=="recab") && argv.get(1).is_some_and(|p|p=="init"))
        {
            return false;
        }
        self.saved.completed.contains(&step.command)
    }
    pub fn completed(&mut self, step: &Step) -> Result<()> {
        if !self.saved.completed.contains(&step.command) {
            self.saved.completed.push(step.command.clone());
            self.save()?;
        }
        Ok(())
    }
    pub fn finish(&mut self) -> Result<()> {
        self.saved.finished = true;
        self.save()
    }
    fn save(&self) -> Result<()> {
        let data = serde_json::to_vec(&self.saved).map_err(|e| fail(e.to_string()))?;
        target_root::write_durable(&self.file, &data, Some(0o600))?;
        Ok(())
    }
}
