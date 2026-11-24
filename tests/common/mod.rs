//! Shared by the tests that compare persistence snapshots.

/// Locations outside the temporary home that `persist` also watches. They belong to the
/// whole machine, so another process (on CI, the runner registering its own login items)
/// can change them while a test runs.
const SYSTEM: &[&str] = &["/Library/", "/Applications/", "/private/var/db/", "/etc/", "crontab -l"];

/// Drops changes to system locations from the output of `persist --since --strict --json`
/// and recomputes the exit code from the changes that remain. Checks first that the real
/// exit code agrees with the real change list.
pub fn home_only((code, json): (i32, String)) -> (i32, String) {
    let body = json.trim().strip_prefix("{\"changes\":[").and_then(|s| s.strip_suffix("]}"));
    let Some(body) = body else { return (code, json) };
    let items: Vec<&str> =
        if body.is_empty() { vec![] } else { body.split("},{").map(|i| i.trim_start_matches('{').trim_end_matches('}')).collect() };
    let flagged = |list: &[&str]| list.iter().any(|i| !i.starts_with("\"change\":\"removed\"")) as i32;
    assert_eq!(code, flagged(&items), "exit code disagrees with the change list: {json}");
    let kept: Vec<&str> =
        items.into_iter().filter(|i| !SYSTEM.iter().any(|s| i.contains(&format!("\"path\":\"{s}")))).collect();
    let rebuilt: Vec<String> = kept.iter().map(|i| format!("{{{i}}}")).collect();
    (flagged(&kept), format!("{{\"changes\":[{}]}}\n", rebuilt.join(",")))
}

#[test]
fn system_changes_are_dropped_and_home_changes_kept() {
    let sys = r#"{"change":"changed","category":"login items","path":"/private/var/db/com.apple.backgroundtaskmanagement/BackgroundItems-v13.btm"}"#;
    let cron = r#"{"change":"added","category":"cron","path":"crontab -l"}"#;
    let home = r#"{"change":"added","category":"agent config","path":"~/.claude.json#mcpServers,hooks"}"#;
    let gone = r#"{"change":"removed","category":"autostart","path":"~/Library/LaunchAgents/x.plist"}"#;
    let wrap = |items: &[&str]| format!("{{\"changes\":[{}]}}\n", items.join(","));
    assert_eq!(home_only((1, wrap(&[sys, cron]))), (0, wrap(&[])));
    assert_eq!(home_only((1, wrap(&[sys, home]))), (1, wrap(&[home])));
    assert_eq!(home_only((1, wrap(&[gone, sys]))), (0, wrap(&[gone])));
    assert_eq!(home_only((0, wrap(&[]))), (0, wrap(&[])));
}

#[test]
#[should_panic(expected = "exit code disagrees")]
fn a_wrong_exit_code_is_still_caught() {
    home_only((0, r#"{"changes":[{"change":"added","category":"cron","path":"crontab -l"}]}"#.to_string()));
}
