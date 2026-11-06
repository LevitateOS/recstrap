//! Integration tests for recfstab binary.
//!
//! These tests run the actual binary and verify behavior.

use std::process::Command;

/// Helper to run recfstab with given args
fn run_recfstab(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_recfstab"))
        .args(args)
        .output()
        .expect("Failed to execute recfstab")
}

/// Validate output from the actual CLI, including a nonempty set of usable entries.
fn assert_fstab_output(output: &std::process::Output) -> String {
    assert!(
        std::path::Path::new("/proc/mounts").is_file(),
        "recfstab integration requires Linux mount information at /proc/mounts"
    );
    assert!(
        output.status.success(),
        "recfstab failed: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout.clone()).expect("fstab stdout is UTF-8");
    let entries: Vec<_> = stdout
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    assert!(
        !entries.is_empty(),
        "successful recfstab emitted no filesystem or swap entries"
    );
    let mut mountpoints = std::collections::HashSet::new();
    for line in entries {
        let fields: Vec<_> = line.split_whitespace().collect();
        assert_eq!(fields.len(), 6, "invalid fstab record: {line}");
        assert_eq!(fields[4], "0", "invalid dump field: {line}");
        let pass: u8 = fields[5].parse().expect("numeric fsck pass");
        assert!(pass <= 2, "invalid fsck pass: {line}");
        if fields[2] != "swap" {
            assert!(
                fields[1].starts_with('/'),
                "mountpoint must be absolute: {line}"
            );
            assert!(
                mountpoints.insert(fields[1]),
                "duplicate mountpoint: {line}"
            );
        }
    }
    stdout
}

// =============================================================================
// CLI Argument Tests
// =============================================================================

#[test]
fn test_help_flag() {
    let output = run_recfstab(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Check for key elements in help
    assert!(stdout.contains("fstab"), "Help should mention fstab");
    assert!(
        stdout.contains("--label") || stdout.contains("-L"),
        "Help should show label flag"
    );
    assert!(
        stdout.contains("<ROOT>") || stdout.contains("ROOT"),
        "Help should show ROOT argument"
    );
}

#[test]
fn test_version_flag() {
    let output = run_recfstab(&["--version"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("recfstab"));
}

#[test]
fn test_missing_root_argument() {
    let output = run_recfstab(&[]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // clap should complain about missing required argument
    assert!(
        stderr.contains("required") || stderr.contains("<ROOT>"),
        "stderr was: {}",
        stderr
    );
}

// =============================================================================
// Error Path Tests
// =============================================================================

#[test]
fn test_nonexistent_directory() {
    let output = run_recfstab(&["/nonexistent/path/12345"]);
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
    let output = run_recfstab(&["/etc/passwd"]);
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
fn test_missing_findmnt_fails_with_actionable_error() {
    let empty_path = tempfile::tempdir().expect("create empty command search path");
    let output = Command::new(env!("CARGO_BIN_EXE_recfstab"))
        .arg("/")
        .env("PATH", empty_path.path())
        .output()
        .expect("execute the real recfstab binary without findmnt available");
    assert_eq!(output.status.code(), Some(7));
    assert!(
        output.stdout.is_empty(),
        "failed mount lookup emitted a partial fstab"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E007:") && stderr.contains("findmnt") && stderr.contains("util-linux"),
        "missing dependency must report the cause and remediation: {stderr}"
    );
}

#[test]
fn test_empty_directory_no_mounts() {
    let temp_dir = tempfile::tempdir().expect("create isolated scan directory");
    let output = run_recfstab(&[temp_dir.path().to_str().unwrap()]);
    if output.status.success() {
        // Global swap partitions may be emitted even when this directory has no mounts.
        let stdout = assert_fstab_output(&output);
        for line in stdout
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let fields: Vec<_> = line.split_whitespace().collect();
            assert_eq!(
                fields[1], "none",
                "unexpected mount in empty directory: {line}"
            );
            assert_eq!(
                fields[2], "swap",
                "unexpected filesystem in empty directory: {line}"
            );
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert!(
            output.stdout.is_empty(),
            "failed scan emitted partial fstab"
        );
        assert!(
            stderr.contains("E024:") && stderr.contains("no filesystems found"),
            "expected explicit empty-scan error, got: {stderr}"
        );
    }
}

// =============================================================================
// Exit Code Tests
// =============================================================================

#[test]
fn test_exit_code_success_on_help() {
    let output = run_recfstab(&["--help"]);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn test_exit_code_failure_on_error() {
    let output = run_recfstab(&["/nonexistent"]);
    assert_ne!(output.status.code(), Some(0));
}

// =============================================================================
// Real Linux mount checks (require /proc/mounts and a block-backed root)
// =============================================================================

#[test]
fn test_root_filesystem() {
    let stdout = assert_fstab_output(&run_recfstab(&["/"]));
    let roots: Vec<_> = stdout
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|line| line.split_whitespace().nth(1) == Some("/"))
        .collect();
    assert_eq!(
        roots.len(),
        1,
        "expected one block-backed root mount; inspect findmnt -rn: {stdout}"
    );
}

#[test]
fn test_label_flag_parses() {
    // Test that -L flag is recognized (even if no labels exist)
    let output = run_recfstab(&["-L", "/nonexistent_path_12345"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should fail with E001, NOT with "unknown argument"
    assert!(
        stderr.contains("E001:"),
        "-L flag should be recognized, got: {}",
        stderr
    );
}

#[test]
fn test_long_label_flag() {
    // Test that --label flag works the same as -L
    let output = run_recfstab(&["--label", "/nonexistent_path_12345"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should fail with E001
    assert!(
        stderr.contains("E001:"),
        "--label flag should be recognized, got: {}",
        stderr
    );
}

#[test]
fn test_partuuid_flag_parses() {
    // Test that -p flag is recognized
    let output = run_recfstab(&["-p", "/nonexistent_path_12345"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should fail with E001, NOT with "unknown argument"
    assert!(
        stderr.contains("E001:"),
        "-p flag should be recognized, got: {}",
        stderr
    );
}

#[test]
fn test_long_partuuid_flag() {
    // Test that --partuuid flag works the same as -p
    let output = run_recfstab(&["--partuuid", "/nonexistent_path_12345"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should fail with E001
    assert!(
        stderr.contains("E001:"),
        "--partuuid flag should be recognized, got: {}",
        stderr
    );
}

#[test]
fn test_partlabel_flag_parses() {
    // Test that -t flag is recognized
    let output = run_recfstab(&["-t", "/nonexistent_path_12345"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should fail with E001, NOT with "unknown argument"
    assert!(
        stderr.contains("E001:"),
        "-t flag should be recognized, got: {}",
        stderr
    );
}

#[test]
fn test_long_partlabel_flag() {
    // Test that --partlabel flag works the same as -t
    let output = run_recfstab(&["--partlabel", "/nonexistent_path_12345"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should fail with E001
    assert!(
        stderr.contains("E001:"),
        "--partlabel flag should be recognized, got: {}",
        stderr
    );
}

#[test]
fn test_conflicting_flags() {
    // Test that conflicting flags (-L and -p) produce an error
    let output = run_recfstab(&["-L", "-p", "/"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // clap should complain about conflicts
    assert!(
        stderr.contains("cannot be used with") || stderr.contains("conflict"),
        "Should report conflicting flags, got: {}",
        stderr
    );
}

// =============================================================================
// Output Format Tests
// =============================================================================

#[test]
fn test_output_has_comment_lines() {
    let stdout = assert_fstab_output(&run_recfstab(&["/"]));
    assert!(
        stdout.contains("# /dev/") || stdout.contains("# UUID="),
        "missing device comments in fstab output: {stdout}"
    );
}

#[test]
fn test_output_is_valid_fstab_format() {
    assert_fstab_output(&run_recfstab(&["/"]));
}

#[test]
fn test_pseudo_filesystems_excluded() {
    let stdout = assert_fstab_output(&run_recfstab(&["/"]));
    let forbidden = [
        "proc", "sysfs", "devpts", "tmpfs", "cgroup2", "devtmpfs", "overlay",
    ];
    for line in stdout
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        assert!(
            !forbidden.contains(&fields[2]),
            "pseudo-filesystem leaked into fstab: {line}"
        );
    }
}
