//! `--audit-json`: one audit result object (format `audit-result/1`) for a run.
//!
//! Each record of `--json` becomes a finding that keeps the record unchanged under `record`
//! and adds a rule id, a severity fixed by the record, one sentence and its evidence. A
//! finding that `--strict` would not fail on is `info`, so the exit status is the one
//! `--strict` gives.
use crate::safe::json_escape;

pub const FORMAT: &str = "audit-result/1";

/// One finding. `record` is the record's JSON text exactly as `--json` prints it.
pub struct Finding {
    pub rule: String,
    pub severity: &'static str,
    pub message: String,
    pub path: String,
    pub record: String,
}

/// The exit status the object states: 2 with errors, 1 with a finding above info, else 0.
pub fn exit_code(findings: &[Finding], errors: &[String]) -> u8 {
    if !errors.is_empty() {
        2
    } else if findings.iter().any(|f| f.severity != "info") {
        1
    } else {
        0
    }
}

/// The object as one line of JSON, keys in a fixed order.
pub fn render(home: &str, findings: &[Finding], errors: &[String]) -> String {
    let items: Vec<String> = findings
        .iter()
        .map(|f| {
            format!(
                "{{\"rule\":\"{}\",\"severity\":\"{}\",\"message\":\"{}\",\"evidence\":{{\"path\":\"{}\"}},\"record\":{}}}",
                json_escape(&f.rule),
                f.severity,
                json_escape(&f.message),
                json_escape(&f.path),
                f.record
            )
        })
        .collect();
    let errs: Vec<String> = errors.iter().map(|e| format!("{{\"message\":\"{}\"}}", json_escape(e))).collect();
    format!(
        "{{\"format\":\"{FORMAT}\",\"tool\":{{\"name\":\"glassbox\",\"version\":\"{}\"}},\"target\":{{\"kind\":\"host\",\"name\":\"{}\"}},\"exit\":{},\"findings\":[{}],\"errors\":[{}]}}",
        env!("CARGO_PKG_VERSION"),
        json_escape(if home.is_empty() { "-" } else { home }),
        exit_code(findings, errors),
        items.join(","),
        errs.join(",")
    )
}

/// `reach`: a sensitive location something can be read at is high; every other result is info.
pub fn reach_finding(category: &str, sensitive: bool, path: &str, reach: &str, was: Option<&str>, record: String) -> Finding {
    let open = matches!(reach, "readable" | "partly-readable");
    let state = match reach {
        "readable" => "is readable".to_string(),
        "partly-readable" => "is partly readable".to_string(),
        "listable-only" => "can be listed, but no entry in it can be read".to_string(),
        "denied-privacy" => "is refused by a privacy control or sandbox".to_string(),
        "denied-permissions" => "is refused by file permissions".to_string(),
        "absent" => "is not present".to_string(),
        other => format!("gives {other}"),
    };
    let kind = if sensitive { format!("Sensitive location {path} ({category})") } else { format!("{path} ({category})") };
    let mut message = format!("{kind} {state}");
    if let Some(w) = was.filter(|w| *w != reach) {
        message.push_str(&format!("; it was {w}"));
    }
    Finding {
        rule: format!("glassbox.reach.{reach}"),
        severity: if sensitive && open { "high" } else { "info" },
        message,
        path: path.to_string(),
        record,
    }
}

/// `persist --since`: an added or changed persistence point is high; a removed one is info.
pub fn persist_finding(change: &str, category: &str, path: &str, record: String) -> Finding {
    let verb = match change {
        "added" => "was added",
        "changed" => "changed",
        "removed" => "was removed",
        other => other,
    };
    Finding {
        rule: format!("glassbox.persist.{change}"),
        severity: if change == "removed" { "info" } else { "high" },
        message: format!("Persistence point {path} ({category}) {verb} since the snapshot"),
        path: path.to_string(),
        record,
    }
}
