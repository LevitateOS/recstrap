//! recsetup - set up an extracted system: time zone, locale, keymap,
//! hostname, accounts, systemd-boot and its entries, machine id and services.
//!
//!   recsetup inspect --root /mnt --json      # the target, and what can be chosen
//!   recsetup plan --root /mnt --hostname pc  # the exact commands, run nothing
//!   recsetup apply --root /mnt ...           # run them, stop at the first failure
//!
//! It works out the init system (systemd or OpenRC) and the libc from the
//! target itself, and picks each step's commands from what the target ships.

mod apply;
mod plan;
mod progress;
mod request;
mod target;

use apply::ApplyOptions;
use install_tools::output::{to_json, ErrorPayload};
use clap::{Parser, Subcommand, ValueEnum};
use install_tools::error::{Error, Result};
use target_root::is_protected_path;
use install_tools::session::resolve_target;
use request::RequestFlags;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use target::Root;

#[derive(Parser)]
#[command(name = "recsetup")]
#[command(version)]
#[command(about = "Set up an extracted system: time zone, locale, keymap, hostname, accounts, boot and services")]
#[command(after_help = "Start here:\n  recsetup inspect --root /mnt\n  recsetup inspect --root /mnt --list timezones\n  recsetup plan --root /mnt --hostname my-pc --timezone Europe/Brussels\n  recsetup apply --root /mnt --hostname my-pc --timezone Europe/Brussels\n\ninspect and plan leave the target unchanged. apply runs the requested changes.\nFor users/passwords and boot options: recsetup apply --help")]
struct Cli {
    /// Emit machine-readable JSON (errors too, on stderr)
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Describe the target and list what can be chosen (time zones, locales, keymaps, ...)
    Inspect {
        /// The extracted system's root
        #[arg(long, default_value = "/mnt")]
        root: PathBuf,
        /// Show the available choices as text, one per line
        #[arg(long, value_enum, conflicts_with = "json")]
        list: Option<ChoiceList>,
    },
    /// Check a request and print the exact commands, without running them
    Plan {
        /// The extracted system's root
        #[arg(long, default_value = "/mnt")]
        root: PathBuf,
        #[command(flatten)]
        request: RequestFlags,
    },
    /// Run the plan's commands in order, stopping at the first failure
    Apply {
        /// The extracted system's root
        #[arg(long, default_value = "/mnt")]
        root: PathBuf,
        #[command(flatten)]
        request: RequestFlags,
        /// Show the steps and what apply would refuse, without running them
        #[arg(long)]
        dry_run: bool,
        /// Read the passwords as name:password lines from stdin (never pass them as arguments)
        #[arg(long)]
        passwords_stdin: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ChoiceList {
    Timezones,
    Locales,
    Keymaps,
    Shells,
    Groups,
    Users,
    Services,
}

fn report_error(err: &Error, json: bool) -> ExitCode {
    if json {
        eprintln!("{}", to_json(&ErrorPayload::of(err)));
    } else {
        eprintln!("recsetup: {err}");
    }
    ExitCode::from(err.code.exit_status())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command, cli.json) {
        Ok(code) => code,
        Err(err) => report_error(&err, cli.json),
    }
}

/// The target root; `plan` and `apply` refuse the host's own directories.
fn open_root(path: &Path, writes: bool) -> Result<Root> {
    let resolved = resolve_target(&path.to_string_lossy())?;
    if writes && is_protected_path(&resolved) {
        return Err(Error::protected_path(&resolved.to_string_lossy())
            .fix("point --root at the mounted target, such as /mnt"));
    }
    Ok(Root::new(resolved))
}

fn run(command: Commands, json: bool) -> Result<ExitCode> {
    match command {
        Commands::Inspect { root, list } => {
            let root = open_root(&root, false)?;
            let target = target::detect(&root)?;
            let choices = target::choices(&root, &target)?;
            if let Some(list) = list {
                print_choices(&choices, list);
            } else if json {
                #[derive(Serialize)]
                struct Inspect<'a> {
                    schema_version: u32,
                    target: &'a target::Target,
                    choices: &'a target::Choices,
                }
                println!("{}", to_json(&Inspect { schema_version: 1, target: &target, choices: &choices }));
            } else {
                print_target(&target);
                println!("  time zones: {}", choices.timezones.len());
                println!("  locales:    {}", choices.locales.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join(" "));
                println!("  keymaps:    {}", choices.keymaps.len());
                println!("  shells:     {}", choices.shells.join(" "));
                println!("  services:   {}", choices.services.len());
                println!("\nList choices: recsetup inspect --root '{}' --list <CATEGORY>", target.root.replace('\'', "'\\''"));
                println!("  Categories: timezones, locales, keymaps, shells, groups, users, services");
                println!("Preview changes: recsetup plan --root '{}' --hostname my-pc", target.root.replace('\'', "'\\''"));
            }
            Ok(ExitCode::SUCCESS)
        }
        Commands::Plan { root, request } => {
            let plan = make_plan(&root, request)?;
            if json {
                println!("{}", to_json(&plan));
            } else {
                print_target(&plan.target);
                println!("Preview only: no changes made.");
                print_steps(plan.steps.iter().map(|s| (s, None)));
                print_warnings(&plan.warnings);
            }
            Ok(ExitCode::SUCCESS)
        }
        Commands::Apply { root, request, dry_run, passwords_stdin } => {
            let plan = make_plan(&root, request)?;
            let (result, err) = apply::apply(plan, &ApplyOptions { dry_run, passwords_stdin });
            if json {
                println!("{}", to_json(&result));
            } else {
                print_target(&result.target);
                if dry_run {
                    println!("Dry run: no changes made.");
                }
                print_steps(result.steps.iter().map(|r| (&r.step, Some(r))));
                print_warnings(&result.warnings);
                if !dry_run && err.is_none() && !result.steps.is_empty() {
                    println!("\nCompleted {} setup steps.", result.steps.len());
                }
            }
            Ok(match err {
                Some(err) => report_error(&err, json),
                None => ExitCode::SUCCESS,
            })
        }
    }
}

fn print_choices(choices: &target::Choices, list: ChoiceList) {
    let lines: Vec<String> = match list {
        ChoiceList::Timezones => choices.timezones.iter().map(|v| v.name.clone()).collect(),
        ChoiceList::Locales => choices.locales.iter().map(|v| v.name.clone()).collect(),
        ChoiceList::Keymaps => choices.keymaps.clone(),
        ChoiceList::Shells => choices.shells.clone(),
        ChoiceList::Groups => choices.groups.clone(),
        ChoiceList::Users => choices.users.clone(),
        ChoiceList::Services => choices.services.iter().map(|v| {
            if v.enabled_in.is_empty() {
                v.name.clone()
            } else {
                format!("{} (enabled in: {})", v.name, v.enabled_in.join(", "))
            }
        }).collect(),
    };
    if lines.is_empty() {
        println!("No choices found in this target.");
    }
    for line in lines {
        println!("{line}");
    }
}

fn make_plan(root: &Path, flags: RequestFlags) -> Result<plan::Plan> {
    let root = open_root(root, true)?;
    let request = flags.into_request()?;
    let target = target::detect(&root)?;
    plan::build(root, target, request)
}

fn print_target(target: &target::Target) {
    println!(
        "recsetup: {} at {} ({}, {})",
        target.os_name, target.root, target.profile.init, target.profile.libc
    );
}

fn print_steps<'a>(steps: impl Iterator<Item = (&'a plan::Step, Option<&'a apply::StepRecord>)>) {
    let mut any = false;
    for (step, record) in steps {
        any = true;
        let mark = match record.map(|r| r.status) {
            None | Some(apply::Status::Planned) => "",
            Some(apply::Status::Done) => " [done]",
            Some(apply::Status::Failed) => " [FAILED]",
            Some(apply::Status::NotRun) => " [not run]",
        };
        println!("\n{}. {}{mark}", step.index, step.description);
        for line in step.command.lines() {
            println!("   {line}");
        }
        if let Some(stderr) = record.and_then(|r| r.stderr.as_deref()).filter(|s| !s.trim().is_empty()) {
            if record.is_some_and(|r| r.status == apply::Status::Failed) {
                for line in stderr.lines() {
                    println!("   ! {line}");
                }
            }
        }
    }
    if !any {
        println!("\nNothing to do: the request asks for nothing.");
    }
}

fn print_warnings(warnings: &[String]) {
    if !warnings.is_empty() {
        println!("\nWarnings:");
        for w in warnings {
            println!("  {w}");
        }
    }
}
