//! Integration tests for recchroot binary.
//!
//! These tests run the actual binary and verify behavior.
//! Target and protected-path checks run unprivileged; nothing here mounts.
use std::process::Command;

/// Helper to run recchroot with given args
fn run_recchroot(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_recchroot"))
        .args(args)
        .output()
        .expect("Failed to execute recchroot")
}

#[test]
fn test_nonexistent_directory() {
    let output = run_recchroot(&["/nonexistent/path/12345"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should show E001 error code
    assert!(
        stderr.contains("E001:"),
        "Expected E001, stderr was: {}",
        stderr
    );
    assert!(stderr.contains("does not exist"), "stderr was: {}", stderr);
}

#[test]
fn test_file_instead_of_directory() {
    let output = run_recchroot(&["/etc/passwd"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should show E002 error code
    assert!(
        stderr.contains("E002:"),
        "Expected E002, stderr was: {}",
        stderr
    );
    assert!(stderr.contains("not a directory"), "stderr was: {}", stderr);
}

#[test]
fn test_empty_path() {
    let output = run_recchroot(&[""]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E001:"),
        "Expected E001 for empty path, stderr was: {}",
        stderr
    );
}

#[test]
fn test_not_root() {
    if install_tools::guard::is_root() {
        // As root, recchroot would really bind-mount into the directory.
        eprintln!("skipped: running as root");
        return;
    }
    let dir = tempfile::tempdir().expect("Failed to create temp dir");
    let output = run_recchroot(&[dir.path().to_str().unwrap()]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E008:"),
        "Expected E008 (not root), stderr was: {}",
        stderr
    );
    assert!(
        stderr.contains("must run as root"),
        "Expected 'must run as root' message, stderr was: {}",
        stderr
    );
    assert_eq!(
        output.status.code(),
        Some(8),
        "Expected exit code 8 for E008"
    );
}

#[test]
fn test_protected_path_root() {
    // Test that / is protected - E010 should come before E008 now
    let output = run_recchroot(&["/"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E010:"),
        "Expected E010 (protected path), stderr was: {}",
        stderr
    );
    assert_eq!(
        output.status.code(),
        Some(10),
        "E010 should exit with code 10"
    );
}
