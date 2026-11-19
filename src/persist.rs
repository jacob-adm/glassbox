//! Persistence points: places where something can be set up to run again later.
//!
//! Each point is hashed (SHA-256 of its content) so two snapshots can be compared:
//! a point that appears or whose content changes between them was added or changed.
use crate::{json, sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub category: String,
    pub path: String,
    pub hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    Added,
    Changed,
    Removed,
}

impl Change {
    pub fn as_str(&self) -> &'static str {
        match self {
            Change::Added => "added",
            Change::Changed => "changed",
            Change::Removed => "removed",
        }
    }
}

const SHELL: &[&str] = &[
    ".zshrc", ".zprofile", ".zshenv", ".zlogin", ".zlogout", ".bashrc", ".bash_profile", ".bash_login",
    ".bash_logout", ".profile", ".config/fish/config.fish",
];
const SSH: &[&str] = &[".ssh/rc", ".ssh/environment", ".ssh/authorized_keys", ".ssh/authorized_keys2", ".ssh/config"];
const GIT: &[&str] = &[".gitconfig", ".config/git/config"];
const AGENT_FILES: &[&str] = &[
    ".claude/settings.json", ".claude/settings.local.json", ".cursor/mcp.json", ".codex/config.toml",
    ".gemini/settings.json",
];
/// Files rewritten with usage data on every run: only these keys can make something run.
const AGENT_KEYED: &[(&str, &[&str])] = &[(".claude.json", &["mcpServers", "hooks"])];
const REPO_AGENT_FILES: &[&str] = &[".mcp.json", ".claude/settings.json", ".cursor/mcp.json", ".vscode/mcp.json"];

#[cfg(target_os = "macos")]
const HOME_DIRS: &[(&str, &str)] = &[("autostart", "Library/LaunchAgents"), ("shell startup", ".config/fish/conf.d")];
#[cfg(target_os = "macos")]
const SYSTEM_DIRS: &[(&str, &str)] = &[("autostart", "/Library/LaunchAgents"), ("system service", "/Library/LaunchDaemons")];
#[cfg(target_os = "macos")]
const SYSTEM_FILES: &[(&str, &str)] = &[];
#[cfg(target_os = "macos")]
const PYTHON_SITES: &[&str] = &["Library/Python"];

#[cfg(not(target_os = "macos"))]
const HOME_DIRS: &[(&str, &str)] = &[
    ("autostart", ".config/systemd/user"),
    ("autostart", ".local/share/systemd/user"),
    ("autostart", ".config/autostart"),
    ("shell startup", ".config/fish/conf.d"),
];
#[cfg(not(target_os = "macos"))]
const SYSTEM_DIRS: &[(&str, &str)] = &[
    ("system service", "/etc/systemd/system"),
    ("autostart", "/etc/xdg/autostart"),
    ("cron", "/etc/cron.d"),
    ("shell startup", "/etc/profile.d"),
];
#[cfg(not(target_os = "macos"))]
const SYSTEM_FILES: &[(&str, &str)] =
    &[("cron", "/etc/crontab"), ("preload", "/etc/ld.so.preload"), ("system service", "/etc/rc.local")];
#[cfg(not(target_os = "macos"))]
const PYTHON_SITES: &[&str] = &[".local/lib"];

/// Every persistence point under `home`, the system locations, and each repo's hooks and agent configs.
pub fn collect(home: &Path, repos: &[PathBuf]) -> Vec<Item> {
    let mut c = Collector { home, items: BTreeMap::new() };
    for (cat, list) in [("shell startup", SHELL), ("ssh", SSH), ("git config", GIT), ("agent config", AGENT_FILES)] {
        for rel in list {
            c.file(cat, &home.join(rel));
        }
    }
    for (rel, keys) in AGENT_KEYED {
        c.json_keys("agent config", &home.join(rel), keys);
    }
    for (cat, rel) in HOME_DIRS {
        c.dir(cat, &home.join(rel), 3);
    }
    for (cat, abs) in SYSTEM_DIRS {
        c.dir(cat, Path::new(abs), 2);
    }
    for (cat, abs) in SYSTEM_FILES {
        c.file(cat, Path::new(abs));
    }
    for rel in PYTHON_SITES {
        c.python_startup(&home.join(rel));
    }
    c.crontab();
    for repo in repos {
        c.dir("git hook", &repo.join(".git/hooks"), 1);
        c.file("git config", &repo.join(".git/config"));
        for rel in REPO_AGENT_FILES {
            c.file("agent config", &repo.join(rel));
        }
    }
    c.items.into_values().collect()
}

struct Collector<'a> {
    home: &'a Path,
    items: BTreeMap<String, Item>,
}

impl Collector<'_> {
    fn add(&mut self, category: &str, path: String, hash: String) {
        self.items.insert(path.clone(), Item { category: category.into(), path, hash });
    }

    fn show(&self, path: &Path) -> String {
        match path.strip_prefix(self.home) {
            Ok(rel) => format!("~/{}", rel.display()),
            Err(_) => path.display().to_string(),
        }
    }

    fn file(&mut self, category: &str, path: &Path) {
        if fs::symlink_metadata(path).is_ok() {
            let shown = self.show(path);
            self.add(category, shown, content_hash(path));
        }
    }

    fn dir(&mut self, category: &str, dir: &Path, depth: usize) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let p = entry.path();
            match fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() && depth > 0 => self.dir(category, &p, depth - 1),
                Ok(m) if !m.is_dir() => self.file(category, &p),
                _ => {}
            }
        }
    }

    fn json_keys(&mut self, category: &str, path: &Path, keys: &[&str]) {
        let Ok(text) = fs::read_to_string(path) else { return };
        let label = format!("{}#{}", self.show(path), keys.join(","));
        let hash = match json::parse(&text) {
            Some(v) => {
                let parts: Vec<String> = v.find(keys).iter().map(|(p, v)| format!("{p}={}", v.canonical())).collect();
                sha256::hex(parts.join("\n").as_bytes())
            }
            None => format!("unparsed:{}", sha256::hex(text.as_bytes())),
        };
        self.add(category, label, hash);
    }

    /// `.pth` files and user/site customize modules run on every Python start.
    fn python_startup(&mut self, root: &Path) {
        let mut stack = vec![(root.to_path_buf(), 4usize)];
        while let Some((dir, depth)) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let p = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if p.is_dir() && depth > 0 && !p.is_symlink() {
                    stack.push((p, depth - 1));
                } else if dir.ends_with("site-packages")
                    && (name.ends_with(".pth") || name == "usercustomize.py" || name == "sitecustomize.py")
                {
                    self.file("python startup", &p);
                }
            }
        }
    }

    fn crontab(&mut self) {
        let Ok(out) = std::process::Command::new("crontab").arg("-l").output() else { return };
        if out.status.success() {
            self.add("cron", "crontab -l".into(), sha256::hex(&out.stdout));
        }
    }
}

fn content_hash(path: &Path) -> String {
    match fs::read(path) {
        Ok(bytes) => sha256::hex(&bytes),
        Err(_) => match fs::symlink_metadata(path) {
            // Unreadable: fall back to what metadata can show.
            Ok(m) => format!("unreadable:{}:{:?}", m.len(), m.modified().ok()),
            Err(e) => format!("error:{}", e.kind()),
        },
    }
}

pub fn diff(before: &[Item], after: &[Item]) -> Vec<(Change, Item)> {
    let old: BTreeMap<&str, &Item> = before.iter().map(|i| (i.path.as_str(), i)).collect();
    let new: BTreeMap<&str, &Item> = after.iter().map(|i| (i.path.as_str(), i)).collect();
    let mut out = Vec::new();
    for (path, item) in &new {
        match old.get(path) {
            None => out.push((Change::Added, (*item).clone())),
            Some(prev) if prev.hash != item.hash => out.push((Change::Changed, (*item).clone())),
            _ => {}
        }
    }
    for (path, item) in &old {
        if !new.contains_key(path) {
            out.push((Change::Removed, (*item).clone()));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.path.cmp(&b.1.path)));
    out
}

const HEADER: &str = "# glassbox persist v1";

pub fn save(items: &[Item], file: &Path) -> io::Result<()> {
    let mut out = format!("{HEADER}\n");
    for i in items {
        out.push_str(&format!("{}\t{}\t{}\n", i.hash, escape(&i.category), escape(&i.path)));
    }
    fs::write(file, out)
}

pub fn load(file: &Path) -> io::Result<Vec<Item>> {
    let text = fs::read_to_string(file)?;
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a glassbox persist snapshot"));
    }
    lines
        .map(|l| {
            let mut f = l.splitn(3, '\t');
            match (f.next(), f.next(), f.next()) {
                (Some(h), Some(c), Some(p)) => Ok(Item { hash: h.into(), category: unescape(c), path: unescape(p) }),
                _ => Err(io::Error::new(io::ErrorKind::InvalidData, format!("bad snapshot line: {l}"))),
            }
        })
        .collect()
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}
