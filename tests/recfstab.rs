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
    // Nothing is mounted under a fresh directory, and the host's own swap is
    // not the target's, so there is nothing to write.
    let temp_dir = tempfile::tempdir().expect("create isolated scan directory");
    let output = run_recfstab(&[temp_dir.path().to_str().unwrap()]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(24), "{stderr}");
    assert!(output.stdout.is_empty(), "failed scan emitted partial fstab");
    assert!(stderr.contains("E024:"), "expected explicit empty-scan error, got: {stderr}");
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

/// The target's own filesystem by stable identifier, its swap file re-rooted,
/// and nothing of the host's. Mounts a loop filesystem, so it needs root.
#[test]
#[ignore = "needs root: mounts a loop filesystem and enables a swap file"]
fn fstab_describes_a_loop_target_with_a_swap_file() {
    use std::process::Command as Cmd;
    let run = |program: &str, args: &[&str]| {
        let status = Cmd::new(program).args(args).status().expect(program);
        assert!(status.success(), "{program} {args:?} failed");
    };
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("disk.img");
    let target = dir.path().join("t");
    std::fs::create_dir(&target).unwrap();
    let (image, target) = (image.to_str().unwrap(), target.to_str().unwrap());
    run("truncate", &["-s", "200M", image]);
    run("mkfs.ext4", &["-q", "-L", "RECFSTABTEST", image]);
    run("mount", &["-o", "loop,noatime", image, target]);
    let swapfile = format!("{target}/swapfile");
    run("dd", &["if=/dev/zero", &format!("of={swapfile}"), "bs=1M", "count=16", "status=none"]);
    run("chmod", &["600", &swapfile]);
    run("mkswap", &["-q", &swapfile]);
    run("swapon", &[&swapfile]);

    let output = run_recfstab(&["-L", target]);

    run("swapoff", &[&swapfile]);
    run("umount", &[target]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let records: Vec<Vec<&str>> = stdout
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split('\t').collect())
        .collect();
    assert_eq!(
        records,
        vec![
            vec!["LABEL=RECFSTABTEST", "/", "ext4", "noatime", "0", "1"],
            vec!["/swapfile", "none", "swap", "defaults", "0", "0"],
        ],
        "full output:\n{stdout}"
    );
}
