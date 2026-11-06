//! CLI entry point for recfstab.

use clap::Parser;
use recfstab::{run, IdType};
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
    #[arg(short = 'L', long, conflicts_with_all = ["partuuid", "partlabel"])]
    label: bool,

    /// Use partition UUID (PARTUUID) instead of filesystem UUID
    #[arg(short = 'p', long, conflicts_with_all = ["label", "partlabel"])]
    partuuid: bool,

    /// Use partition LABEL (PARTLABEL) instead of filesystem UUID
    #[arg(short = 't', long, conflicts_with_all = ["label", "partuuid"])]
    partlabel: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();

    // Determine identifier type from flags
    let id_type = if args.label {
        IdType::Label
    } else if args.partuuid {
        IdType::Partuuid
    } else if args.partlabel {
        IdType::Partlabel
    } else {
        IdType::Uuid
    };

    match run(&args.root, id_type) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("recfstab: {}", e);
            ExitCode::FAILURE
        }
    }
}
