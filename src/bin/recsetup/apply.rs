//! Apply: run a plan's steps in order and stop at the first that fails.
//! The result records every step (done, failed, or not run), so a front end
//! can show how far it got.

use crate::plan::{Action, Plan, Step};
use crate::target::{Root, Target};
use install_tools::error::{Code, Error, Result};
use target_root::is_root;
use install_tools::output::ErrorPayload;
use install_tools::session::Session;
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

pub const APPLY_SCHEMA_VERSION: u32 = 1;

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
    let mut progress=if opts.dry_run {None}else {
        if !is_root() {let e=Error::not_root();return(result(&plan,opts,warnings,plan_records(&plan,Status::NotRun),Some(&e)),Some(e));}
        match crate::progress::Progress::open(&plan) {
            Ok(p)=>Some(p),Err(e)=>return(result(&plan,opts,warnings,plan_records(&plan,Status::NotRun),Some(&e)),Some(e)),
        }
    };
    let needs_passwords=plan.steps.iter().any(|step|!progress.as_ref().is_some_and(|p|p.done(step)) && matches!(&step.action,Action::Chroot{passwords:true,..}));
    let preflight = || -> Result<Option<String>> {
        if !is_root() {
            return Err(Error::not_root().fix("run recsetup apply as root on the live system"));
        }
        if !needs_passwords {
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
        if progress.as_ref().is_some_and(|p|p.done(step)) {
            records.push(record(step,Status::Done,Some((0,"Completed by this installation's earlier attempt".into(),String::new()))));
            continue;
        }
        match run(&plan.root, step, passwords.as_deref()) {
            Ok(out) => {
                match progress.as_mut().unwrap().completed(step) {
                    Ok(())=>records.push(record(step,Status::Done,out)),
                    Err(e)=>{records.push(record(step,Status::Failed,out));failure=Some(e);},
                }
            },
            Err((err, out)) => {
                records.push(record(step, Status::Failed, out));
                failure = Some(err);
            }
        }
    }
    if failure.is_none() {
        if let Err(e)=progress.as_mut().unwrap().finish(){failure=Some(e);}
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

fn run(root: &Root, step: &Step, passwords: Option<&str>) -> std::result::Result<Output, (Error, Output)> {
    match &step.action {
        Action::Chroot { argv, passwords: wants } => {
            // The chroot recchroot enters: the host's /proc, /sys, /dev, /run
            // and DNS, and the target's own PATH.
            let mut session = Session::begin("recsetup").map_err(|e| (e, None))?;
            session.prepare_chroot(root.path()).map_err(|e| (e, None))?;
            let stdin = if *wants { passwords } else { None };
            let result = exec(session.chroot_command(root.path(), argv), &argv[0], stdin);
            match (session.close(), result) {
                (Err(e), Ok(out)) => Err((e, out)),
                (Err(e), Err(failed)) => {
                    eprintln!("recsetup: {e}");
                    Err(failed)
                }
                (Ok(()), result) => result,
            }
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
    // Dropping the pipe closes it, so the child sees the end of its input.
    let written = match (stdin, child.stdin.take()) {
        (Some(input), Some(mut pipe)) => pipe.write_all(input.as_bytes()),
        _ => Ok(()),
    };
    let output = child.wait_with_output().map_err(|e| (Error::command_failed(name, e), None))?;
    // A signal death has no exit code: -1 in the record.
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let failed = if !output.status.success() {
        Some(Error::tool_failed(name, &format!("{} ({})", stderr.trim(), target_root::status_text(output.status))))
    } else if let Err(e) = written {
        Some(Error::tool_failed(name, &format!("it did not read its input: {e}")))
    } else {
        None
    };
    match failed {
        None => Ok(Some((code, stdout, stderr))),
        Some(err) => Err((
            err.fix("fix the cause shown in the step's output, then run the plan again; completed steps are reused from installation progress"),
            Some((code, stdout, stderr)),
        )),
    }
}
