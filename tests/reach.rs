//! Reach audit tests against real files and processes. No mocks.
use porthole::reach::{parent_chain, probe, Reach};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("porthole-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn is_root() -> bool {
    let out = Command::new("id").arg("-u").output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim() == "0"
}

#[test]
fn readable_directory_is_readable() {
    assert_eq!(probe(&scratch("readable")), Reach::Readable);
}

#[test]
fn missing_path_is_absent() {
    assert_eq!(probe(&scratch("missing").join("nope")), Reach::Absent);
}

#[test]
fn mode_000_directory_is_denied_by_permissions() {
    if is_root() {
        eprintln!("skipped: root bypasses permission bits");
        return;
    }
    let dir = scratch("locked").join("locked");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).unwrap();
    let got = probe(&dir);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(got, Reach::DeniedPermissions);
}

#[test]
fn probing_a_file_does_not_change_it() {
    let file = scratch("untouched").join("secret");
    fs::write(&file, b"do not touch").unwrap();
    let before = fs::metadata(&file).unwrap().modified().unwrap();
    assert_eq!(probe(&file), Reach::Readable);
    assert_eq!(fs::read(&file).unwrap(), b"do not touch");
    assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), before);
}

#[test]
fn parent_chain_starts_with_our_parent() {
    let chain = parent_chain(std::process::id());
    assert!(!chain.is_empty(), "no parent chain");
    assert_eq!(chain[0].0, std::os::unix::process::parent_id());
}

fn run(home: &PathBuf, strict: bool) -> (i32, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_porthole"));
    cmd.args(["reach", "--json", "--home"]).arg(home);
    if strict {
        cmd.arg("--strict");
    }
    let out = cmd.output().unwrap();
    (out.status.code().unwrap(), String::from_utf8(out.stdout).unwrap())
}

#[test]
fn strict_passes_when_nothing_sensitive_is_reachable() {
    let (code, json) = run(&scratch("empty-home"), true);
    assert_eq!(code, 0, "{json}");
    assert!(json.contains("\"reach\":\"absent\""), "{json}");
}

#[test]
fn strict_fails_when_ssh_keys_are_readable() {
    let home = scratch("ssh-home");
    fs::create_dir(home.join(".ssh")).unwrap();
    let (code, json) = run(&home, true);
    assert_eq!(code, 1, "{json}");
    assert!(json.contains("\"path\":\"~/.ssh\",\"reach\":\"readable\""), "{json}");
    let (code, _) = run(&home, false);
    assert_eq!(code, 0, "without --strict the report never fails");
}
