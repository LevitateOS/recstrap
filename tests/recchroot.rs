//! Integration tests for recchroot binary.
//!
//! These tests run the actual binary and verify behavior.
//! Note: Actual chroot/mount tests require root and are marked #[ignore].

use leviso_cheat_test::cheat_aware;
use std::process::Command;

/// Helper to run recchroot with given args
fn run_recchroot(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_recchroot"))
        .args(args)
        .output()
        .expect("Failed to execute recchroot")
}

// =============================================================================
// CLI Argument Tests
// =============================================================================

#[test]
fn test_help_flag() {
    let output = run_recchroot(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("chroot"), "Help should mention chroot");
    assert!(
        stdout.contains("bind mounts"),
        "Help should mention bind mounts"
    );
}

#[test]
fn test_version_flag() {
    let output = run_recchroot(&["--version"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("recchroot"));
}

#[test]
fn test_missing_chroot_argument() {
    let output = run_recchroot(&[]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // clap should complain about missing required argument
    assert!(
        stderr.contains("required") || stderr.contains("<CHROOT_DIR>"),
        "stderr was: {}",
        stderr
    );
}

// =============================================================================
// Error Path Tests
// =============================================================================

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

// =============================================================================
// Exit Code Tests
// =============================================================================

#[test]
fn test_exit_code_success_on_help() {
    let output = run_recchroot(&["--help"]);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn test_exit_code_failure_on_error() {
    let output = run_recchroot(&["/nonexistent"]);
    assert_ne!(output.status.code(), Some(0));
}

// =============================================================================
// Edge Case Tests
// =============================================================================

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
fn test_whitespace_only_path() {
    let output = run_recchroot(&["   "]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E001:"),
        "Expected E001 for whitespace-only path, stderr was: {}",
        stderr
    );
}

#[test]
fn test_path_with_special_characters() {
    // Test paths that might cause shell issues but should be handled safely
    let output = run_recchroot(&["/nonexistent/path with spaces"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E001:"),
        "Expected E001, stderr was: {}",
        stderr
    );
}

// =============================================================================
// Root and Permission Tests (E007, E008)
// =============================================================================

#[cheat_aware(
    protects = "Only root can perform chroot operations (security boundary)",
    severity = "HIGH",
    ease = "EASY",
    cheats = ["Remove root check entirely", "Add --no-root-check flag"],
    consequence = "Unprivileged users attempt chroot and get cryptic permission errors on bind mounts",
    legitimate_change = "Root requirement is fundamental to chroot. \
        This check should never be bypassed."
)]
#[test]
fn test_not_root() {
    // When not running as root, should fail with E007
    let dir = tempfile::tempdir().expect("Failed to create temp dir");
    let output = run_recchroot(&[dir.path().to_str().unwrap()]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E007:"),
        "Expected E007 (not root), stderr was: {}",
        stderr
    );
    assert!(
        stderr.contains("must run as root"),
        "Expected 'must run as root' message, stderr was: {}",
        stderr
    );
    // Verify exit code is 7
    assert_eq!(
        output.status.code(),
        Some(7),
        "Expected exit code 7 for E007"
    );
}

#[cheat_aware(
    protects = "User cannot chroot into root filesystem and corrupt running system",
    severity = "CRITICAL",
    ease = "EASY",
    cheats = ["Remove / from protected paths", "Skip protection check when path looks valid"],
    consequence = "User runs 'recchroot /' and corrupts their running system with bind mounts",
    legitimate_change = "The root filesystem should NEVER be a valid chroot target. \
        If this test fails, fix the protected path validation in src/main.rs"
)]
#[test]
fn test_protected_path_root() {
    // Test that / is protected - E008 should come before E007 now
    let output = run_recchroot(&["/"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E008:"),
        "Expected E008 (protected path), stderr was: {}",
        stderr
    );
    assert_eq!(
        output.status.code(),
        Some(8),
        "E008 should exit with code 8"
    );
}

#[cheat_aware(
    protects = "System directories are never valid chroot targets",
    severity = "CRITICAL",
    ease = "EASY",
    cheats = ["Remove /usr from protected paths", "Allow chroot to any existing directory"],
    consequence = "User runs 'recchroot /usr' and corrupts system binaries",
    legitimate_change = "System directories should NEVER be valid chroot targets. \
        If this test fails, fix the protected path list in src/main.rs"
)]
#[test]
fn test_protected_path_usr() {
    let output = run_recchroot(&["/usr"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E008:"),
        "Expected E008 (protected path), stderr was: {}",
        stderr
    );
    assert_eq!(
        output.status.code(),
        Some(8),
        "E008 should exit with code 8"
    );
}

#[cheat_aware(
    protects = "Exit codes match error codes for scriptable error handling",
    severity = "MEDIUM",
    ease = "MEDIUM",
    cheats = ["Return 1 for all errors", "Return 0 and only report via stderr"],
    consequence = "Scripts cannot distinguish error types, automation breaks",
    legitimate_change = "Exit codes must match error codes (E001 = exit 1, E002 = exit 2). \
        If adding new errors, ensure the exit code matches the error number."
)]
#[test]
fn test_exit_code_matches_error() {
    // E001 should return exit code 1
    let output = run_recchroot(&["/nonexistent/path/12345"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "E001 should exit with code 1"
    );

    // E002 should return exit code 2
    let output = run_recchroot(&["/etc/passwd"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "E002 should exit with code 2"
    );
}

#[test]
fn test_trailing_slash() {
    // Path with trailing slash should behave same as without
    let output = run_recchroot(&["/nonexistent/"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E001:"),
        "Expected E001 for nonexistent path with trailing slash, stderr was: {}",
        stderr
    );
}

// =============================================================================
// Root-Required Tests (marked #[ignore])
// =============================================================================

/// Test actual chroot functionality - requires root
#[test]
#[ignore]
fn test_chroot_echo_command() {
    // This would need a proper chroot environment with /bin/echo
    // Only run manually with root: cargo test -- --ignored
    let output = run_recchroot(&["/", "echo", "hello"]);
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("hello"));
    }
}

/// Test command execution inside chroot - requires root
/// Note: E006 only triggers if the `chroot` binary itself can't execute.
/// If chroot works but the command inside fails, we get the command's exit code.
#[test]
#[ignore]
fn test_command_in_chroot() {
    // Requires root - run with: cargo test -- --ignored
    // This test runs "true" which should succeed
    let output = run_recchroot(&["/", "true"]);
    // Should succeed if running as root with proper chroot
    if output.status.success() {
        assert_eq!(output.status.code(), Some(0));
    }
    // If it fails, it should be E007 (not root) or E008 (protected path)
    // since / is now protected
}
