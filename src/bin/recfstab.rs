//! CLI entry point for recfstab.

use clap::Parser;
use install_tools::fstab::{generate, report, IdType, Write};
use install_tools::output::{to_json, ErrorPayload};
use std::process::ExitCode;

/// Command-line arguments for recfstab.
#[derive(Parser, Debug)]
#[command(name = "recfstab")]
#[command(version)]
#[command(about = "Generate fstab from mounted filesystems (like genfstab)")]
#[command(
    long_about = "Reads mounted filesystems under a root directory and outputs \
    fstab entries naming each partition by PARTUUID (or filesystem UUID/LABEL, or PARTLABEL). \
    PARTUUID is what the boot entries name the root by, and it survives the reformat an A/B \
    update makes. Designed for system \
    installation workflows where you need to generate /etc/fstab for a newly installed system."
)]
#[command(after_help = "Examples:\n  recfstab /mnt                   Preview entries; leave fstab unchanged\n  recfstab /mnt --write           Append entries to /mnt/etc/fstab\n  recfstab /mnt --write --replace  Replace entries for these mount points\n\nMount the target's root, boot and other filesystems before generating entries.\nPlain stdout is valid fstab text, so 'recfstab /mnt > fstab.preview' also works.")]
struct Args {
    /// Root directory to scan for mounted filesystems (e.g., /mnt)
    root: String,

    /// Use filesystem LABEL instead of PARTUUID for device identification
    #[arg(short = 'L', long, group = "id")]
    label: bool,

    /// Use the filesystem UUID instead of PARTUUID
    #[arg(short = 'U', long, group = "id")]
    uuid: bool,

    /// Use partition UUID (PARTUUID), the default
    #[arg(short = 'p', long, group = "id")]
    partuuid: bool,

    /// Use partition LABEL (PARTLABEL) instead of PARTUUID
    #[arg(short = 't', long, group = "id")]
    partlabel: bool,

    /// Append the entries to <ROOT>/etc/fstab; refused (E036) when it already has
    /// entries for the same mount points
    #[arg(short = 'w', long)]
    write: bool,

    /// With --write: replace the fstab's entries for those mount points
    #[arg(long, requires = "write")]
    replace: bool,

    /// Print the entries, the fstab's current entries and warnings as JSON
    /// (errors go to stderr as JSON)
    #[arg(long)]
    json: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();

    let id_type = if args.label {
        IdType::Label
    } else if args.uuid {
        IdType::Uuid
    } else if args.partlabel {
        IdType::Partlabel
    } else {
        IdType::Partuuid
    };

    let write = match (args.write, args.replace) {
        (false, _) => Write::No,
        (true, false) => Write::Append,
        (true, true) => Write::Replace,
    };
    let outcome = if args.json || write != Write::No {
        report(&args.root, id_type, write).map(|result| {
            if args.json {
                println!("{}", to_json(&result));
            } else {
                for warning in &result.warnings {
                    eprintln!("recfstab: warning: {warning}");
                }
                eprintln!("recfstab: wrote {} entries to {}", result.entries.len(), result.fstab_path);
            }
        })
    } else {
        generate(&args.root, id_type).map(|entries| {
            for entry in entries {
                println!("{entry}");
            }
        })
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if args.json {
                eprintln!("{}", to_json(&ErrorPayload::of(&e)));
            } else {
                eprintln!("recfstab: {}", e);
            }
            ExitCode::from(e.code.exit_status())
        }
    }
}
