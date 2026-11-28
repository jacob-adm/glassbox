//! Persistence points: places where something can be set up to run again later.
//!
//! Each point is hashed (SHA-256 of its content) so two snapshots can be compared:
//! a point that appears or whose content changes between them was added or changed.
use crate::{json, safe, sha256};
use std::os::unix::fs::FileTypeExt;
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
const REPO_AGENT_FILES: &[&str] =
    &[".mcp.json", ".claude/settings.json", ".claude/settings.local.json", ".cursor/mcp.json", ".vscode/mcp.json"];
/// Instructions agents load at the start of every session: changing them steers every later run.
const INSTRUCTION_FILES: &[&str] = &[".claude/CLAUDE.md", ".codex/AGENTS.md", ".gemini/GEMINI.md"];
const INSTRUCTION_DIRS: &[&str] = &[".claude/skills", ".claude/agents", ".claude/commands"];
const REPO_INSTRUCTION_FILES: &[&str] = &[
    "CLAUDE.md", ".claude/CLAUDE.md", "AGENTS.md", "GEMINI.md", ".cursorrules", ".windsurfrules",
    ".github/copilot-instructions.md",
];
/// Skill bundles nest scripts several levels below the skill directory.
const INSTRUCTION_DEPTH: usize = 6;
const REPO_INSTRUCTION_DIRS: &[&str] = &[".claude/skills", ".claude/agents", ".claude/commands", ".cursor/rules"];

#[cfg(target_os = "macos")]
const HOME_DIRS: &[(&str, &str)] = &[("autostart", "Library/LaunchAgents"), ("shell startup", ".config/fish/conf.d")];
#[cfg(target_os = "macos")]
const SYSTEM_DIRS: &[(&str, &str)] = &[("autostart", "/Library/LaunchAgents"), ("system service", "/Library/LaunchDaemons")];
#[cfg(target_os = "macos")]
const SYSTEM_FILES: &[(&str, &str)] = &[];
/// Background Task Management database (macOS 13+): rewritten when a login item or background
/// service is registered, approved or removed. `sfltool dumpbtm` (admin) shows its contents.
#[cfg(target_os = "macos")]
const BTM_DIR: Option<&str> = Some("/private/var/db/com.apple.backgroundtaskmanagement");
#[cfg(not(target_os = "macos"))]
const BTM_DIR: Option<&str> = None;
/// Folders whose app bundles can carry their own launchd plists and login-item helpers (SMAppService).
#[cfg(target_os = "macos")]
const APP_DIRS: &[&str] = &["/Applications", "~/Applications"];
#[cfg(not(target_os = "macos"))]
const APP_DIRS: &[&str] = &[];
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
    for rel in INSTRUCTION_FILES {
        c.file("agent instructions", &home.join(rel));
    }
    for rel in INSTRUCTION_DIRS {
        c.dir("agent instructions", &home.join(rel), INSTRUCTION_DEPTH);
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
    if let Some(dir) = BTM_DIR {
        c.dir("login items", Path::new(dir), 0);
    }
    for dir in APP_DIRS {
        c.app_bundles(&dir.strip_prefix("~/").map_or_else(|| PathBuf::from(dir), |rel| home.join(rel)));
    }
    for rel in PYTHON_SITES {
        c.python_startup(&home.join(rel));
    }
    c.crontab();
    for (i, repo) in repos.iter().enumerate() {
        // Labels are relative to the repository, so a snapshot of one clone or worktree
        // compares cleanly with another.
        let tag = if i == 0 { "<repo>".to_string() } else { format!("<repo{}>", i + 1) };
        let git_tag = format!("{tag}/.git");
        let (git_dir, common_dir) = git_dirs(repo);
        c.file_in("git config", &common_dir.join("config"), Some((&common_dir, &git_tag)));
        c.file_in("git config", &git_dir.join("config.worktree"), Some((&git_dir, &git_tag)));
        match hooks_path(home, repo, &common_dir, &git_dir) {
            // Inside the repository: <repo>/...; elsewhere: its own path (absolute or ~/...).
            Some(dir) => c.hooks_in(&dir, Some((repo, &tag))),
            None => c.hooks_in(&common_dir.join("hooks"), Some((&common_dir, &git_tag))),
        }
        for rel in REPO_AGENT_FILES {
            c.file_in("agent config", &repo.join(rel), Some((repo, &tag)));
        }
        for rel in REPO_INSTRUCTION_FILES {
            c.file_in("agent instructions", &repo.join(rel), Some((repo, &tag)));
        }
        for rel in REPO_INSTRUCTION_DIRS {
            c.dir_in("agent instructions", &repo.join(rel), INSTRUCTION_DEPTH, Some((repo, &tag)));
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
        self.file_in(category, path, None);
    }

    fn dir(&mut self, category: &str, dir: &Path, depth: usize) {
        self.dir_in(category, dir, depth, None);
    }

    /// Record `path`, labelled `<tag>/<path relative to base>` when `root` is (base, tag).
    fn file_in(&mut self, category: &str, path: &Path, root: Option<(&Path, &str)>) {
        if fs::symlink_metadata(path).is_ok() {
            let label = match root.and_then(|(base, tag)| path.strip_prefix(base).ok().map(|rel| (tag, rel))) {
                Some((tag, rel)) => format!("{tag}/{}", rel.display()),
                None => self.show(path),
            };
            self.add(category, label, content_hash(path));
        }
    }

    fn dir_in(&mut self, category: &str, dir: &Path, depth: usize, root: Option<(&Path, &str)>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let p = entry.path();
            match fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() && depth > 0 => self.dir_in(category, &p, depth - 1, root),
                // A symlinked directory (for example a skill linked from a plugin checkout) is
                // walked too, so edits inside it are seen; depth bounds any link loop.
                Ok(m) if m.file_type().is_symlink() && depth > 0 && fs::metadata(&p).is_ok_and(|t| t.is_dir()) => {
                    self.dir_in(category, &p, depth - 1, root)
                }
                Ok(m) if !m.is_dir() => self.file_in(category, &p, root),
                _ => {}
            }
        }
    }

    /// A git hooks directory. Git never runs a file named `*.sample` (the examples `git init`
    /// writes), so those are not persistence points; everything else is watched as by `dir_in`.
    fn hooks_in(&mut self, dir: &Path, root: Option<(&Path, &str)>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let p = entry.path();
            let sample = p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".sample"));
            match fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() => self.dir_in("git hook", &p, 0, root),
                Ok(m) if m.file_type().is_symlink() && fs::metadata(&p).is_ok_and(|t| t.is_dir()) => {
                    self.dir_in("git hook", &p, 0, root)
                }
                Ok(_) if sample => {}
                Ok(_) => self.file_in("git hook", &p, root),
                _ => {}
            }
        }
    }

    fn json_keys(&mut self, category: &str, path: &Path, keys: &[&str]) {
        if fs::symlink_metadata(path).is_err() {
            return;
        }
        let label = format!("{}#{}", self.show(path), keys.join(","));
        // Too large to parse, unreadable or not a regular file: compare the whole file instead.
        let raw = match safe::read_capped(path, JSON_CAP) {
            Ok((raw, size)) if size <= JSON_CAP => raw,
            _ => return self.add(category, label, content_hash(path)),
        };
        let text = String::from_utf8_lossy(&raw);
        let hash = match json::parse(&text) {
            Some(v) => {
                let parts: Vec<String> = v.find(keys).iter().map(|(p, v)| format!("{p}={}", v.canonical())).collect();
                sha256::hex(parts.join("\n").as_bytes())
            }
            None => format!("unparsed:{}", sha256::hex(text.as_bytes())),
        };
        self.add(category, label, hash);
    }

    /// Launchd plists and login-item helpers inside each app bundle. Symlinked bundles are skipped.
    fn app_bundles(&mut self, apps: &Path) {
        let Ok(entries) = fs::read_dir(apps) else { return };
        for entry in entries.flatten() {
            let app = entry.path();
            if app.extension().is_none_or(|e| e != "app") || !fs::symlink_metadata(&app).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            let lib = app.join("Contents/Library");
            self.dir("autostart", &lib.join("LaunchAgents"), 0);
            self.dir("system service", &lib.join("LaunchDaemons"), 0);
            let Ok(helpers) = fs::read_dir(lib.join("LoginItems")) else { continue };
            for helper in helpers.flatten() {
                if fs::symlink_metadata(helper.path()).is_ok_and(|m| m.is_dir()) {
                    self.file("login items", &helper.path().join("Contents/Info.plist"));
                }
            }
        }
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

/// Agent config files are parsed only up to this size; larger ones are compared whole.
const JSON_CAP: u64 = 64 << 20;
/// Larger files are compared by size and change time rather than read, so a huge sparse
/// file cannot stall the audit. A normal user cannot set a file's change time (ctime).
const HASH_LIMIT: u64 = 1 << 30;
const SPARSE_MIN: u64 = 1 << 20;

/// SHA-256 of a whole regular file. FIFOs, devices and sockets are never opened for
/// reading; they are identified by type (and a symlink by its target) instead.
fn content_hash(path: &Path) -> String {
    use std::os::unix::fs::MetadataExt;
    if let Ok(mut file) = safe::open_regular(path) {
        return match file.metadata() {
            // Very large, or mostly holes (a real persistence file is neither): compare by
            // metadata so planted files cannot make the audit read gigabytes.
            Ok(m) if m.len() > HASH_LIMIT || (m.len() > SPARSE_MIN && m.blocks() * 512 < m.len() / 2) => {
                format!("oversize:{}:{}.{}:{}", m.len(), m.ctime(), m.ctime_nsec(), m.ino())
            }
            _ => match safe::hash_file(&mut file) {
                Ok(digest) => sha256::to_hex(&digest),
                Err(e) => format!("error:{}", e.kind()),
            },
        };
    }
    match fs::metadata(path) {
        // A directory where a file is expected: compare the names it holds.
        Ok(m) if m.is_dir() => {
            let mut names: Vec<String> = fs::read_dir(path)
                .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
                .unwrap_or_default();
            names.sort();
            format!("directory:{}", sha256::hex(names.join("\n").as_bytes()))
        }
        Ok(m) if !m.is_file() => {
            let t = m.file_type();
            let kind = if t.is_fifo() {
                "fifo"
            } else if t.is_char_device() {
                "char-device"
            } else if t.is_block_device() {
                "block-device"
            } else if t.is_socket() {
                "socket"
            } else {
                "other"
            };
            let target = fs::read_link(path).map(|t| t.display().to_string()).unwrap_or_default();
            format!("special:{kind}:{}", sha256::hex(target.as_bytes()))
        }
        // Unreadable: fall back to what metadata can show.
        Ok(m) => format!("unreadable:{}:{:?}", m.len(), m.modified().ok()),
        Err(_) => match fs::read_link(path) {
            Ok(target) => format!("dangling:{}", sha256::hex(target.to_string_lossy().as_bytes())),
            Err(e) => format!("error:{}", e.kind()),
        },
    }
}

/// (git dir, common dir) of a working tree. In a linked worktree `.git` is a file naming
/// its own git dir, and hooks and the main config live in the common dir it points to.
fn git_dirs(repo: &Path) -> (PathBuf, PathBuf) {
    let dot_git = repo.join(".git");
    let pointer = fs::symlink_metadata(&dot_git).is_ok_and(|m| m.is_file());
    let read = |p: &Path| safe::read_capped(p, 4096).ok().map(|(b, _)| String::from_utf8_lossy(&b).trim().to_string());
    let Some(git_dir) = pointer
        .then(|| read(&dot_git))
        .flatten()
        .and_then(|t| t.strip_prefix("gitdir:").map(|g| repo.join(g.trim())))
    else {
        return (dot_git.clone(), dot_git);
    };
    let common = read(&git_dir.join("commondir")).map(|c| git_dir.join(c)).unwrap_or_else(|| git_dir.clone());
    (git_dir, common)
}

/// The hooks directory git uses, if `core.hooksPath` moves it: system, global, repository and
/// (when `extensions.worktreeConfig` is on) worktree configuration, later ones winning.
fn hooks_path(home: &Path, repo: &Path, common_dir: &Path, git_dir: &Path) -> Option<PathBuf> {
    let read = |p: &Path| safe::read_capped(p, 1 << 20).ok().map(|(b, _)| String::from_utf8_lossy(&b).into_owned());
    let repo_config = read(&common_dir.join("config")).unwrap_or_default();
    let mut configs = vec![
        read(Path::new("/etc/gitconfig")),
        read(&home.join(".config/git/config")),
        read(&home.join(".gitconfig")),
        Some(repo_config.clone()),
    ];
    if config_value(&repo_config, "extensions", "worktreeconfig").is_some_and(|v| v.eq_ignore_ascii_case("true")) {
        configs.push(read(&git_dir.join("config.worktree")));
    }
    let value = configs.iter().flatten().rev().find_map(|text| config_value(text, "core", "hookspath"))?;
    if value.is_empty() {
        return None;
    }
    Some(match value.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if value == "~" => home.to_path_buf(),
        None if Path::new(&value).is_absolute() => PathBuf::from(value),
        None => repo.join(value),
    })
}

/// Last value of `section.key` (names compared case-insensitively) in git config text,
/// with quotes removed and `#`/`;` comments outside quotes dropped.
fn config_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut current = String::new();
    let mut found = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix('[') {
            current = rest.split([']', ' ', '"']).next().unwrap_or("").to_ascii_lowercase();
            continue;
        }
        if current != section {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        if !k.trim().eq_ignore_ascii_case(key) {
            continue;
        }
        let (mut value, mut quoted) = (String::new(), false);
        for ch in v.trim().chars() {
            match ch {
                '"' => quoted = !quoted,
                '#' | ';' if !quoted => break,
                c => value.push(c),
            }
        }
        found = Some(value.trim().to_string());
    }
    found
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

/// v2: repository points are labelled <repo>/..., and agent instructions are watched.
const HEADER: &str = "# glassbox persist v2";
const SNAPSHOT_CAP: u64 = 256 << 20;

pub fn save(items: &[Item], file: &Path) -> io::Result<()> {
    let mut out = format!("{HEADER}\n");
    for i in items {
        out.push_str(&format!("{}\t{}\t{}\n", i.hash, escape(&i.category), escape(&i.path)));
    }
    // Write a new file next to the target and rename it into place: a FIFO or symlink
    // planted at the target is replaced, never opened or written through.
    let tmp = file.with_file_name(format!(
        ".{}.{}.tmp",
        file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        std::process::id()
    ));
    let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    std::io::Write::write_all(&mut f, out.as_bytes())?;
    f.sync_all()?;
    fs::rename(&tmp, file).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

pub fn load(file: &Path) -> io::Result<Vec<Item>> {
    // The snapshot may sit where the audited agent can write: read it like any other planted file.
    let (raw, size) = safe::read_capped(file, SNAPSHOT_CAP)?;
    if size > SNAPSHOT_CAP {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "snapshot is too large"));
    }
    let text = String::from_utf8(raw).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "snapshot is not UTF-8"))?;
    // A trailing \r is a line-ending artefact (CRLF): paths store \r escaped as \\r.
    let mut lines = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).filter(|l| !l.is_empty());
    let header = lines.next();
    if header.is_some_and(|h| h.starts_with("# glassbox persist v") && h != HEADER) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "snapshot was written by another glassbox version; save a new one"));
    }
    if header != Some(HEADER) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a glassbox persist snapshot"));
    }
    lines
        .map(|l| {
            let mut f = l.splitn(3, '\t');
            match (f.next(), f.next(), f.next()) {
                (Some(h), Some(c), Some(p)) => Ok(Item { hash: h.into(), category: unescape(c), path: unescape(p) }),
                _ => Err(io::Error::new(io::ErrorKind::InvalidData, "bad snapshot line")),
            }
        })
        .collect()
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n").replace('\r', "\\r")
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}
