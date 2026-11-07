//! Integration tests for recsetup binary.
//!
//! `plan` only reads, so these run unprivileged against a minimal fake
//! target in a temp dir.

use std::path::Path;
use std::process::Command;

/// A target recsetup can detect: systemd with bootctl, and a /boot directory.
/// Returns the target and a kernel file outside it.
fn fake_target() -> (tempfile::TempDir, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("create target");
    for dir in ["usr/lib/systemd", "usr/bin", "etc", "boot"] {
        std::fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    std::fs::write(root.path().join("usr/lib/systemd/systemd"), b"").unwrap();
    std::fs::write(root.path().join("usr/bin/bootctl"), b"").unwrap();
    std::fs::write(root.path().join("etc/os-release"), b"ID=levitateos\nNAME=LevitateOS\n").unwrap();
    let media = tempfile::tempdir().expect("create kernel dir");
    std::fs::write(media.path().join("vmlinuz"), b"kernel").unwrap();
    (root, media)
}

fn plan(root: &Path, kernel: &Path, boot_args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_recsetup"))
        .args(["plan", "--root"])
        .arg(root)
        .arg("--kernel")
        .arg(kernel)
        .arg("--no-initrd")
        .args(boot_args)
        .output()
        .expect("Failed to execute recsetup")
}

fn assert_root_not_mounted_refused(boot_args: &[&str]) {
    let (root, media) = fake_target();
    let output = plan(root.path(), &media.path().join("vmlinuz"), boot_args);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E011:") && stderr.contains("mount the installed system's root partition at --root"),
        "Expected E011 with the mount remediation, stderr was: {}",
        stderr
    );
    assert_eq!(output.status.code(), Some(11), "Exit code should be 11 for E011");
}

#[test]
fn test_mutable_boot_needs_root_mounted() {
    assert_root_not_mounted_refused(&["--boot", "mutable", "--root-device", "/dev/null"]);
}

#[test]
fn test_ab_boot_needs_root_mounted() {
    assert_root_not_mounted_refused(&["--boot", "ab", "--slot-a", "/dev/null", "--slot-b", "/dev/zero"]);
}
