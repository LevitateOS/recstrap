//! Integration tests for recstrap binary.
//!
//! These tests run the actual binary and verify behavior.
//!
//! Target, protected-path and root checks run unprivileged; the checks after
//! the root check are `#[ignore]`d and run as root with `--include-ignored`.

use distro_spec::shared::is_root;
use leviso_cheat_test::cheat_aware;
use std::process::Command;

/// Helper to run recstrap with given args
fn run_recstrap(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_recstrap"))
        .args(args)
        .output()
        .expect("Failed to execute recstrap")
}

#[cheat_aware(
    protects = "Only root can run system extraction (security boundary)",
    severity = "HIGH",
    ease = "EASY",
    cheats = ["Remove root check entirely", "Add --no-root-check flag"],
    consequence = "Unprivileged users attempt extraction and get cryptic permission errors",
    legitimate_change = "Root requirement is fundamental to system installation. \
        This check should never be bypassed."
)]
#[test]
fn test_root_check_without_root() {
    if is_root() {
        eprintln!("skipped: running as root, the root check cannot fail");
        return;
    }
    let target = tempfile::tempdir().expect("create unprotected target");
    let output = run_recstrap(&[target.path().to_str().unwrap()]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E008:"),
        "Expected E008 (must run as root), stderr was: {}",
        stderr
    );
    assert!(
        stderr.contains("root"),
        "Error should mention root, stderr was: {}",
        stderr
    );
    assert_eq!(
        output.status.code(),
        Some(8),
        "Exit code should be 8 for E008"
    );
}

#[test]
fn test_nonexistent_directory() {
    let output = run_recstrap(&["/nonexistent/path/12345"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E001:"),
        "Expected E001, stderr was: {}",
        stderr
    );
    assert!(stderr.contains("does not exist"), "stderr was: {}", stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "Exit code should be 1 for E001"
    );
}

#[test]
fn test_file_instead_of_directory() {
    let output = run_recstrap(&["/etc/passwd"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E002:"),
        "Expected E002, stderr was: {}",
        stderr
    );
    assert!(stderr.contains("not a directory"), "stderr was: {}", stderr);
    assert_eq!(
        output.status.code(),
        Some(2),
        "Exit code should be 2 for E002"
    );
}

#[cheat_aware(
    protects = "User's root filesystem is never overwritten",
    severity = "CRITICAL",
    ease = "EASY",
    cheats = ["Remove / from PROTECTED_PATHS", "Skip path validation when --force is used"],
    consequence = "User runs 'recstrap /' and loses their entire system",
    legitimate_change = "The root filesystem should NEVER be a valid target. \
        The list is PROTECTED_PATHS in distro-spec/src/shared/paths.rs."
)]
#[test]
fn test_protected_path_root() {
    let output = run_recstrap(&["/"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E010:"),
        "Expected E010 for /, stderr was: {}",
        stderr
    );
    assert_eq!(
        output.status.code(),
        Some(10),
        "Exit code should be 10 for E010"
    );
}

#[cheat_aware(
    protects = "User's /usr directory is never overwritten",
    severity = "CRITICAL",
    ease = "EASY",
    cheats = ["Remove /usr from PROTECTED_PATHS", "Allow --force to bypass protected paths"],
    consequence = "User runs 'recstrap /usr' and destroys their system binaries",
    legitimate_change = "System directories should NEVER be valid targets. \
        The list is PROTECTED_PATHS in distro-spec/src/shared/paths.rs."
)]
#[test]
fn test_protected_path_usr() {
    let output = run_recstrap(&["/usr"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E010:"),
        "Expected E010 for /usr, stderr was: {}",
        stderr
    );
    assert_eq!(
        output.status.code(),
        Some(10),
        "Exit code should be 10 for E010"
    );
}

#[cheat_aware(
    protects = "Unsupported rootfs formats fail with explicit diagnostics",
    severity = "HIGH",
    ease = "MEDIUM",
    cheats = ["Silently treat unknown extensions as valid", "Fall back to legacy squashfs handling"],
    consequence = "Users can extract wrong media format and get unpredictable failures",
    legitimate_change = "recstrap is EROFS-only; unsupported formats must fail with E016."
)]
#[test]
#[ignore = "needs root: runs past the root check"]
fn test_squashfs_extension_rejected() {
    let temp_dir = std::env::temp_dir().join("recstrap_test_erofs_only_target");
    let fake_rootfs = std::env::temp_dir().join("recstrap_test_erofs_only.squashfs");
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::remove_file(&fake_rootfs);
    let _ = std::fs::create_dir_all(&temp_dir);
    let _ = std::fs::write(&fake_rootfs, b"hsqs");

    let output = run_recstrap(&[
        "--force",
        "--allow-host-root-fs",
        "--rootfs",
        fake_rootfs.to_str().unwrap(),
        temp_dir.to_str().unwrap(),
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E016:"),
        "Expected E016 for unsupported extension, stderr was: {}",
        stderr
    );
    assert_eq!(
        output.status.code(),
        Some(16),
        "Exit code should be 16 for E016"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::remove_file(&fake_rootfs);
}

#[test]
#[ignore = "needs root: runs past the root check"]
fn test_rootfs_is_directory() {
    let temp_dir = std::env::temp_dir().join("recstrap_test_rootfs_dir");
    let fake_rootfs_dir = std::env::temp_dir().join("recstrap_test_fake_rootfs_dir");
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::remove_dir_all(&fake_rootfs_dir);
    let _ = std::fs::create_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&fake_rootfs_dir);

    let output = run_recstrap(&[
        "--force",
        "--allow-host-root-fs",
        "--rootfs",
        fake_rootfs_dir.to_str().unwrap(),
        temp_dir.to_str().unwrap(),
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E013:"),
        "Expected E013, stderr was: {}",
        stderr
    );
    assert_eq!(
        output.status.code(),
        Some(13),
        "Exit code should be 13 for E013"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::remove_dir_all(&fake_rootfs_dir);
}

#[cheat_aware(
    protects = "Non-empty target directory requires explicit --force flag",
    severity = "HIGH",
    ease = "MEDIUM",
    cheats = ["Silently overwrite existing files", "Auto-add --force when target has files"],
    consequence = "User accidentally overwrites existing data without confirmation",
    legitimate_change = "The --force flag exists for intentional overwrites. \
        Default behavior must protect user data."
)]
#[test]
#[ignore = "needs root: runs past the root check"]
fn test_target_not_empty() {
    let temp_dir = std::env::temp_dir().join("recstrap_test_notempty");
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);
    let _ = std::fs::write(temp_dir.join("test_file"), b"test");

    let output = run_recstrap(&[
        "--allow-host-root-fs",
        "--allow-non-mountpoint",
        temp_dir.to_str().unwrap(),
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E009:"), "Expected E009, stderr was: {}", stderr);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[cheat_aware(
    protects = "--force flag correctly bypasses safety checks when user explicitly requests",
    severity = "MEDIUM",
    ease = "MEDIUM",
    cheats = ["Make --force bypass protected path checks too", "Ignore --force flag entirely"],
    consequence = "--force doesn't work as documented, or bypasses too many safety checks",
    legitimate_change = "--force should skip empty/mount-point checks but NEVER protected paths. \
        Verify PROTECTED_PATHS is checked regardless of --force."
)]
#[test]
#[ignore = "needs root: runs past the root check"]
fn test_force_flag_allows_nonempty() {
    let temp_dir = std::env::temp_dir().join("recstrap_test_force");
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);
    let _ = std::fs::write(temp_dir.join("test_file"), b"test");

    let output = run_recstrap(&["--force", "--allow-host-root-fs", temp_dir.to_str().unwrap()]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("E009:") && !stderr.contains("E011:"),
        "--force should skip empty and mount point checks, stderr was: {}",
        stderr
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_mnt_is_allowed() {
    if !std::path::Path::new("/mnt").is_dir() {
        eprintln!("skipped: no /mnt on this host");
        return;
    }
    let output = run_recstrap(&["/mnt"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("E010:"), "/mnt should not be protected, stderr was: {}", stderr);
}
