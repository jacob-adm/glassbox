//! Follow a local agent transcript and capture the machine's state when a trigger fires.
//!
//! The transcript is read for structure only (tool names, argument hashes, token counts,
//! error flags); its content never reaches the output or a capture.
use crate::json::{self, Value};
use crate::persist::{self, Item};
use crate::safe::{json_escape, visible};
use crate::sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, BufRead, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy)]
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
    /// Tool name in canonical JSON-escaped form (as stored by `json::Value::Str`).
    pub tool: Option<String>,
    /// SHA-256 of the tool name and canonical arguments; identifies the call without its content.
    pub fingerprint: Option<String>,
    /// Line number in the transcript file.
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
        // The tool name is already in escaped form; the fingerprint is hex.
        let raw = |v: &Option<String>| v.as_ref().map_or("null".into(), |s| format!("\"{s}\""));
        format!(
            "{{\"kind\":\"{}\",\"value\":{},\"threshold\":{},\"tool\":{},\"fingerprint\":{},\"line\":{},\"transcript\":\"{}\"}}",
            self.kind,
            self.value,
            self.threshold,
            raw(&self.tool),
            raw(&self.fingerprint),
            self.line,
            json_escape(&transcript.display().to_string())
        )
    }
}

/// Tracks the trigger conditions over a stream of transcript lines.
pub struct Detector {
    t: Thresholds,
    last_call: Option<String>,
    last_message: Option<String>,
    run: u64,
    failed: u64,
    pub recognised: u64,
    pub line: u64,
}

impl Detector {
    /// `lines_before`: lines already in the transcript, so reported line numbers match the file.
    pub fn new(t: Thresholds, lines_before: u64) -> Self {
        Detector { t, last_call: None, last_message: None, run: 0, failed: 0, recognised: 0, line: lines_before }
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
        // One model response is written as several lines (one per content block), each
        // repeating the response's id and usage: count it once.
        let id = get(message, "id").and_then(as_str).map(str::to_string);
        let new_response = id.is_none() || id != self.last_message;
        if id.is_some() {
            self.last_message = id;
        }
        if new_response {
            if let Some(out) = get(message, "usage").and_then(|u| get(u, "output_tokens")).and_then(as_u64) {
                if out > self.t.spike {
                    fired.push(self.trigger("spike", out, self.t.spike, None, None));
                }
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

/// Calls `f` with every line of `path` (without the newline), streaming, never holding the file.
pub fn for_each_line(path: &Path, mut f: impl FnMut(&str)) -> io::Result<()> {
    let reader = io::BufReader::new(crate::safe::open_regular(path)?);
    for line in reader.split(b'\n') {
        f(&String::from_utf8_lossy(&line?));
    }
    Ok(())
}

/// Number of complete lines in `path`.
pub fn line_count(path: &Path) -> io::Result<u64> {
    let mut file = crate::safe::open_regular(path)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut n = 0u64;
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            return Ok(n);
        }
        n += buf[..read].iter().filter(|b| **b == b'\n').count() as u64;
    }
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
///
/// The directory is always newly created (never one that already exists, which a
/// watched agent could have planted with symlinks inside), and every file in it is
/// created new, so nothing is ever written through a link.
pub fn capture(out: &Path, seq: u64, trigger: &Trigger, ctx: &Context) -> io::Result<PathBuf> {
    fs::create_dir_all(out)?;
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let base = format!("{secs}-{seq}-{}", trigger.kind);
    let mut dir = out.join(&base);
    let mut n = 1;
    loop {
        match fs::create_dir(&dir) {
            Ok(()) => break,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && n < 1000 => {
                n += 1;
                dir = out.join(format!("{base}-{n}"));
            }
            Err(e) => return Err(e),
        }
    }
    let write_new = |name: &str, text: &str| -> io::Result<()> {
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(dir.join(name))?;
        f.write_all(text.as_bytes())
    };
    if let Some(pid) = ctx.pid {
        let (tree, pids) = process_tree(pid);
        write_new("processes.txt", &tree)?;
        write_new("connections.txt", &connections(pid, &pids))?;
    }
    if !ctx.repos.is_empty() {
        write_new("repos.txt", &repo_changes(ctx.repos))?;
    }
    let changes = match ctx.baseline {
        Some(before) => persist::diff(before, &persist::collect(ctx.home, ctx.repos)),
        None => Vec::new(),
    };
    let items: Vec<String> = changes
        .iter()
        .map(|(c, i)| {
            format!("{{\"change\":\"{}\",\"category\":\"{}\",\"path\":\"{}\"}}", c.as_str(), json_escape(&i.category), json_escape(&i.path))
        })
        .collect();
    write_new("persist.json", &format!("{{\"since_watch_began\":{},\"changes\":[{}]}}\n", ctx.baseline.is_some(), items.join(",")))?;
    // Last, so a reader that sees trigger.json sees a complete capture.
    write_new("trigger.json", &(trigger.to_json(ctx.transcript) + "\n"))?;
    Ok(dir)
}

/// Every process as (pid, parent, state, command name). Command arguments are never read:
/// they can hold secrets. Linux reads /proc (minimal systems have no ps); macOS uses ps.
fn processes() -> Vec<(i32, i32, String, String)> {
    let mut all = Vec::new();
    if cfg!(target_os = "linux") {
        for entry in fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else { continue };
            let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else { continue };
            let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else { continue };
            let mut f = stat[close + 1..].split_whitespace();
            let (Some(state), Some(ppid)) = (f.next(), f.next().and_then(|p| p.parse().ok())) else { continue };
            all.push((pid, ppid, state.to_string(), stat[open + 1..close].to_string()));
        }
    } else if let Ok(out) = Command::new("ps").args(["-A", "-o", "pid=,ppid=,stat=,comm="]).output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let mut f = line.split_whitespace();
            let (Some(pid), Some(ppid)) = (f.next().and_then(|p| p.parse().ok()), f.next().and_then(|p| p.parse().ok())) else {
                continue;
            };
            let state = f.next().unwrap_or("?").to_string();
            all.push((pid, ppid, state, f.collect::<Vec<_>>().join(" ")));
        }
    }
    all
}

/// pid, parent, state and command name for `root` and every process below it. Each field
/// is made visible on its own, so a name cannot add columns or lines.
fn process_tree(root: i32) -> (String, Vec<i32>) {
    let mut children: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
    let mut lines: BTreeMap<i32, String> = BTreeMap::new();
    for (pid, ppid, state, comm) in processes() {
        lines.insert(pid, format!("{pid}\t{ppid}\t{}\t{}", visible(&state), visible(&comm)));
        children.entry(ppid).or_default().push(pid);
    }
    let mut out = String::from("pid\tppid\tstate\tcommand\n");
    let mut seen = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        if !seen.insert(pid) {
            continue;
        }
        if let Some(l) = lines.get(&pid) {
            out.push_str(l);
            out.push('\n');
        }
        stack.extend(children.get(&pid).into_iter().flatten().copied());
    }
    (out, seen.into_iter().collect())
}

const CONNECTIONS_NOTE: &str = "# Open connections of the process tree when this capture was written. A point-in-time list: \
connections that opened and closed earlier are not shown.\n";

/// Open TCP and UDP sockets of `pids`, one per line: pid, protocol, local, remote, state.
/// Read as the same user, with no extra privileges. Each field is made visible on its own.
fn connections(root: i32, pids: &[i32]) -> String {
    let mut out = String::from(CONNECTIONS_NOTE);
    let rows = if cfg!(target_os = "linux") { proc_sockets(root, pids) } else { lsof_sockets(pids) };
    match rows {
        Ok(rows) => {
            out.push_str("pid\tprotocol\tlocal\tremote\tstate\n");
            for (pid, proto, local, remote, state) in rows {
                out.push_str(&format!("{pid}\t{}\t{}\t{}\t{}\n", visible(&proto), visible(&local), visible(&remote), visible(&state)));
            }
        }
        Err(why) => out.push_str(&format!("# unavailable: {}\n", visible(&why))),
    }
    out
}

type Socket = (i32, String, String, String, String);

/// macOS and other Unix systems: `lsof` field output (-F), so names cannot shift columns.
fn lsof_sockets(pids: &[i32]) -> Result<Vec<Socket>, String> {
    let list = pids.iter().map(i32::to_string).collect::<Vec<_>>().join(",");
    let out = Command::new("lsof").args(["-nP", "-w", "-a", "-i", "-p", &list, "-F", "pfPnT"]).output().map_err(|e| format!("lsof: {e}"))?;
    let mut rows = Vec::new();
    let mut pid = 0;
    let mut cur: Option<Socket> = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let (tag, val) = line.split_at(line.len().min(1));
        match tag {
            "p" => {
                rows.extend(cur.take());
                pid = val.parse().unwrap_or(0);
            }
            "f" => {
                rows.extend(cur.take());
                cur = Some((pid, String::new(), String::new(), String::new(), String::new()));
            }
            "P" => cur.iter_mut().for_each(|c| c.1 = val.to_string()),
            "n" => cur.iter_mut().for_each(|c| {
                let (l, r) = val.split_once("->").unwrap_or((val, ""));
                (c.2, c.3) = (l.to_string(), r.to_string());
            }),
            "T" => {
                if let Some(st) = val.strip_prefix("ST=") {
                    cur.iter_mut().for_each(|c| c.4 = st.to_string());
                }
            }
            _ => {}
        }
    }
    rows.extend(cur);
    Ok(rows)
}

/// Linux: socket inodes from /proc/<pid>/fd, matched against the root's network namespace tables.
fn proc_sockets(root: i32, pids: &[i32]) -> Result<Vec<Socket>, String> {
    let mut owner: BTreeMap<u64, i32> = BTreeMap::new();
    for &pid in pids {
        for fd in fs::read_dir(format!("/proc/{pid}/fd")).into_iter().flatten().flatten() {
            let Ok(target) = fs::read_link(fd.path()) else { continue };
            let t = target.to_string_lossy();
            if let Some(inode) = t.strip_prefix("socket:[").and_then(|r| r.strip_suffix(']')).and_then(|n| n.parse().ok()) {
                owner.entry(inode).or_insert(pid);
            }
        }
    }
    let mut rows = Vec::new();
    for (file, proto, v6) in [("tcp", "TCP", false), ("tcp6", "TCP", true), ("udp", "UDP", false), ("udp6", "UDP", true)] {
        let Ok(table) = fs::read_to_string(format!("/proc/{root}/net/{file}")) else { continue };
        for line in table.lines().skip(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            let (Some(l), Some(r), Some(st), Some(inode)) = (f.get(1), f.get(2), f.get(3), f.get(9).and_then(|n| n.parse::<u64>().ok())) else { continue };
            let Some(&pid) = owner.get(&inode) else { continue };
            let state = if proto == "TCP" { tcp_state(st) } else if r.ends_with(":0000") { "" } else { "CONNECTED" };
            rows.push((pid, proto.to_string(), proc_addr(l, v6), if r.ends_with(":0000") { String::new() } else { proc_addr(r, v6) }, state.to_string()));
        }
    }
    if owner.is_empty() && !Path::new(&format!("/proc/{root}/fd")).exists() {
        return Err(format!("/proc/{root}/fd not readable"));
    }
    Ok(rows)
}

/// "0100007F:1F90" -> "127.0.0.1:8080"; IPv6 as "[::1]:8080". Each 32-bit word is host order.
fn proc_addr(hex: &str, v6: bool) -> String {
    let Some((addr, port)) = hex.split_once(':') else { return visible(hex) };
    let port = u16::from_str_radix(port, 16).unwrap_or(0);
    let words: Vec<u32> = (0..addr.len() / 8).filter_map(|i| u32::from_str_radix(&addr[i * 8..i * 8 + 8], 16).ok()).collect();
    if v6 && words.len() == 4 {
        let mut b = [0u8; 16];
        for (i, w) in words.iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        format!("[{}]:{port}", std::net::Ipv6Addr::from(b))
    } else if let Some(w) = words.first() {
        format!("{}:{port}", std::net::Ipv4Addr::from(w.to_le_bytes()))
    } else {
        visible(hex)
    }
}

fn tcp_state(code: &str) -> &'static str {
    match code {
        "01" => "ESTABLISHED",
        "02" => "SYN_SENT",
        "03" => "SYN_RECV",
        "04" => "FIN_WAIT1",
        "05" => "FIN_WAIT2",
        "06" => "TIME_WAIT",
        "07" => "CLOSE",
        "08" => "CLOSE_WAIT",
        "09" => "LAST_ACK",
        "0A" => "LISTEN",
        "0B" => "CLOSING",
        _ => "UNKNOWN",
    }
}

/// File names and states of uncommitted changes in each repository (names only, no diffs).
/// Each line is made visible on its own, so a file name cannot forge lines.
fn repo_changes(repos: &[PathBuf]) -> String {
    let mut out = String::new();
    for repo in repos {
        out.push_str(&format!("# {}\n", visible(&repo.display().to_string())));
        match Command::new("git").arg("-C").arg(repo).args(["status", "--porcelain=v1", "-z"]).output() {
            Ok(o) if o.status.success() => {
                for entry in o.stdout.split(|b| *b == 0).filter(|e| !e.is_empty()) {
                    out.push_str(&visible(&String::from_utf8_lossy(entry)));
                    out.push('\n');
                }
            }
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
const EPERM: i32 = 1;

/// Stop `pid` (it can be resumed with `kill -CONT`).
pub fn pause(pid: i32) -> io::Result<()> {
    if unsafe { kill(pid, SIGSTOP) } == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// Whether `pid` exists. EPERM means it exists but belongs to another user.
pub fn alive(pid: i32) -> bool {
    let exists = unsafe { kill(pid, 0) } == 0;
    exists || io::Error::last_os_error().raw_os_error() == Some(EPERM)
}

/// Reads complete lines appended to a file since the last call.
pub struct Follower {
    path: PathBuf,
    file: fs::File,
    id: (u64, u64),
    pos: u64,
    partial: Vec<u8>,
}

impl Follower {
    /// Start at the end: what is already in the file is history, not new activity.
    pub fn at_end(path: &Path) -> io::Result<Self> {
        let file = crate::safe::open_regular(path)?;
        let m = file.metadata()?;
        Ok(Follower { path: path.to_path_buf(), id: (m.dev(), m.ino()), pos: m.len(), file, partial: Vec::new() })
    }

    /// New complete lines (without their newline). Bytes are kept until a line is complete,
    /// so a character split across two reads is decoded whole. If the path now names a
    /// different file (replaced by rename) or the file shrank (truncated), reading starts
    /// again from the beginning of what is there now.
    pub fn new_lines(&mut self) -> io::Result<Vec<String>> {
        if let Ok(m) = fs::metadata(&self.path) {
            if (m.dev(), m.ino()) != self.id {
                if let Ok(file) = crate::safe::open_regular(&self.path) {
                    self.file = file;
                    self.id = (m.dev(), m.ino());
                    self.pos = 0;
                    self.partial.clear();
                }
            }
        }
        let len = self.file.metadata()?.len();
        if len < self.pos {
            self.pos = 0;
            self.partial.clear();
        }
        if len == self.pos {
            return Ok(Vec::new());
        }
        self.file.seek(SeekFrom::Start(self.pos))?;
        (&mut self.file).take(len - self.pos).read_to_end(&mut self.partial)?;
        self.pos = len;
        let mut lines = Vec::new();
        while let Some(i) = self.partial.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=i).collect();
            lines.push(String::from_utf8_lossy(&line[..i]).into_owned());
        }
        Ok(lines)
    }
}

/// Whether the transcript looks like a supported format: true if it has no complete line
/// yet (a first line still being written is not evidence either way), or if any of its
/// first complete lines is a recognised record.
pub fn recognised_format(path: &Path) -> io::Result<bool> {
    let (raw, _) = crate::safe::read_capped(path, 4 << 20)?;
    let Some(end) = raw.iter().rposition(|b| *b == b'\n') else { return Ok(true) };
    let text = String::from_utf8_lossy(&raw[..end]);
    let mut d = Detector::new(DEFAULTS, 0);
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
