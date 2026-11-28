//! `--audit-json` tests against real files in a temporary home. No mocks.
use glassbox::json::{parse, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glassbox-audit-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// (exit code, stdout, stderr) of glassbox with ARGS.
fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_glassbox")).args(args).output().unwrap();
    (out.status.code().unwrap(), String::from_utf8(out.stdout).unwrap(), String::from_utf8(out.stderr).unwrap())
}

fn obj(v: &Value) -> &BTreeMap<String, Value> {
    match v {
        Value::Obj(o) => o,
        other => panic!("not an object: {other:?}"),
    }
}

fn text(v: &Value) -> &str {
    match v {
        Value::Str(s) => s,
        other => panic!("not a string: {other:?}"),
    }
}

fn list(v: &Value) -> &Vec<Value> {
    match v {
        Value::Arr(a) => a,
        other => panic!("not an array: {other:?}"),
    }
}

/// Parse the object and check the format's rules a glassbox result can break.
fn check(code: i32, out: &str) -> Value {
    let doc = parse(out.trim_end()).expect("one JSON object");
    let top = obj(&doc);
    let keys: Vec<&str> = top.keys().map(String::as_str).collect();
    assert_eq!(keys, ["errors", "exit", "findings", "format", "target", "tool"]);
    assert_eq!(text(&top["format"]), "audit-result/1");
    assert_eq!(text(&obj(&top["tool"])["name"]), "glassbox");
    assert_eq!(text(&obj(&top["tool"])["version"]), env!("CARGO_PKG_VERSION"));
    assert_eq!(text(&obj(&top["target"])["kind"]), "host");
    let found = list(&top["findings"]).iter().any(|f| text(&obj(f)["severity"]) != "info");
    let want = if !list(&top["errors"]).is_empty() { 2 } else if found { 1 } else { 0 };
    assert_eq!(top["exit"], Value::Num(want.to_string()));
    assert_eq!(code, want, "{out}");
    for f in list(&top["findings"]) {
        let keys: Vec<&str> = obj(f).keys().map(String::as_str).collect();
        assert_eq!(keys, ["evidence", "message", "record", "rule", "severity"]);
        // reach and persist records carry the path; a watch record names the transcript instead.
        let record = obj(&obj(f)["record"]);
        let shown = record.get("path").or_else(|| record.get("transcript")).expect("a path in the record");
        assert_eq!(&obj(&obj(f)["evidence"])["path"], shown);
    }
    doc
}

/// (rule, severity, message) of the finding whose evidence path is PATH.
fn finding<'a>(doc: &'a Value, path: &str) -> (&'a str, &'a str, &'a str) {
    let f = list(&obj(doc)["findings"])
        .iter()
        .map(obj)
        .find(|f| text(&obj(&f["evidence"])["path"]) == path)
        .unwrap_or_else(|| panic!("no finding for {path}"));
    (text(&f["rule"]), text(&f["severity"]), text(&f["message"]))
}

#[test]
fn reach_readable_credentials_are_high_and_exit_like_strict() {
    let home = scratch("reach-ssh");
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(home.join(".ssh/id_ed25519"), "x").unwrap();
    let h = home.to_str().unwrap();
    let (code, out, err) = run(&["reach", "--audit-json", "--home", h]);
    let doc = check(code, &out);
    assert_eq!(err, "");
    assert_eq!(finding(&doc, "~/.ssh"), ("glassbox.reach.readable", "high", "Sensitive location ~/.ssh (credentials) is readable"));
    assert_eq!(finding(&doc, "~/.aws/credentials").1, "info");
    assert_eq!(text(&obj(&obj(&doc)["target"])["name"]), h);
    // The exit status is the one --strict gives, and the records are the --json findings.
    let (strict, _, _) = run(&["reach", "--strict", "--home", h]);
    assert_eq!(code, strict);
    let (_, json, _) = run(&["reach", "--json", "--home", h]);
    let json = parse(json.trim_end()).unwrap();
    let records: Vec<&Value> = list(&obj(&doc)["findings"]).iter().map(|f| &obj(f)["record"]).collect();
    let expected: Vec<&Value> = list(&obj(&json)["findings"]).iter().collect();
    assert_eq!(records, expected);
}

#[test]
fn reach_empty_home_has_only_info_and_exits_0() {
    let home = scratch("reach-empty");
    let (code, out, _) = run(&["reach", "--audit-json", "--home", home.to_str().unwrap()]);
    let doc = check(code, &out);
    assert_eq!(code, 0);
    assert_eq!(finding(&doc, "~/.ssh"), ("glassbox.reach.absent", "info", "Sensitive location ~/.ssh (credentials) is not present"));
}

#[test]
fn reach_since_names_what_it_was() {
    let home = scratch("reach-since");
    let h = home.to_str().unwrap();
    let (_, before, _) = run(&["reach", "--json", "--home", h]);
    let report = home.with_extension("before.json");
    fs::write(&report, before).unwrap();
    fs::create_dir_all(home.join(".ssh")).unwrap();
    let (code, out, _) = run(&["reach", "--audit-json", "--since", report.to_str().unwrap(), "--home", h]);
    let doc = check(code, &out);
    assert_eq!(finding(&doc, "~/.ssh").2, "Sensitive location ~/.ssh (credentials) is readable; it was absent");
}

#[test]
fn reach_since_a_file_that_is_not_a_report_is_an_error_object() {
    let home = scratch("reach-bad-since");
    let bad = home.with_extension("bad.json");
    fs::write(&bad, "[]").unwrap();
    let (code, out, err) = run(&["reach", "--audit-json", "--since", bad.to_str().unwrap(), "--home", home.to_str().unwrap()]);
    let doc = check(code, &out);
    let errors = list(&obj(&doc)["errors"]);
    assert!(text(&obj(&errors[0])["message"]).ends_with("is not a `glassbox reach --json` report"));
    assert!(err.contains("is not a `glassbox reach --json` report"));
}

/// A per-user location that runs a program at login on this platform.
fn autostart(home: &Path) -> (PathBuf, &'static str) {
    if cfg!(target_os = "macos") {
        (home.join("Library/LaunchAgents/com.example.agent.plist"), "~/Library/LaunchAgents/com.example.agent.plist")
    } else {
        (home.join(".config/systemd/user/example.service"), "~/.config/systemd/user/example.service")
    }
}

#[test]
fn persist_added_changed_removed() {
    let home = scratch("persist");
    let h = home.to_str().unwrap();
    fs::write(home.join(".zshrc"), "a\n").unwrap();
    fs::write(home.join(".profile"), "a\n").unwrap();
    let snap = home.with_extension("snapshot");
    assert_eq!(run(&["persist", "--save", snap.to_str().unwrap(), "--home", h]).0, 0);
    let (agent, shown) = autostart(&home);
    fs::create_dir_all(agent.parent().unwrap()).unwrap();
    fs::write(&agent, "x").unwrap();
    fs::write(home.join(".zshrc"), "b\n").unwrap();
    fs::remove_file(home.join(".profile")).unwrap();
    let (code, out, _) = run(&["persist", "--since", snap.to_str().unwrap(), "--audit-json", "--home", h]);
    let doc = check(code, &out);
    assert_eq!(code, 1);
    let (rule, severity, message) = finding(&doc, shown);
    assert_eq!((rule, severity), ("glassbox.persist.added", "high"));
    assert_eq!(message, format!("Persistence point {shown} (autostart) was added since the snapshot"));
    assert_eq!(finding(&doc, "~/.zshrc"), ("glassbox.persist.changed", "high", "Persistence point ~/.zshrc (shell startup) changed since the snapshot"));
    assert_eq!(finding(&doc, "~/.profile"), ("glassbox.persist.removed", "info", "Persistence point ~/.profile (shell startup) was removed since the snapshot"));
}

#[test]
fn persist_unreadable_snapshot_is_an_error_object() {
    let home = scratch("persist-bad");
    let missing = home.with_extension("missing");
    let (code, out, _) = run(&["persist", "--since", missing.to_str().unwrap(), "--audit-json", "--home", home.to_str().unwrap()]);
    let doc = check(code, &out);
    assert!(text(&obj(&list(&obj(&doc)["errors"])[0])["message"]).starts_with("cannot read snapshot"));
}

#[test]
fn usage_errors_are_error_objects() {
    let home = scratch("usage");
    let h = home.to_str().unwrap();
    for (args, msg) in [
        (vec!["reach", "--json", "--audit-json", "--home", h], "--json and --audit-json cannot be used together"),
        (vec!["persist", "--audit-json", "--home", h], "--audit-json needs --since"),
        (vec!["reach", "--audit-json", "--bogus"], "unknown argument: --bogus"),
    ] {
        let (code, out, err) = run(&args);
        let doc = check(code, &out);
        assert_eq!(text(&obj(&list(&obj(&doc)["errors"])[0])["message"]), msg);
        assert!(err.starts_with(msg), "{err}");
    }
    // Without the flag, a usage error prints nothing on stdout, as before.
    assert_eq!(run(&["reach", "--bogus"]).1, "");
}

#[test]
fn unset_home_is_an_error_object() {
    let out = Command::new(env!("CARGO_BIN_EXE_glassbox")).args(["reach", "--audit-json"]).env_remove("HOME").output().unwrap();
    let doc = check(out.status.code().unwrap(), &String::from_utf8(out.stdout).unwrap());
    assert_eq!(text(&obj(&list(&obj(&doc)["errors"])[0])["message"]), "HOME is not set; pass --home");
}

#[test]
fn documented_example_is_real_output() {
    // The example in docs/commands.md: a home with ~/.ssh, as `reach --audit-json --home /tmp/demo`.
    let home = scratch("docs");
    fs::create_dir_all(home.join(".ssh")).unwrap();
    let (_, out, _) = run(&["reach", "--audit-json", "--home", home.to_str().unwrap()]);
    let got = out.replace(home.to_str().unwrap(), "/tmp/demo");
    let docs = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/commands.md")).unwrap();
    let section = docs.split("### Audit result output").nth(1).expect("section in docs/commands.md");
    let block = section.split("```json\n").nth(1).unwrap().split("```").next().unwrap();
    // The page shows one finding of the list; it must be in the real output exactly.
    let shown = parse(block.trim_end()).expect("the example is JSON");
    let got = parse(got.trim_end()).unwrap();
    assert!(list(&obj(&got)["findings"]).contains(&shown), "{block}");
}

/// One assistant turn calling `tool`, in Claude Code's transcript format.
fn call(id: usize, tool: &str, input: &str) -> String {
    format!(
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t{id}","name":"{tool}","input":{input}}}],"usage":{{"input_tokens":10,"output_tokens":50}}}}}}"#
    )
}

fn tool_result(id: usize) -> String {
    format!(
        r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t{id}","content":"private output","is_error":false}}]}}}}"#
    )
}

/// N identical Bash calls with their results; the input holds a word that must never be printed.
fn transcript(dir: &Path, n: usize) -> PathBuf {
    let lines: Vec<String> =
        (0..n).flat_map(|i| [call(i, "Bash", r#"{"command":"echo hidden-input"}"#), tool_result(i)]).collect();
    let path = dir.join(format!("session-{n}.jsonl"));
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    path
}

#[test]
fn watch_once_reports_a_loop_with_its_line() {
    let home = scratch("watch-loop");
    let t = transcript(&home, 4);
    let (code, out, _) = run(&["watch", t.to_str().unwrap(), "--once", "--audit-json", "--home", home.to_str().unwrap()]);
    let doc = check(code, &out);
    assert_eq!(code, 1);
    let found = list(&obj(&doc)["findings"]);
    assert_eq!(found.len(), 1, "{out}");
    let f = obj(&found[0]);
    assert_eq!((text(&f["rule"]), text(&f["severity"])), ("glassbox.watch.loop", "medium"));
    assert_eq!(text(&f["message"]), "Agent transcript loop: 4 identical Bash calls in a row (line 7)");
    assert_eq!(obj(&f["evidence"])["line"], Value::Num("7".into()));
    assert_eq!(text(&obj(&f["evidence"])["path"]), t.to_str().unwrap());
    assert!(!out.contains("hidden-input") && !out.contains("private output"), "content leaked: {out}");
}

#[test]
fn watch_once_with_nothing_fired_exits_0() {
    let home = scratch("watch-clean");
    let t = transcript(&home, 3);
    let (code, out, _) = run(&["watch", t.to_str().unwrap(), "--once", "--audit-json", "--home", home.to_str().unwrap()]);
    check(code, &out);
    assert_eq!(code, 0);
}

#[test]
fn watch_audit_json_needs_once_and_a_known_transcript() {
    let home = scratch("watch-errors");
    let h = home.to_str().unwrap();
    let t = transcript(&home, 1);
    let (code, out, _) = run(&["watch", t.to_str().unwrap(), "--audit-json", "--home", h]);
    let doc = check(code, &out);
    assert_eq!(text(&obj(&list(&obj(&doc)["errors"])[0])["message"]), "--audit-json needs --once");
    let odd = home.join("odd.jsonl");
    fs::write(&odd, "not a transcript\n").unwrap();
    let (code, out, err) = run(&["watch", odd.to_str().unwrap(), "--once", "--audit-json", "--home", h]);
    let doc = check(code, &out);
    assert!(text(&obj(&list(&obj(&doc)["errors"])[0])["message"]).contains("not a transcript format glassbox knows"));
    assert!(err.contains("not a transcript format glassbox knows"));
}
