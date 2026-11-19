//! Follow a local agent transcript and capture the machine's state when a trigger fires.
//!
//! The transcript is read for structure only (tool names, argument hashes, token counts,
//! error flags); its content never reaches the output or a capture.
use crate::json::{self, Value};
use crate::persist::{self, Item};
use crate::safe::visible;
use crate::sha256;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Thresholds {
    /// Identical tool calls (same tool, same arguments) in a row.
    pub repeats: u64,
    /// Output tokens in one assistant turn.
    pub spike: u64,
    /// Failed tool results in a row.
    pub failures: u64,
}

pub const DEFAULTS: Thresholds = Thresholds { repeats: 4, spike: 16_000, failures: 5 };

#[derive(Debug, Clone)]
pub struct Trigger {
    pub kind: &'static str,
    pub value: u64,
    pub threshold: u64,
    pub tool: Option<String>,
    /// SHA-256 of the tool name and canonical arguments; identifies the call without its content.
    pub fingerprint: Option<String>,
    pub line: u64,
}

impl Trigger {
    pub fn describe(&self) -> String {
        match self.kind {
            "loop" => format!(
                "loop: {} identical {} calls in a row (line {})",
                self.value,
                visible(self.tool.as_deref().unwrap_or("?")),
                self.line
            ),
            "spike" => format!("spike: {} output tokens in one turn, threshold {} (line {})", self.value, self.threshold, self.line),
            _ => format!("failures: {} failed tool calls in a row (line {})", self.value, self.line),
        }
    }

    pub fn to_json(&self, transcript: &Path) -> String {
        let opt = |v: &Option<String>| v.as_ref().map_or("null".into(), |s| format!("\"{}\"", esc(s)));
        format!(
            "{{\"kind\":\"{}\",\"value\":{},\"threshold\":{},\"tool\":{},\"fingerprint\":{},\"line\":{},\"transcript\":\"{}\"}}",
            self.kind,
            self.value,
            self.threshold,
            opt(&self.tool),
            opt(&self.fingerprint),
            self.line,
            esc(&transcript.display().to_string())
        )
    }
}

/// Tracks the trigger conditions over a stream of transcript lines.
pub struct Detector {
    t: Thresholds,
    last_call: Option<String>,
    run: u64,
    failed: u64,
    pub recognised: u64,
    pub line: u64,
}

impl Detector {
    pub fn new(t: Thresholds) -> Self {
        Detector { t, last_call: None, run: 0, failed: 0, recognised: 0, line: 0 }
    }

    /// Feed one transcript line; returns the triggers it fires.
    pub fn feed(&mut self, line: &str) -> Vec<Trigger> {
        self.line += 1;
        let mut fired = Vec::new();
        let Some(v) = json::parse(line.trim()) else { return fired };
        let kind = get(&v, "type").and_then(as_str);
        let Some(message) = get(&v, "message").filter(|_| matches!(kind, Some("assistant" | "user"))) else {
            return fired;
        };
        self.recognised += 1;
        if let Some(out) = get(message, "usage").and_then(|u| get(u, "output_tokens")).and_then(as_u64) {
            if out > self.t.spike {
                fired.push(self.trigger("spike", out, self.t.spike, None, None));
            }
        }
        let Some(Value::Arr(blocks)) = get(message, "content") else { return fired };
        for block in blocks {
            match get(block, "type").and_then(as_str) {
                Some("tool_use") => {
                    let name = get(block, "name").and_then(as_str).unwrap_or("?").to_string();
                    let args = get(block, "input").map(Value::canonical).unwrap_or_default();
                    let fp = sha256::hex(format!("{name}\n{args}").as_bytes());
                    self.run = if self.last_call.as_deref() == Some(fp.as_str()) { self.run + 1 } else { 1 };
                    self.last_call = Some(fp.clone());
                    // Fire once when the run reaches the threshold, not on every call after it.
                    if self.run == self.t.repeats {
                        fired.push(self.trigger("loop", self.run, self.t.repeats, Some(name), Some(fp)));
                    }
                }
                Some("tool_result") => {
                    if get(block, "is_error") == Some(&Value::Bool(true)) {
                        self.failed += 1;
                        if self.failed == self.t.failures {
                            fired.push(self.trigger("failures", self.failed, self.t.failures, None, None));
                        }
                    } else {
                        self.failed = 0;
                    }
                }
                _ => {}
            }
        }
        fired
    }

    fn trigger(&self, kind: &'static str, value: u64, threshold: u64, tool: Option<String>, fingerprint: Option<String>) -> Trigger {
        Trigger { kind, value, threshold, tool, fingerprint, line: self.line }
    }
}

fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Obj(o) => o.get(key),
        _ => None,
    }
}

fn as_str(v: &Value) -> Option<&str> {
    match v {
        Value::Str(s) => Some(s),
        _ => None,
    }
}

fn as_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Num(n) => n.parse().ok(),
        _ => None,
    }
}

fn esc(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// The most recently modified Claude Code transcript under `home`.
pub fn latest_transcript(home: &Path) -> Option<PathBuf> {
    let projects = home.join(".claude/projects");
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for project in fs::read_dir(projects).ok()?.flatten() {
        for file in fs::read_dir(project.path()).into_iter().flatten().flatten() {
            let p = file.path();
            if p.extension().is_some_and(|e| e == "jsonl") {
                if let Ok(t) = file.metadata().and_then(|m| m.modified()) {
                    if newest.as_ref().is_none_or(|(n, _)| t > *n) {
                        newest = Some((t, p));
                    }
                }
            }
        }
    }
    newest.map(|(_, p)| p)
}

/// What a capture needs besides the trigger.
pub struct Context<'a> {
    pub transcript: &'a Path,
    pub home: &'a Path,
    pub repos: &'a [PathBuf],
    pub pid: Option<i32>,
    pub baseline: Option<&'a [Item]>,
}

/// Write a capture directory for `trigger` under `out`; returns its path.
pub fn capture(out: &Path, seq: u64, trigger: &Trigger, ctx: &Context) -> io::Result<PathBuf> {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let dir = out.join(format!("{secs}-{seq}-{}", trigger.kind));
    fs::create_dir_all(&dir)?;
    // Written last-but-one so a reader that sees trigger.json and persist.json sees a complete capture.
    if let Some(pid) = ctx.pid {
        fs::write(dir.join("processes.txt"), process_tree(pid))?;
    }
    if !ctx.repos.is_empty() {
        fs::write(dir.join("repos.txt"), repo_changes(ctx.repos))?;
    }
    let changes = match ctx.baseline {
        Some(before) => persist::diff(before, &persist::collect(ctx.home, ctx.repos)),
        None => Vec::new(),
    };
    let items: Vec<String> = changes
        .iter()
        .map(|(c, i)| format!("{{\"change\":\"{}\",\"category\":\"{}\",\"path\":\"{}\"}}", c.as_str(), esc(&i.category), esc(&i.path)))
        .collect();
    fs::write(dir.join("persist.json"), format!("{{\"since_watch_began\":{},\"changes\":[{}]}}\n", ctx.baseline.is_some(), items.join(",")))?;
    fs::write(dir.join("trigger.json"), trigger.to_json(ctx.transcript) + "\n")?;
    Ok(dir)
}

/// Every process as (pid, parent, "state<TAB>command name"). Command arguments are never
/// read: they can hold secrets. Linux reads /proc (minimal systems have no ps); macOS uses ps.
fn processes() -> Vec<(i32, i32, String)> {
    let mut all = Vec::new();
    if cfg!(target_os = "linux") {
        for entry in fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else { continue };
            let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else { continue };
            let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else { continue };
            let mut f = stat[close + 1..].split_whitespace();
            let (Some(state), Some(ppid)) = (f.next(), f.next().and_then(|p| p.parse().ok())) else { continue };
            all.push((pid, ppid, format!("{state}\t{}", &stat[open + 1..close])));
        }
    } else if let Ok(out) = Command::new("ps").args(["-A", "-o", "pid=,ppid=,stat=,comm="]).output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let mut f = line.split_whitespace();
            let (Some(pid), Some(ppid)) = (f.next().and_then(|p| p.parse().ok()), f.next().and_then(|p| p.parse().ok())) else {
                continue;
            };
            let state = f.next().unwrap_or("?").to_string();
            all.push((pid, ppid, format!("{state}\t{}", f.collect::<Vec<_>>().join(" "))));
        }
    }
    all
}

/// pid, parent, state and command name for `root` and every process below it.
fn process_tree(root: i32) -> String {
    let mut children: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
    let mut lines: BTreeMap<i32, String> = BTreeMap::new();
    for (pid, ppid, rest) in processes() {
        lines.insert(pid, format!("{pid}\t{ppid}\t{}", visible(&rest).replace("\\t", "\t")));
        children.entry(ppid).or_default().push(pid);
    }
    let mut out = String::from("pid\tppid\tstate\tcommand\n");
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        if let Some(l) = lines.get(&pid) {
            out.push_str(l);
            out.push('\n');
        }
        stack.extend(children.get(&pid).into_iter().flatten().copied());
    }
    out
}

/// File names and states of uncommitted changes in each repository (names only, no diffs).
fn repo_changes(repos: &[PathBuf]) -> String {
    let mut out = String::new();
    for repo in repos {
        out.push_str(&format!("# {}\n", visible(&repo.display().to_string())));
        match Command::new("git").arg("-C").arg(repo).args(["status", "--porcelain=v1"]).output() {
            Ok(o) if o.status.success() => out.push_str(&visible(&String::from_utf8_lossy(&o.stdout)).replace("\\n", "\n")),
            _ => out.push_str("git status unavailable\n"),
        }
    }
    out
}

unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}
#[cfg(target_os = "macos")]
const SIGSTOP: i32 = 17;
#[cfg(not(target_os = "macos"))]
const SIGSTOP: i32 = 19;

/// Stop `pid` (it can be resumed with `kill -CONT`).
pub fn pause(pid: i32) -> io::Result<()> {
    if unsafe { kill(pid, SIGSTOP) } == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

pub fn alive(pid: i32) -> bool {
    unsafe { kill(pid, 0) == 0 }
}

/// Reads complete lines appended to a file since the last call.
pub struct Follower {
    file: fs::File,
    pos: u64,
    partial: String,
}

impl Follower {
    /// Start at the end: what is already in the file is history, not new activity.
    pub fn at_end(path: &Path) -> io::Result<Self> {
        let file = crate::safe::open_regular(path)?;
        let pos = file.metadata()?.len();
        Ok(Follower { file, pos, partial: String::new() })
    }

    pub fn new_lines(&mut self) -> io::Result<Vec<String>> {
        let len = self.file.metadata()?.len();
        if len < self.pos {
            // Truncated or replaced: start again from the beginning of what is there now.
            self.pos = 0;
            self.partial.clear();
        }
        if len == self.pos {
            return Ok(Vec::new());
        }
        self.file.seek(SeekFrom::Start(self.pos))?;
        let mut buf = Vec::new();
        (&mut self.file).take(len - self.pos).read_to_end(&mut buf)?;
        self.pos = len;
        self.partial.push_str(&String::from_utf8_lossy(&buf));
        let mut lines: Vec<String> = self.partial.split('\n').map(str::to_string).collect();
        self.partial = lines.pop().unwrap_or_default();
        Ok(lines.into_iter().filter(|l| !l.trim().is_empty()).collect())
    }
}

/// Whether existing transcript content looks like a supported format: true if empty,
/// or if any of the first lines is a recognised record.
pub fn recognised_format(path: &Path) -> io::Result<bool> {
    let (raw, _) = crate::safe::read_capped(path, 4 << 20)?;
    let text = String::from_utf8_lossy(&raw);
    let mut d = Detector::new(DEFAULTS);
    let mut any = false;
    for line in text.lines().filter(|l| !l.trim().is_empty()).take(500) {
        any = true;
        d.feed(line);
        if d.recognised > 0 {
            return Ok(true);
        }
    }
    Ok(!any)
}
