//! `glassbox watch` against real transcript files, real files and real processes. No mocks.
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const SECRET: &str = "sk-test-SECRET-do-not-copy";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glassbox-watch-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// One assistant turn calling `tool` with `input`, in Claude Code's transcript format.
fn call(id: usize, tool: &str, input: &str, output_tokens: u64) -> String {
    format!(
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t{id}","name":"{tool}","input":{input}}}],"usage":{{"input_tokens":10,"output_tokens":{output_tokens}}}}}}}"#
    )
}

fn result(id: usize, error: bool) -> String {
    format!(
        r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t{id}","content":"output {SECRET}","is_error":{error}}}]}}}}"#
    )
}

fn transcript(dir: &Path, lines: &[String]) -> PathBuf {
    let path = dir.join("session.jsonl");
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    path
}

fn repeated(n: usize) -> Vec<String> {
    let input = format!(r#"{{"command":"curl https://example.invalid/{SECRET}"}}"#);
    (0..n).flat_map(|i| [call(i, "Bash", &input, 50), result(i, false)]).collect()
}

fn once(args: &[&str], home: &Path) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_glassbox"))
        .arg("watch").args(args).arg("--once").arg("--home").arg(home).output().unwrap();
    (out.status.code().unwrap(), String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn loop_trigger_fires_at_four_identical_calls_not_three() {
    let dir = scratch("loop");
    let three = transcript(&dir, &repeated(3));
    assert_eq!(once(&[three.to_str().unwrap()], &dir).0, 0);
    let four = transcript(&dir, &repeated(4));
    let (code, out) = once(&[four.to_str().unwrap()], &dir);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("loop") && out.contains("Bash"), "{out}");
    assert!(!out.contains(SECRET), "transcript content leaked: {out}");
}

#[test]
fn token_spike_and_failure_runs_fire() {
    let dir = scratch("spike");
    let spike = transcript(&dir, &[call(0, "Read", r#"{"file":"a"}"#, 20_000)]);
    let (code, out) = once(&[spike.to_str().unwrap()], &dir);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("spike"), "{out}");

    let failing: Vec<String> =
        (0..5).flat_map(|i| [call(i, "Edit", &format!(r#"{{"n":{i}}}"#), 50), result(i, true)]).collect();
    let fails = transcript(&dir, &failing);
    let (code, out) = once(&[fails.to_str().unwrap()], &dir);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("failures"), "{out}");
}

#[test]
fn thresholds_are_configurable() {
    let dir = scratch("thresholds");
    let three = transcript(&dir, &repeated(3));
    assert_eq!(once(&[three.to_str().unwrap(), "--loop", "3"], &dir).0, 1);
    let small = transcript(&dir, &[call(0, "Read", r#"{"file":"a"}"#, 500)]);
    assert_eq!(once(&[small.to_str().unwrap(), "--spike", "400"], &dir).0, 1);
}

#[test]
fn an_unknown_transcript_format_is_an_error_not_silence() {
    let dir = scratch("unknown");
    let path = dir.join("other.jsonl");
    fs::write(&path, "{\"event\":\"x\"}\n{\"event\":\"y\"}\n").unwrap();
    assert_eq!(once(&[path.to_str().unwrap()], &dir).0, 2);
}

fn autostart_entry(home: &Path) -> PathBuf {
    let dir = if cfg!(target_os = "macos") { home.join("Library/LaunchAgents") } else { home.join(".config/systemd/user") };
    fs::create_dir_all(&dir).unwrap();
    dir.join("com.example.planted.plist")
}

/// Start following `path`; returns once glassbox reports it is watching.
fn follow(path: &Path, home: &Path, out: &Path, extra: &[&str]) -> Child {
    let mut child = Command::new(env!("CARGO_BIN_EXE_glassbox"))
        .arg("watch").arg(path).arg("--home").arg(home).arg("--out").arg(out).args(extra)
        .stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap()).read_line(&mut line).unwrap();
    assert!(line.contains("watching"), "unexpected first line: {line}");
    child
}

fn wait_for_capture(out: &Path) -> PathBuf {
    let start = Instant::now();
    loop {
        if let Some(dir) = fs::read_dir(out).ok().and_then(|mut d| d.find_map(|e| e.ok().map(|e| e.path()))) {
            if dir.join("trigger.json").exists() && dir.join("persist.json").exists() {
                return dir;
            }
        }
        assert!(start.elapsed() < Duration::from_secs(15), "no capture written");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_capture_holds_the_trigger_and_what_changed_since_watching_began_but_no_content() {
    let (dir, home, out) = (scratch("capture"), scratch("capture-home"), scratch("capture-out"));
    let path = transcript(&dir, &[call(0, "Read", r#"{"file":"a"}"#, 50)]);
    let mut child = follow(&path, &home, &out, &[]);
    fs::write(autostart_entry(&home), "run at login").unwrap();
    let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all((repeated(4).join("\n") + "\n").as_bytes()).unwrap();
    drop(f);
    let capture = wait_for_capture(&out);
    child.kill().unwrap();
    child.wait().unwrap();
    let trigger = fs::read_to_string(capture.join("trigger.json")).unwrap();
    let persist = fs::read_to_string(capture.join("persist.json")).unwrap();
    assert!(trigger.contains("\"kind\":\"loop\"") && trigger.contains("\"tool\":\"Bash\""), "{trigger}");
    assert!(persist.contains("\"change\":\"added\"") && persist.contains("com.example.planted.plist"), "{persist}");
    for entry in fs::read_dir(&capture).unwrap() {
        let text = fs::read_to_string(entry.unwrap().path()).unwrap_or_default();
        assert!(!text.contains(SECRET), "transcript content in capture: {text}");
    }
}

#[test]
fn history_already_in_the_transcript_does_not_fire_when_following() {
    let (dir, home, out) = (scratch("history"), scratch("history-home"), scratch("history-out"));
    let path = transcript(&dir, &repeated(6));
    let mut child = follow(&path, &home, &out, &[]);
    std::thread::sleep(Duration::from_millis(1500));
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(fs::read_dir(&out).unwrap().count(), 0, "a trigger fired on history");
}

/// The process state letter (T = stopped): /proc on Linux, where minimal systems lack ps.
fn process_state(pid: &str) -> String {
    if cfg!(target_os = "linux") {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        stat[stat.rfind(')').unwrap() + 2..].chars().take(1).collect()
    } else {
        String::from_utf8(Command::new("ps").args(["-o", "stat=", "-p", pid]).output().unwrap().stdout).unwrap()
    }
}

#[test]
fn pause_stops_the_agent_process_and_its_tree_is_captured() {
    let (dir, home, out) = (scratch("pause"), scratch("pause-home"), scratch("pause-out"));
    let mut agent = Command::new("sleep").arg("60").spawn().unwrap();
    let pid = agent.id().to_string();
    let path = transcript(&dir, &[call(0, "Read", r#"{"file":"a"}"#, 50)]);
    let mut child = follow(&path, &home, &out, &["--pid", &pid, "--pause"]);
    let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all((repeated(4).join("\n") + "\n").as_bytes()).unwrap();
    drop(f);
    let capture = wait_for_capture(&out);
    std::thread::sleep(Duration::from_millis(300));
    let stat = process_state(&pid);
    let _ = Command::new("kill").args(["-CONT", &pid]).status();
    agent.kill().unwrap();
    agent.wait().unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    let tree = fs::read_to_string(capture.join("processes.txt")).unwrap();
    assert!(tree.lines().any(|l| l.starts_with(&format!("{pid}\t")) && l.ends_with("sleep")), "{tree}");
    assert!(stat.trim_start().starts_with('T'), "agent not stopped: {stat:?}");
}

#[test]
fn pause_is_refused_with_once() {
    let dir = scratch("pause-once");
    let path = transcript(&dir, &repeated(4));
    assert_eq!(once(&[path.to_str().unwrap(), "--pid", "1", "--pause"], &dir).0, 2);
}

// Review of the first version.

#[test]
fn one_response_split_over_several_lines_spikes_once() {
    let dir = scratch("split");
    let part = |block: &str| {
        format!(r#"{{"type":"assistant","message":{{"id":"msg_1","role":"assistant","content":[{block}],"usage":{{"output_tokens":20000}}}}}}"#)
    };
    let path = transcript(&dir, &[part(r#"{"type":"thinking","thinking":"x"}"#), part(r#"{"type":"text","text":"y"}"#), part(r#"{"type":"text","text":"z"}"#)]);
    let (code, out) = once(&[path.to_str().unwrap()], &dir);
    assert_eq!(code, 1);
    assert_eq!(out.matches("trigger spike").count(), 1, "{out}");
}

#[test]
fn a_planted_capture_directory_is_never_written_through() {
    let (dir, out) = (scratch("plant"), scratch("plant-out"));
    let victim = dir.join("victim");
    fs::write(&victim, "keep me").unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    for s in now..now + 30 {
        let d = out.join(format!("{s}-1-loop"));
        fs::create_dir_all(&d).unwrap();
        for f in ["trigger.json", "persist.json"] {
            std::os::unix::fs::symlink(&victim, d.join(f)).unwrap();
        }
    }
    let path = transcript(&dir, &repeated(4));
    let (code, _) = once(&[path.to_str().unwrap(), "--out", out.to_str().unwrap()], &dir);
    assert_eq!(code, 1);
    assert_eq!(fs::read_to_string(&victim).unwrap(), "keep me");
}

#[test]
fn another_users_process_counts_as_alive() {
    // pid 1 belongs to root: signalling it fails with EPERM for an unprivileged user.
    if String::from_utf8(Command::new("id").arg("-u").output().unwrap().stdout).unwrap().trim() != "0" {
        assert!(glassbox::watch::alive(1));
    }
}

#[test]
fn a_character_split_across_two_reads_is_reassembled() {
    let dir = scratch("utf8");
    let path = dir.join("t.jsonl");
    fs::write(&path, "").unwrap();
    let mut follower = glassbox::watch::Follower::at_end(&path).unwrap();
    let line = "{\"type\":\"user\",\"message\":{\"content\":\"é\"}}\n".as_bytes();
    let cut = line.iter().position(|b| *b == 0xC3).unwrap() + 1;
    let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(&line[..cut]).unwrap();
    assert!(follower.new_lines().unwrap().is_empty());
    f.write_all(&line[cut..]).unwrap();
    assert_eq!(follower.new_lines().unwrap(), vec![String::from_utf8(line[..line.len() - 1].to_vec()).unwrap()]);
}

#[test]
fn a_transcript_replaced_by_rename_is_followed() {
    let dir = scratch("rename");
    let path = dir.join("t.jsonl");
    fs::write(&path, "old\n").unwrap();
    let mut follower = glassbox::watch::Follower::at_end(&path).unwrap();
    let tmp = dir.join("t.tmp");
    fs::write(&tmp, "new\n").unwrap();
    fs::rename(&tmp, &path).unwrap();
    assert_eq!(follower.new_lines().unwrap(), vec!["new".to_string()]);
}

#[test]
fn trigger_line_numbers_are_transcript_line_numbers_when_following() {
    let (dir, home, out) = (scratch("lines"), scratch("lines-home"), scratch("lines-out"));
    let history: Vec<String> = (0..5).map(|i| call(100 + i, "Read", &format!(r#"{{"n":{i}}}"#), 50)).collect();
    let path = transcript(&dir, &history);
    let mut child = follow(&path, &home, &out, &[]);
    let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all((repeated(4).join("\n") + "\n").as_bytes()).unwrap();
    drop(f);
    let capture = wait_for_capture(&out);
    child.kill().unwrap();
    child.wait().unwrap();
    // Lines 6..13 are call/result pairs; the fourth identical call is line 12.
    assert!(fs::read_to_string(capture.join("trigger.json")).unwrap().contains("\"line\":12"));
}

#[test]
fn tool_names_are_escaped_once_in_trigger_json() {
    let (dir, out) = (scratch("toolname"), scratch("toolname-out"));
    let lines: Vec<String> = (0..4).map(|i| call(i, "mcp__x\\\"y", "{}", 50)).collect();
    let path = transcript(&dir, &lines);
    assert_eq!(once(&[path.to_str().unwrap(), "--out", out.to_str().unwrap()], &dir).0, 1);
    let capture = fs::read_dir(&out).unwrap().next().unwrap().unwrap().path();
    let trigger = fs::read_to_string(capture.join("trigger.json")).unwrap();
    assert!(trigger.contains(r#""tool":"mcp__x\"y""#), "{trigger}");
}

#[test]
fn a_first_line_still_being_written_is_not_an_unknown_format() {
    let (dir, home, out) = (scratch("partial"), scratch("partial-home"), scratch("partial-out"));
    let path = dir.join("session.jsonl");
    fs::write(&path, r#"{"type":"assistant","message":{"content":[{"type":"text","text":"half"#).unwrap();
    let mut child = follow(&path, &home, &out, &[]);
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn watch_options_may_be_given_once() {
    let dir = scratch("dupes");
    let path = transcript(&dir, &repeated(1));
    assert_eq!(once(&[path.to_str().unwrap(), "--loop", "3", "--loop", "4"], &dir).0, 2);
}
