use glassbox::models::{self, Model};
use glassbox::persist::{self, Change};
use glassbox::reach::{self, Finding, Reach};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: glassbox reach  [--json] [--strict] [--home DIR]
       glassbox models [--json] [--home DIR] [--dir DIR]...
       glassbox persist [--save FILE | --since FILE [--strict]] [--json] [--home DIR] [--repo DIR]...

reach   Reports what a process started like this one (for example an agent
        launched from this terminal) could read. Opens each location
        read-only; never reads contents, never lists directories, never writes.
models  Lists local model weight files (Ollama, LM Studio, Hugging Face cache,
        --dir) and how many bytes of each are in the page cache. Measuring
        does not load the files.
persist Lists persistence points (launch agents, systemd units, cron, shell
        startup files, git hooks, ssh, Python startup, AI agent configs) with a
        SHA-256 of each. --save writes a snapshot; --since compares with one and
        reports what was added, changed or removed.

  --json     machine-readable output
  --strict   reach: exit 1 if any sensitive location is readable
             persist --since: exit 1 if anything was added or changed
  --home DIR use DIR instead of $HOME
  --dir DIR  models: also search DIR for .gguf and .safetensors files
  --repo DIR persist: also watch DIR's git hooks, git config and agent configs
  --save F   persist: write a snapshot to F
  --since F  persist: compare with the snapshot in F";

struct Args {
    json: bool,
    strict: bool,
    home: Option<PathBuf>,
    dirs: Vec<PathBuf>,
    repos: Vec<PathBuf>,
    save: Option<PathBuf>,
    since: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str);
    if !matches!(command, Some("reach" | "models" | "persist")) {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let mut a = Args { json: false, strict: false, home: std::env::var_os("HOME").map(PathBuf::from), dirs: Vec::new(), repos: Vec::new(), save: None, since: None };
    let mut it = args.iter().skip(1);
    while let Some(arg) = it.next() {
        match (arg.as_str(), command) {
            ("--json", _) => a.json = true,
            ("--strict", Some("reach" | "persist")) => a.strict = true,
            ("--home", _) | ("--dir", Some("models")) | ("--repo", Some("persist")) | ("--save" | "--since", Some("persist")) => {
                let Some(value) = it.next().map(PathBuf::from) else {
                    eprintln!("{arg} needs a value\n{USAGE}");
                    return ExitCode::from(2);
                };
                match arg.as_str() {
                    "--home" => a.home = Some(value),
                    "--dir" => a.dirs.push(value),
                    "--repo" => a.repos.push(value),
                    "--save" => a.save = Some(value),
                    _ => a.since = Some(value),
                }
            }
            _ => {
                eprintln!("unknown argument: {arg}\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(home) = a.home.clone() else {
        eprintln!("HOME is not set; pass --home");
        return ExitCode::from(2);
    };
    if command == Some("persist") {
        return run_persist(&home, &a);
    }
    if command == Some("models") {
        return run_models(&home, &a);
    }
    let (json, strict) = (a.json, a.strict);
    let findings = reach::audit(&home);
    let chain = reach::parent_chain(std::process::id());
    if json {
        print_json(&findings, &chain);
    } else {
        print_text(&findings, &chain);
    }
    let exposed = findings.iter().any(|f| f.location.sensitive && f.reach == Reach::Readable);
    if strict && exposed { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

fn display(f: &Finding) -> String {
    format!("~/{}", f.location.rel)
}

fn print_text(findings: &[Finding], chain: &[(u32, String)]) {
    println!("Process chain (nearest first):");
    for (pid, comm) in chain {
        println!("  {pid:>7}  {comm}");
    }
    if let Some(app) = reach::responsible_app(chain) {
        println!("Privacy grants on macOS follow the responsible app: {app}");
    }
    println!();
    for f in findings.iter().filter(|f| f.reach != Reach::Absent) {
        let flag = if f.location.sensitive && f.reach == Reach::Readable { "!" } else { " " };
        println!("{flag} {:<20} {:<17} {}", f.reach.as_str(), f.location.category, display(f));
    }
    let readable = findings.iter().filter(|f| f.location.sensitive && f.reach == Reach::Readable).count();
    let absent = findings.iter().filter(|f| f.reach == Reach::Absent).count();
    println!("\n{readable} sensitive location(s) readable (marked !); {absent} not present.");
}

fn esc(s: &str) -> String {
    s.chars().flat_map(|c| match c {
        '"' => "\\\"".chars().collect::<Vec<_>>(),
        '\\' => "\\\\".chars().collect(),
        c if (c as u32) < 0x20 => format!("\\u{:04x}", c as u32).chars().collect(),
        c => vec![c],
    }).collect()
}

fn print_json(findings: &[Finding], chain: &[(u32, String)]) {
    let app = reach::responsible_app(chain).map_or("null".into(), |a| format!("\"{}\"", esc(&a)));
    let chain: Vec<String> = chain.iter().map(|(p, c)| format!("{{\"pid\":{p},\"command\":\"{}\"}}", esc(c))).collect();
    let items: Vec<String> = findings
        .iter()
        .map(|f| {
            format!(
                "{{\"category\":\"{}\",\"sensitive\":{},\"path\":\"{}\",\"reach\":\"{}\"}}",
                esc(f.location.category), f.location.sensitive, esc(&display(f)), f.reach.as_str()
            )
        })
        .collect();
    println!("{{\"chain\":[{}],\"responsible_app\":{app},\"findings\":[{}]}}", chain.join(","), items.join(","));
}

fn run_models(home: &Path, a: &Args) -> ExitCode {
    let found = models::discover(home, &a.dirs);
    let mut rows: Vec<(&Model, u64)> = found.iter().map(|m| (m, m.resident())).collect();
    rows.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.name.cmp(&y.0.name)));
    if a.json {
        let items: Vec<String> = rows
            .iter()
            .map(|(m, r)| {
                let files: Vec<String> = m.files.iter().map(|f| format!("\"{}\"", esc(&f.to_string_lossy()))).collect();
                format!(
                    "{{\"name\":\"{}\",\"source\":\"{}\",\"bytes\":{},\"resident\":{r},\"files\":[{}]}}",
                    esc(&m.name), m.source, m.bytes, files.join(",")
                )
            })
            .collect();
        println!("{{\"models\":[{}]}}", items.join(","));
        return ExitCode::SUCCESS;
    }
    if rows.is_empty() {
        println!("No model files found. Use --dir to add a directory of .gguf or .safetensors files.");
        return ExitCode::SUCCESS;
    }
    let w = rows.iter().map(|(m, _)| m.name.chars().count()).max().unwrap_or(5).max(5);
    println!("{:<w$}  {:<11}  {:>9}  {:>9}  {:>4}", "MODEL", "SOURCE", "SIZE", "IN MEMORY", "");
    for (m, r) in &rows {
        let pct = if m.bytes == 0 { 0.0 } else { 100.0 * *r as f64 / m.bytes as f64 };
        println!("{:<w$}  {:<11}  {:>9}  {:>9}  {:>3.0}%", m.name, m.source, size(m.bytes), size(*r), pct);
    }
    let (total, warm): (u64, u64) = rows.iter().fold((0, 0), |(t, w), (m, r)| (t + m.bytes, w + r));
    println!("\n{} model(s), {} on disk, {} in the page cache.", rows.len(), size(total), size(warm));
    println!("In the page cache means a load reads from memory, not disk. A model already loaded");
    println!("by a running server may also hold its weights in other memory, which is not counted here.");
    ExitCode::SUCCESS
}

fn size(bytes: u64) -> String {
    const GIB: f64 = (1u64 << 30) as f64;
    const MIB: f64 = (1u64 << 20) as f64;
    let b = bytes as f64;
    if b >= GIB { format!("{:.1} GiB", b / GIB) } else { format!("{:.0} MiB", b / MIB) }
}

fn run_persist(home: &Path, a: &Args) -> ExitCode {
    if a.save.is_some() && a.since.is_some() {
        eprintln!("use --save or --since, not both\n{USAGE}");
        return ExitCode::from(2);
    }
    let now = persist::collect(home, &a.repos);
    if let Some(file) = &a.save {
        if let Err(e) = persist::save(&now, file) {
            eprintln!("cannot write {}: {e}", file.display());
            return ExitCode::from(2);
        }
        println!("Saved {} persistence point(s) to {}.", now.len(), file.display());
        return ExitCode::SUCCESS;
    }
    let Some(file) = &a.since else {
        if a.json {
            let items: Vec<String> = now
                .iter()
                .map(|i| format!("{{\"category\":\"{}\",\"path\":\"{}\",\"sha256\":\"{}\"}}", esc(&i.category), esc(&i.path), esc(&i.hash)))
                .collect();
            println!("{{\"items\":[{}]}}", items.join(","));
        } else {
            for i in &now {
                println!("{:<15} {}", i.category, i.path);
            }
            println!("\n{} persistence point(s). Save a snapshot with --save FILE before a run, then compare with --since FILE.", now.len());
        }
        return ExitCode::SUCCESS;
    };
    let before = match persist::load(file) {
        Ok(items) => items,
        Err(e) => {
            eprintln!("cannot read snapshot {}: {e}", file.display());
            return ExitCode::from(2);
        }
    };
    let changes = persist::diff(&before, &now);
    if a.json {
        let items: Vec<String> = changes
            .iter()
            .map(|(c, i)| format!("{{\"change\":\"{}\",\"category\":\"{}\",\"path\":\"{}\"}}", c.as_str(), esc(&i.category), esc(&i.path)))
            .collect();
        println!("{{\"changes\":[{}]}}", items.join(","));
    } else if changes.is_empty() {
        println!("No persistence points added, changed or removed since {}.", file.display());
    } else {
        for (c, i) in &changes {
            let mark = match c { Change::Added => "+", Change::Changed => "~", Change::Removed => "-" };
            println!("{mark} {:<8} {:<15} {}", c.as_str(), i.category, i.path);
        }
    }
    let flagged = changes.iter().any(|(c, _)| *c != Change::Removed);
    if a.strict && flagged { ExitCode::from(1) } else { ExitCode::SUCCESS }
}
