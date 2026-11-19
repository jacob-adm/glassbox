//! Reach audit: what could a process started like this one read?
//!
//! Each location is probed by opening it read-only. Directories are opened
//! but never listed, files are opened but never read, nothing is written.
//! A child process (an agent) started from the same parent inherits the same
//! user, permissions and, on macOS, the privacy grants of the responsible app,
//! so this process's reach is the agent's reach.
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const EPERM: i32 = 1;
const EACCES: i32 = 13;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// The open succeeded.
    Readable,
    /// EPERM: blocked by a privacy control or sandbox (macOS TCC, SIP, Seatbelt).
    DeniedPrivacy,
    /// EACCES: blocked by ordinary permissions or ACLs.
    DeniedPermissions,
    /// Nothing at this path.
    Absent,
    /// Any other error, with its errno.
    Error(i32),
}

impl Reach {
    pub fn as_str(&self) -> String {
        match self {
            Reach::Readable => "readable".into(),
            Reach::DeniedPrivacy => "denied-privacy".into(),
            Reach::DeniedPermissions => "denied-permissions".into(),
            Reach::Absent => "absent".into(),
            Reach::Error(e) => format!("error-{e}"),
        }
    }
}

/// Open `path` read-only without reading anything from it.
pub fn probe(path: &Path) -> Reach {
    let opened = match fs::symlink_metadata(path) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Reach::Absent,
        Ok(m) if m.is_dir() => fs::read_dir(path).map(drop),
        _ => fs::File::open(path).map(drop),
    };
    match opened {
        Ok(()) => Reach::Readable,
        Err(e) => match e.raw_os_error() {
            Some(EPERM) => Reach::DeniedPrivacy,
            Some(EACCES) => Reach::DeniedPermissions,
            _ if e.kind() == ErrorKind::NotFound => Reach::Absent,
            Some(code) => Reach::Error(code),
            None => Reach::Error(-1),
        },
    }
}

pub struct Location {
    pub category: &'static str,
    /// Path relative to the home directory.
    pub rel: &'static str,
    /// Readable here means an agent can read secrets or private data.
    pub sensitive: bool,
}

const fn loc(category: &'static str, rel: &'static str, sensitive: bool) -> Location {
    Location { category, rel, sensitive }
}

const COMMON: &[Location] = &[
    loc("credentials", ".ssh", true),
    loc("credentials", ".aws/credentials", true),
    loc("credentials", ".config/gcloud", true),
    loc("credentials", ".azure", true),
    loc("credentials", ".kube/config", true),
    loc("credentials", ".docker/config.json", true),
    loc("credentials", ".netrc", true),
    loc("credentials", ".npmrc", true),
    loc("credentials", ".pypirc", true),
    loc("credentials", ".git-credentials", true),
    loc("credentials", ".config/gh/hosts.yml", true),
    loc("credentials", ".gnupg", true),
    loc("credentials", ".password-store", true),
    loc("history", ".zsh_history", true),
    loc("history", ".bash_history", true),
    loc("history", ".python_history", true),
];

#[cfg(target_os = "macos")]
const PLATFORM: &[Location] = &[
    loc("private data", "Library/Mail", true),
    loc("private data", "Library/Messages", true),
    loc("private data", "Library/Safari", true),
    loc("private data", "Library/Application Support/AddressBook", true),
    loc("private data", "Library/Calendars", true),
    loc("private data", "Pictures/Photos Library.photoslibrary", true),
    loc("credentials", "Library/Keychains", true),
    loc("browser profile", "Library/Application Support/Google/Chrome", true),
    loc("browser profile", "Library/Application Support/Firefox", true),
    loc("browser profile", "Library/Application Support/BraveSoftware", true),
    loc("protected folder", "Desktop", false),
    loc("protected folder", "Documents", false),
    loc("protected folder", "Downloads", false),
    loc("protected folder", "Library/Mobile Documents", false),
];

#[cfg(not(target_os = "macos"))]
const PLATFORM: &[Location] = &[
    loc("credentials", ".local/share/keyrings", true),
    loc("browser profile", ".mozilla/firefox", true),
    loc("browser profile", ".config/google-chrome", true),
    loc("browser profile", ".config/chromium", true),
    loc("browser profile", ".config/BraveSoftware", true),
    loc("private data", ".thunderbird", true),
    loc("private data", ".local/share/evolution", true),
];

pub fn locations() -> impl Iterator<Item = &'static Location> {
    COMMON.iter().chain(PLATFORM.iter())
}

pub struct Finding {
    pub location: &'static Location,
    pub path: PathBuf,
    pub reach: Reach,
}

pub fn audit(home: &Path) -> Vec<Finding> {
    locations()
        .map(|l| {
            let path = home.join(l.rel);
            Finding { location: l, reach: probe(&path), path }
        })
        .collect()
}

/// Ancestors of `pid` as (pid, command), nearest first, ending at pid 1.
pub fn parent_chain(pid: u32) -> Vec<(u32, String)> {
    let mut chain = Vec::new();
    let Some((mut cur, _)) = ps(pid) else { return chain };
    while cur > 0 && chain.len() < 64 {
        let Some((ppid, comm)) = ps(cur) else { break };
        chain.push((cur, comm));
        if cur == 1 || ppid == cur {
            break;
        }
        cur = ppid;
    }
    chain
}

/// (parent pid, command) of `pid`, from /proc. Minimal images often lack ps(1).
#[cfg(target_os = "linux")]
fn ps(pid: u32) -> Option<(u32, String)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Format: pid (comm) state ppid ...; comm may itself contain spaces or ')'.
    let close = stat.rfind(')')?;
    let comm = &stat[stat.find('(')? + 1..close];
    let ppid = stat[close + 1..].split_whitespace().nth(1)?.parse().ok()?;
    let argv0 = fs::read(format!("/proc/{pid}/cmdline")).ok().and_then(|b| {
        b.split(|c| *c == 0).next().filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned())
    });
    Some((ppid, argv0.unwrap_or_else(|| comm.to_string())))
}

/// (parent pid, command) of `pid`, from ps(1).
#[cfg(not(target_os = "linux"))]
fn ps(pid: u32) -> Option<(u32, String)> {
    let out = std::process::Command::new("ps").args(["-o", "ppid=", "-o", "comm=", "-p", &pid.to_string()]).output().ok()?;
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let (ppid, comm) = line.split_once(char::is_whitespace)?;
    Some((ppid.trim().parse().ok()?, comm.trim().to_string()))
}

/// On macOS, privacy grants belong to the responsible app: the outermost .app in the chain.
pub fn responsible_app(chain: &[(u32, String)]) -> Option<String> {
    chain.iter().rev().find_map(|(_, c)| {
        c.find(".app/").map(|i| c[..i + 4].rsplit('/').next().unwrap_or(c).to_string())
    })
}
