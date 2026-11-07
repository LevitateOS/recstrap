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

/// `--write` appends to the target's fstab and keeps what was there; a second
/// `--write` is refused (E036) rather than duplicating the entries, and
/// `--replace` swaps them, comment lines included. Mounts a loop filesystem.
#[test]
#[ignore = "needs root: mounts a loop filesystem"]
fn write_appends_refuses_duplicates_and_replaces() {
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
    run("mkfs.ext4", &["-q", "-L", "RECFSTABWRITE", image]);
    run("mount", &["-o", "loop", image, target]);
    let fstab = format!("{target}/etc/fstab");
    std::fs::create_dir(format!("{target}/etc")).unwrap();
    std::fs::write(&fstab, "# shipped by the rootfs\ntmpfs /tmp tmpfs defaults 0 0\n").unwrap();

    let first = run_recfstab(&["-L", "--write", "--json", target]);
    let second = run_recfstab(&["-L", "--write", "--json", target]);
    let after_second = std::fs::read_to_string(&fstab).unwrap();
    let replaced = run_recfstab(&["-L", "--write", "--replace", "--json", target]);
    let after_replace = std::fs::read_to_string(&fstab).unwrap();
    run("umount", &[target]);

    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));
    let report: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["written"], true);
    assert_eq!(report["existing"][0]["file"], "/tmp");
    assert_eq!(report["existing"][0]["conflicts"], false);

    assert_eq!(second.status.code(), Some(36));
    let error: serde_json::Value = serde_json::from_slice(&second.stderr).unwrap();
    assert_eq!(error["code"], "E036");
    let expected = format!(
        "# shipped by the rootfs\ntmpfs /tmp tmpfs defaults 0 0\n# {}\nLABEL=RECFSTABWRITE\t/\text4\trelatime\t0\t1\n",
        report["entries"][0]["origin"].as_str().unwrap()
    );
    assert_eq!(after_second, expected, "a refused write leaves the fstab alone");

    assert!(replaced.status.success(), "{}", String::from_utf8_lossy(&replaced.stderr));
    let report: serde_json::Value = serde_json::from_slice(&replaced.stdout).unwrap();
    assert_eq!(report["existing"][1]["conflicts"], true);
    assert_eq!(after_replace, expected, "replacing gives the same fstab, not a second copy");
}
