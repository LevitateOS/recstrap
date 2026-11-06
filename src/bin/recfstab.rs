//! CLI entry point for recfstab.

use clap::Parser;
use install_tools::fstab::{generate, IdType};
use std::process::ExitCode;

/// Command-line arguments for recfstab.
#[derive(Parser, Debug)]
#[command(name = "recfstab")]
#[command(version)]
#[command(about = "Generate fstab from mounted filesystems (like genfstab)")]
#[command(
    long_about = "Reads mounted filesystems under a root directory and outputs \
    fstab entries with UUIDs (or LABELs/PARTUUIDs/PARTLABELs). Designed for system \
    installation workflows where you need to generate /etc/fstab for a newly installed system."
)]
struct Args {
    /// Root directory to scan for mounted filesystems (e.g., /mnt)
    root: String,

    /// Use filesystem LABEL instead of UUID for device identification
    #[arg(short = 'L', long, group = "id")]
    label: bool,

    /// Use partition UUID (PARTUUID) instead of filesystem UUID
    #[arg(short = 'p', long, group = "id")]
    partuuid: bool,

    /// Use partition LABEL (PARTLABEL) instead of filesystem UUID
    #[arg(short = 't', long, group = "id")]
    partlabel: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();

    let id_type = if args.label {
        IdType::Label
    } else if args.partuuid {
        IdType::Partuuid
    } else if args.partlabel {
        IdType::Partlabel
    } else {
        IdType::Uuid
    };

    match generate(&args.root, id_type) {
        Ok(entries) => {
            for entry in entries {
                println!("{entry}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("recfstab: {}", e);
            ExitCode::from(e.code.exit_status())
        }
    }
}
