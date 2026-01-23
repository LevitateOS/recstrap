//! Integration tests for recstrap binary.
//!
//! These tests run the actual binary and verify behavior.
//!
//! Note: Most error path tests require root to get past the root check.
//! Tests that don't require root are run normally.
//! Tests that require specific error codes run as root in CI or are skipped.

use std::process::Command;

/// Helper to run recstrap with given args
fn run_recstrap(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_recstrap"))
        .args(args)
        .output()
        .expect("Failed to execute recstrap")
}

/// Check if running as root
fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

// =============================================================================
// CLI Argument Tests (no root required)
// =============================================================================

#[test]
fn test_help_flag() {
    let output = run_recstrap(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("squashfs"), "Help should mention squashfs");
    assert!(
        stdout.contains("--squashfs"),
        "Help should show squashfs flag"
    );
    assert!(
        stdout.contains("<TARGET>") || stdout.contains("TARGET"),
        "Help should show TARGET argument"
    );
    assert!(stdout.contains("--force"), "Help should show force flag");
    assert!(stdout.contains("--quiet"), "Help should show quiet flag");
    assert!(stdout.contains("--check"), "Help should show check flag");
}

#[test]
fn test_version_flag() {
    let output = run_recstrap(&["--version"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("recstrap"));
}

#[test]
fn test_missing_target_argument() {
    let output = run_recstrap(&[]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // clap should complain about missing required argument
    assert!(
        stderr.contains("required") || stderr.contains("<TARGET>"),
        "stderr was: {}",
        stderr
    );
}

// =============================================================================
// Root Check Tests
// =============================================================================

#[test]
fn test_root_check_without_root() {
    if is_root() {
        return;
    }
    let output = run_recstrap(&["/tmp"]);
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

// =============================================================================
// Error Path Tests (require root to get past root check)
// =============================================================================

#[test]
fn test_nonexistent_directory() {
    if !is_root() {
        let output = run_recstrap(&["/nonexistent/path/12345"]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("E008:"),
            "Expected E008 when not root, stderr was: {}",
            stderr
        );
        return;
    }
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
    if !is_root() {
        return;
    }
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

#[test]
fn test_protected_path_root() {
    if !is_root() {
        return;
    }
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

#[test]
fn test_protected_path_usr() {
    if !is_root() {
        return;
    }
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

#[test]
fn test_protected_path_etc() {
    if !is_root() {
        return;
    }
    let output = run_recstrap(&["/etc"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E010:"),
        "Expected E010 for /etc, stderr was: {}",
        stderr
    );
}

#[test]
fn test_squashfs_flag_parses() {
    if !is_root() {
        return;
    }
    let output = run_recstrap(&["--squashfs", "/nonexistent.squashfs", "/nonexistent"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E001:"),
        "--squashfs flag should be recognized, got: {}",
        stderr
    );
}

#[test]
fn test_squashfs_not_found() {
    if !is_root() {
        return;
    }
    let temp_dir = std::env::temp_dir().join("recstrap_test_squashfs_notfound");
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);

    let output = run_recstrap(&[
        "--force", // Skip mount point check
        "--squashfs",
        "/nonexistent/path/test.squashfs",
        temp_dir.to_str().unwrap(),
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E004:"),
        "Expected E004, stderr was: {}",
        stderr
    );
    assert_eq!(
        output.status.code(),
        Some(4),
        "Exit code should be 4 for E004"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_squashfs_is_directory() {
    if !is_root() {
        return;
    }
    let temp_dir = std::env::temp_dir().join("recstrap_test_squashfs_dir");
    let fake_squashfs = std::env::temp_dir().join("recstrap_test_fake_squashfs");
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::remove_dir_all(&fake_squashfs);
    let _ = std::fs::create_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&fake_squashfs);

    let output = run_recstrap(&[
        "--force",
        "--squashfs",
        fake_squashfs.to_str().unwrap(),
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
    let _ = std::fs::remove_dir_all(&fake_squashfs);
}

#[test]
fn test_target_not_empty() {
    if !is_root() {
        return;
    }
    let temp_dir = std::env::temp_dir().join("recstrap_test_notempty");
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);
    let _ = std::fs::write(temp_dir.join("test_file"), b"test");

    let output = run_recstrap(&[temp_dir.to_str().unwrap()]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Could get E011 (not mount point) or E009 (not empty) depending on order
    // Current order: mount point first, then empty
    assert!(
        stderr.contains("E009:") || stderr.contains("E011:"),
        "Expected E009 or E011, stderr was: {}",
        stderr
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_force_flag_allows_nonempty() {
    if !is_root() {
        return;
    }
    let temp_dir = std::env::temp_dir().join("recstrap_test_force");
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);
    let _ = std::fs::write(temp_dir.join("test_file"), b"test");

    let output = run_recstrap(&["--force", temp_dir.to_str().unwrap()]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("E009:") && !stderr.contains("E011:"),
        "--force should skip empty and mount point checks, stderr was: {}",
        stderr
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// =============================================================================
// Exit Code Tests
// =============================================================================

#[test]
fn test_exit_code_success_on_help() {
    let output = run_recstrap(&["--help"]);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn test_exit_code_failure_on_error() {
    let output = run_recstrap(&["/nonexistent"]);
    assert_ne!(output.status.code(), Some(0));
}

#[test]
fn test_exit_code_is_error_specific() {
    if !is_root() {
        let output = run_recstrap(&["/tmp"]);
        assert_eq!(
            output.status.code(),
            Some(8),
            "Exit code should match error code"
        );
    }
}

// =============================================================================
// Protected Path Tests
// =============================================================================

#[test]
fn test_protected_path_proc() {
    if !is_root() {
        return;
    }
    if std::path::Path::new("/proc").exists() {
        let output = run_recstrap(&["/proc"]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        // /proc is a protected path
        assert!(
            stderr.contains("E010:"),
            "Expected E010 for /proc, stderr was: {}",
            stderr
        );
        assert_eq!(
            output.status.code(),
            Some(10),
            "Exit code should be 10 for E010"
        );
    }
}

#[test]
fn test_protected_path_home() {
    if !is_root() {
        return;
    }
    if std::path::Path::new("/home").exists() {
        let output = run_recstrap(&["/home"]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("E010:"),
            "Expected E010 for /home, stderr was: {}",
            stderr
        );
    }
}

#[test]
fn test_mnt_is_allowed() {
    if !is_root() {
        return;
    }
    // /mnt should NOT be protected
    if std::path::Path::new("/mnt").exists() {
        let output = run_recstrap(&["/mnt"]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Should NOT get E010
        assert!(
            !stderr.contains("E010:"),
            "/mnt should not be protected, stderr was: {}",
            stderr
        );
    }
}
