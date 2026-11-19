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
fn pause_stops_the_agent_process() {
    let dir = scratch("pause");
    let mut agent = Command::new("sleep").arg("60").spawn().unwrap();
    let pid = agent.id().to_string();
    let path = transcript(&dir, &repeated(4));
    let captures = dir.join("captures");
    let (code, out) = once(&[path.to_str().unwrap(), "--pid", &pid, "--pause", "--out", captures.to_str().unwrap()], &dir);
    let capture = fs::read_dir(&captures).unwrap().next().unwrap().unwrap().path();
    let tree = fs::read_to_string(capture.join("processes.txt")).unwrap();
    assert!(tree.lines().any(|l| l.starts_with(&format!("{pid}\t")) && l.ends_with("sleep")), "{tree}");
    let stat = process_state(&pid);
    let _ = Command::new("kill").args(["-CONT", &pid]).status();
    agent.kill().unwrap();
    agent.wait().unwrap();
    assert_eq!(code, 1, "{out}");
    assert!(stat.trim_start().starts_with('T'), "agent not stopped: {stat:?}");
    assert!(out.contains("kill -CONT"), "no resume instruction: {out}");
}
