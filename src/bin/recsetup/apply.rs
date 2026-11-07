//! Apply: run a plan's steps in order and stop at the first that fails.
//! The result records every step (done, failed, or not run), so a front end
//! can show how far it got.

use crate::plan::{Action, Plan, Step};
use crate::target::{Root, Target};
use install_tools::error::{Code, Error, Result};
use install_tools::guard::is_root;
use install_tools::output::ErrorPayload;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const APPLY_SCHEMA_VERSION: u32 = 1;

const TARGET_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Dry run: shown, not run.
    Planned,
    Done,
    Failed,
    /// An earlier step failed.
    NotRun,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepRecord {
    #[serde(flatten)]
    pub step: Step,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_status: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyResult {
    pub schema_version: u32,
    pub dry_run: bool,
    pub target: Target,
    pub steps: Vec<StepRecord>,
    pub warnings: Vec<String>,
    /// The failure that stopped apply; `None` when every step ran.
    pub error: Option<ErrorPayload>,
}

pub struct ApplyOptions {
    pub dry_run: bool,
    pub passwords_stdin: bool,
}

/// Run the plan. On failure the result still lists every step, and the
/// error is returned beside it.
pub fn apply(plan: Plan, opts: &ApplyOptions) -> (ApplyResult, Option<Error>) {
    let mut warnings = plan.warnings.clone();
    let preflight = || -> Result<Option<String>> {
        if !is_root() {
            return Err(Error::not_root().fix("run recsetup apply as root on the live system"));
        }
        if plan.password_users.is_empty() {
            return Ok(None);
        }
        if !opts.passwords_stdin {
            return Err(Error::new(
                Code::PasswordInput,
                format!("the plan sets the password of {}, but --passwords-stdin was not given", plan.password_users.join(", ")),
            )
            .fix("pipe one name:password line per account into recsetup apply --passwords-stdin"));
        }
        read_passwords(&plan.password_users).map(Some)
    };
    let passwords = if opts.dry_run {
        if let Err(e) = preflight_dry(&plan, opts) {
            warnings.push(e.to_string());
        }
        None
    } else {
        match preflight() {
            Ok(p) => p,
            Err(e) => return (result(&plan, opts, warnings, plan_records(&plan, Status::NotRun), Some(&e)), Some(e)),
        }
    };

    if opts.dry_run {
        return (result(&plan, opts, warnings, plan_records(&plan, Status::Planned), None), None);
    }

    let mut records = Vec::with_capacity(plan.steps.len());
    let mut failure = None;
    for step in &plan.steps {
        if failure.is_some() {
            records.push(record(step, Status::NotRun, None));
            continue;
        }
        match run(&plan.root, step, passwords.as_deref()) {
            Ok(out) => records.push(record(step, Status::Done, out)),
            Err((err, out)) => {
                records.push(record(step, Status::Failed, out));
                failure = Some(err);
            }
        }
    }
    (result(&plan, opts, warnings, records, failure.as_ref()), failure)
}

/// What a real apply would refuse, reported by a dry run as a warning.
fn preflight_dry(plan: &Plan, opts: &ApplyOptions) -> Result<()> {
    if !is_root() {
        return Err(Error::not_root().fix("run recsetup apply as root on the live system"));
    }
    if !plan.password_users.is_empty() && !opts.passwords_stdin {
        return Err(Error::new(Code::PasswordInput, "apply will need --passwords-stdin for this plan"));
    }
    Ok(())
}

fn result(plan: &Plan, opts: &ApplyOptions, warnings: Vec<String>, steps: Vec<StepRecord>, err: Option<&Error>) -> ApplyResult {
    ApplyResult {
        schema_version: APPLY_SCHEMA_VERSION,
        dry_run: opts.dry_run,
        target: plan.target.clone(),
        steps,
        warnings,
        error: err.map(ErrorPayload::of),
    }
}

fn plan_records(plan: &Plan, status: Status) -> Vec<StepRecord> {
    plan.steps.iter().map(|s| record(s, status, None)).collect()
}

/// A finished command's exit status, stdout and stderr.
type Output = Option<(i32, String, String)>;

fn record(step: &Step, status: Status, out: Output) -> StepRecord {
    let (exit_status, stdout, stderr) = match out {
        Some((code, stdout, stderr)) => (Some(code), Some(stdout), Some(stderr)),
        None => (None, None, None),
    };
    StepRecord { step: step.clone(), status, exit_status, stdout, stderr }
}

/// `name:password` lines from stdin, one for each planned account, as the
/// chpasswd input in plan order.
fn read_passwords(users: &[String]) -> Result<String> {
    let bad = |message: String| {
        Error::new(Code::PasswordInput, message)
            .fix(format!("pipe exactly one name:password line for each of: {}", users.join(", ")))
    };
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| bad(format!("cannot read the passwords from stdin: {e}")))?;
    let mut given: BTreeMap<String, String> = BTreeMap::new();
    for line in input.lines().filter(|l| !l.is_empty()) {
        let Some((name, password)) = line.split_once(':') else {
            return Err(bad("a stdin line has no ':' between the name and the password".into()));
        };
        if password.is_empty() {
            return Err(bad(format!("the password for '{name}' is empty")));
        }
        if !users.iter().any(|u| u == name) {
            return Err(bad(format!("stdin has a password for '{name}', which the plan does not set")));
        }
        if given.insert(name.to_string(), password.to_string()).is_some() {
            return Err(bad(format!("stdin has two passwords for '{name}'")));
        }
    }
    let mut out = String::new();
    for user in users {
        let password = given.get(user).ok_or_else(|| bad(format!("stdin has no password for '{user}'")))?;
        out.push_str(&format!("{user}:{password}\n"));
    }
    Ok(out)
}

fn recchroot() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("recchroot")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("recchroot"))
}

fn run(root: &Root, step: &Step, passwords: Option<&str>) -> std::result::Result<Output, (Error, Output)> {
    let plain = |r: Result<()>| r.map(|()| None).map_err(|e| (e, None));
    match &step.action {
        Action::Write { path, content, mode } => plain(write(root, path, content.as_bytes(), *mode)),
        Action::Copy { from, from_target, to } => plain((|| {
            let source = if *from_target { root.resolve(&from.to_string_lossy())? } else { from.clone() };
            let bytes = fs::read(&source).map_err(|e| {
                Error::new(Code::BootFileMissing, format!("cannot read '{}': {e}", source.display()))
            })?;
            write(root, to, &bytes, 0o644)
        })()),
        Action::Chroot { argv, passwords: wants } => {
            let mut command = Command::new(recchroot());
            // The target's own directories, whatever the caller's PATH is
            // (Alpine keeps rc-update in /sbin).
            command.arg(root.path()).arg("--").args(argv).env("PATH", TARGET_PATH);
            let stdin = if *wants { passwords } else { None };
            exec(command, &argv[0], stdin)
        }
        Action::Host { argv } => {
            let mut command = Command::new(&argv[0]);
            command.args(&argv[1..]);
            exec(command, &argv[0], None)
        }
    }
}

fn exec(mut command: Command, name: &str, stdin: Option<&str>) -> std::result::Result<Output, (Error, Output)> {
    command
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| (Error::command_failed(name, e), None))?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // A child that exits without reading shows up in its status below.
        let _ = pipe.write_all(input.as_bytes());
    }
    let output = child.wait_with_output().map_err(|e| (Error::command_failed(name, e), None))?;
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if output.status.success() {
        Ok(Some((code, stdout, stderr)))
    } else {
        let err = Error::tool_failed(name, &stderr)
            .fix("fix the cause shown in the step's output, then run the plan again; finished steps are safe to repeat");
        Err((err, Some((code, stdout, stderr))))
    }
}


fn write(root: &Root, path: &str, content: &[u8], mode: u32) -> Result<()> {
    let host = root.resolve_entry(path)?;
    let parent = host.parent().map(Path::to_path_buf).unwrap_or_else(|| root.path().to_path_buf());
    fs::create_dir_all(&parent).map_err(|e| Error::create_failed(&parent, e))?;
    if fs::symlink_metadata(&host).is_ok_and(|m| m.is_dir()) {
        return Err(Error::new(Code::UnsafeTargetPath, format!("'{}' is a directory", host.display()))
            .fix("remove the directory, then run the plan again"));
    }
    let name = host.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = parent.join(format!(".{name}.recsetup"));
    let written = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(content)?;
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        file.sync_all()?;
        fs::rename(&tmp, &host)
    })();
    written.map_err(|e| {
        let _ = fs::remove_file(&tmp);
        Error::create_failed(&host, e)
    })
}
