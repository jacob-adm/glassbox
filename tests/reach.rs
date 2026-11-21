//! Reach audit tests against real files and processes. No mocks.
use glassbox::reach::{parent_chain, probe, Reach};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glassbox-test-{}-{name}", std::process::id()));
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
fn directory_whose_files_are_all_refused_is_listable_only() {
    if is_root() {
        eprintln!("skipped: root bypasses permission bits");
        return;
    }
    let dir = scratch("listable");
    for name in ["a", "b"] {
        fs::write(dir.join(name), "x").unwrap();
        fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o000)).unwrap();
    }
    assert_eq!(probe(&dir), Reach::Listable);
    assert_eq!(probe(&dir).as_str(), "listable-only");
}

#[test]
fn directory_with_files_that_open_and_files_refused_is_partly_readable() {
    if is_root() {
        eprintln!("skipped: root bypasses permission bits");
        return;
    }
    let dir = scratch("mixed");
    fs::write(dir.join("locked"), "x").unwrap();
    fs::set_permissions(dir.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
    fs::write(dir.join("open"), "x").unwrap();
    assert_eq!(probe(&dir), Reach::Partial);
    assert_eq!(probe(&dir).as_str(), "partly-readable");
    fs::set_permissions(dir.join("locked"), fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(probe(&dir), Reach::Readable);
}

#[test]
fn directory_holding_only_subdirectories_is_readable() {
    let dir = scratch("subdirs");
    fs::create_dir(dir.join("inner")).unwrap();
    assert_eq!(probe(&dir), Reach::Readable);
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
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_glassbox"));
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

fn reach_with(home: &PathBuf, extra: &[&str]) -> (i32, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_glassbox"));
    cmd.arg("reach").arg("--home").arg(home).args(extra);
    let out = cmd.output().unwrap();
    (out.status.code().unwrap(), String::from_utf8(out.stdout).unwrap(), String::from_utf8(out.stderr).unwrap())
}

#[test]
fn since_reports_what_closed_against_an_earlier_report() {
    if is_root() {
        eprintln!("skipped: root bypasses permission bits");
        return;
    }
    let home = scratch("since-home");
    fs::create_dir(home.join(".ssh")).unwrap();
    fs::write(home.join(".zsh_history"), "x").unwrap();
    let saved = scratch("since-report").join("before.json");
    fs::write(&saved, reach_with(&home, &["--json"]).1).unwrap();
    let saved = saved.to_str().unwrap();

    let (_, text, _) = reach_with(&home, &["--since", saved]);
    assert!(text.contains("Against the earlier report: 0 closed, 2 still readable, 0 newly readable."), "{text}");
    assert!(!text.contains("(was "), "{text}");

    fs::set_permissions(home.join(".ssh"), fs::Permissions::from_mode(0o000)).unwrap();
    let (code, text, _) = reach_with(&home, &["--since", saved]);
    let (_, json, _) = reach_with(&home, &["--since", saved, "--json"]);
    fs::set_permissions(home.join(".ssh"), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(code, 0);
    assert!(text.contains("~/.ssh  (was readable)"), "{text}");
    assert!(text.contains("Against the earlier report: 1 closed, 1 still readable, 0 newly readable."), "{text}");
    assert!(json.contains("\"path\":\"~/.ssh\",\"reach\":\"denied-permissions\",\"was\":\"readable\""), "{json}");
}

#[test]
fn since_refuses_a_file_that_is_not_a_reach_report() {
    let home = scratch("since-bad-home");
    let dir = scratch("since-bad");
    for (name, text) in [("empty.json", ""), ("other.json", "{\"x\":1}"), ("shape.json", "{\"findings\":[{\"path\":1}]}")] {
        let file = dir.join(name);
        fs::write(&file, text).unwrap();
        let (code, out, err) = reach_with(&home, &["--since", file.to_str().unwrap()]);
        assert_eq!((code, out.as_str()), (2, ""), "{name}");
        assert!(err.contains("is not a `glassbox reach --json` report"), "{err}");
    }
    let (code, _, err) = reach_with(&home, &["--since", dir.join("missing.json").to_str().unwrap()]);
    assert_eq!(code, 2, "{err}");
}

#[test]
fn text_report_names_the_process_chain() {
    let (_, text, _) = reach_with(&scratch("chain-home"), &[]);
    assert!(text.starts_with("Process chain (nearest first):\n"), "{text}");
}
