//! Integration tests for recsetup binary.
//!
//! `plan` only reads, so these run unprivileged against a minimal fake
//! target in a temp dir.

use std::path::Path;
use std::process::Command;

/// A target recsetup can read: a systemd profile, bootctl, Recipe's system setup
/// program, and a /boot directory.
/// Returns the target and a kernel file outside it.
fn fake_target() -> (tempfile::TempDir, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("create target");
    for dir in ["usr/bin", "usr/libexec", "etc", "boot"] {
        std::fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    std::fs::write(root.path().join("usr/bin/bootctl"), b"").unwrap();
    std::fs::write(root.path().join("usr/libexec/recipe-system-setup"), b"").unwrap();
    std::fs::write(
        root.path().join("etc/os-release"),
        b"ID=levitateos\nNAME=LevitateOS\nREC_PACKAGES=rpm\nREC_INIT=systemd\nREC_LIBC=glibc\nREC_PRIVILEGE=sudo\nREC_DESKTOP=plasma\nREC_KERNEL_LINE=lts\nREC_FLAVOUR=base\n",
    )
    .unwrap();
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

#[test]
fn inspect_lists_choices_as_text_and_keeps_full_json() {
    let (root, _) = fake_target();
    let zoneinfo = root.path().join("usr/share/zoneinfo/Europe");
    std::fs::create_dir_all(&zoneinfo).unwrap();
    std::fs::write(zoneinfo.join("Brussels"), b"TZif").unwrap();
    let hostname = root.path().join("etc/hostname");
    std::fs::write(&hostname, "unchanged\n").unwrap();

    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_recsetup"))
            .args(["inspect", "--root"])
            .arg(root.path())
            .args(args)
            .output()
            .unwrap()
    };
    let text = run(&["--list", "timezones"]);
    assert!(text.status.success(), "{}", String::from_utf8_lossy(&text.stderr));
    assert!(String::from_utf8_lossy(&text.stdout).lines().any(|line| line == "Europe/Brussels"));
    let json = run(&["--json"]);
    assert!(json.status.success());
    let document: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(document["schema_version"], 1);
    assert!(document["choices"]["timezones"].as_array().unwrap().iter().any(|zone| zone["name"] == "Europe/Brussels"));
    assert_eq!(std::fs::read_to_string(hostname).unwrap(), "unchanged\n");
    assert_eq!(run(&["--list", "timezones", "--json"]).status.code(), Some(2));
}
