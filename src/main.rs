use glassbox::audit;
use glassbox::models::{self, Model};
use glassbox::persist::{self, Change};
use glassbox::reach::{self, Finding, Reach};
use glassbox::safe::visible;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: glassbox reach  [--json | --audit-json] [--strict] [--since FILE] [--home DIR]
       glassbox models [--json] [--home DIR] [--dir DIR]...
       glassbox persist [--save FILE | --since FILE [--strict]] [--json | --audit-json] [--home DIR] [--repo DIR]...
       glassbox watch [TRANSCRIPT] [--once] [--out DIR] [--loop N] [--spike TOKENS] [--failures N]
                      [--pid PID [--pause]] [--repo DIR]... [--home DIR]
       glassbox --version

watch   Follows a local agent session transcript (default: the newest Claude Code
        session) from its end. When an agent repeats an identical tool call --loop
        times in a row (4), produces more than --spike output tokens in a turn
        (16000), or fails --failures tool calls in a row (5), it writes a capture:
        the trigger, persistence added or changed since watching began, git changes
        in --repo, and the process tree under --pid. --pause stops --pid. --once
        scans the whole transcript and exits 1 if anything fired. Transcript content
        is never copied.
reach   Reports what a process started like this one (for example an agent
        launched from this terminal) could read. Opens each location
        read-only; never reads contents, never prints what a directory holds,
        never writes.
models  Lists local model weight files (Ollama, LM Studio, Hugging Face cache,
        --dir) and how many bytes of each are in the page cache. Measuring
        does not load the files.
persist Lists persistence points (launch agents, systemd units, cron, shell
        startup files, git hooks, ssh, Python startup, AI agent configs) with a
        SHA-256 of each. --save writes a snapshot; --since compares with one and
        reports what was added, changed or removed.

  --json     machine-readable output
  --audit-json
             reach, persist --since: one audit result object (format
             audit-result/1); exits as --strict does
  --strict   reach: exit 1 if any sensitive location is readable
             persist --since: exit 1 if anything was added or changed
  --home DIR use DIR instead of $HOME
  --dir DIR  models: also search DIR for .gguf and .safetensors files
  --repo DIR persist: also watch DIR's git hooks, git config and agent configs
  --save F   persist: write a snapshot to F
  --since F  persist: compare with the snapshot in F
             reach: compare with an earlier `reach --json` report in F";

struct Args {
    json: bool,
    audit: bool,
    strict: bool,
    home: Option<PathBuf>,
    dirs: Vec<PathBuf>,
    repos: Vec<PathBuf>,
    save: Option<PathBuf>,
    since: Option<PathBuf>,
    transcript: Option<PathBuf>,
    once_scan: bool,
    out: Option<PathBuf>,
    thresholds: glassbox::watch::Thresholds,
    pid: Option<i32>,
    pause: bool,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str);
    if command == Some("--version") && args.len() == 1 {
        println!("glassbox {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if !matches!(command, Some("reach" | "models" | "persist" | "watch")) {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    // Asked for before parsing, so that a usage error is still written as one object.
    let audit_asked = matches!(command, Some("reach" | "persist")) && args.iter().any(|x| x == "--audit-json");
    let a = match parse_args(command.unwrap_or_default(), &args[1..]) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}\n{USAGE}");
            return audit_error(audit_asked, "", msg);
        }
    };
    let Some(home) = a.home.clone().or_else(|| std::env::var_os("HOME").map(PathBuf::from)) else {
        eprintln!("HOME is not set; pass --home");
        return audit_error(a.audit, "", "HOME is not set; pass --home".into());
    };
    if command == Some("watch") {
        return run_watch(&home, &a);
    }
    if command == Some("persist") {
        return run_persist(&home, &a);
    }
    if command == Some("models") {
        return run_models(&home, &a);
    }
    let (json, strict) = (a.json, a.strict);
    let earlier = match a.since.as_deref().map(earlier_reach) {
        Some(None) => {
            let msg = format!("{} is not a `glassbox reach --json` report", a.since.clone().unwrap_or_default().display());
            eprintln!("{msg}");
            return audit_error(a.audit, &home.display().to_string(), msg);
        }
        Some(Some(map)) => Some(map),
        None => None,
    };
    let findings = reach::audit(&home);
    let chain = reach::parent_chain(std::process::id());
    if a.audit {
        let out: Vec<audit::Finding> = findings
            .iter()
            .map(|f| {
                let was = earlier.as_ref().and_then(|e| e.get(&esc(&display(f))));
                audit::reach_finding(f.location.category, f.location.sensitive, &display(f), &f.reach.as_str(),
                                     was.map(String::as_str), reach_record(f, earlier.as_ref()))
            })
            .collect();
        return print_audit(&home.display().to_string(), &out, &[]);
    }
    if json {
        print_json(&findings, &chain, earlier.as_ref());
    } else {
        print_text(&findings, &chain, earlier.as_ref());
    }
    let exposed = findings.iter().any(|f| f.location.sensitive && open(f.reach));
    if strict && exposed { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

/// Each option is accepted only by the commands it affects, at most once (except the
/// repeatable --dir and --repo), and an option never consumes another option as its value.
fn parse_args(command: &str, args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        json: false,
        audit: false,
        strict: false,
        home: None,
        dirs: Vec::new(),
        repos: Vec::new(),
        save: None,
        since: None,
        transcript: None,
        once_scan: false,
        out: None,
        thresholds: glassbox::watch::DEFAULTS,
        pid: None,
        pause: false,
    };
    let mut it = args.iter();
    let mut seen = std::collections::BTreeSet::new();
    while let Some(arg) = it.next() {
        let mut value = || match it.next() {
            Some(v) if !v.starts_with("--") => Ok(PathBuf::from(v)),
            _ => Err(format!("{arg} needs a value")),
        };
        let once = |slot: &Option<PathBuf>| if slot.is_some() { Err(format!("{arg} given twice")) } else { Ok(()) };
        match (arg.as_str(), command) {
            ("--json", _) => a.json = true,
            ("--audit-json", "reach" | "persist") => a.audit = true,
            ("--strict", "reach" | "persist") => a.strict = true,
            ("--home", _) => {
                once(&a.home)?;
                a.home = Some(value()?);
            }
            ("--dir", "models") => a.dirs.push(value()?),
            ("--repo", "persist" | "watch") => a.repos.push(value()?),
            ("--once", "watch") => a.once_scan = true,
            ("--pause", "watch") => a.pause = true,
            ("--out", "watch") => {
                once(&a.out)?;
                a.out = Some(value()?);
            }
            ("--loop" | "--spike" | "--failures" | "--pid", "watch") => {
                if !seen.insert(arg.clone()) {
                    return Err(format!("{arg} given twice"));
                }
                let n: u64 = value()?.to_str().and_then(|v| v.parse().ok()).filter(|n| *n > 0).ok_or(format!("{arg} needs a positive number"))?;
                match arg.as_str() {
                    "--loop" => a.thresholds.repeats = n,
                    "--spike" => a.thresholds.spike = n,
                    "--failures" => a.thresholds.failures = n,
                    _ => a.pid = Some(i32::try_from(n).map_err(|_| "--pid is out of range".to_string())?),
                }
            }
            (path, "watch") if !path.starts_with("--") && a.transcript.is_none() => a.transcript = Some(PathBuf::from(path)),
            ("--save", "persist") => {
                once(&a.save)?;
                a.save = Some(value()?);
            }
            ("--since", "persist" | "reach") => {
                once(&a.since)?;
                a.since = Some(value()?);
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    if a.save.is_some() && a.since.is_some() {
        return Err("use --save or --since, not both".into());
    }
    if a.pause && a.pid.is_none() {
        return Err("--pause needs --pid".into());
    }
    if a.pause && a.once_scan {
        return Err("--pause cannot be used with --once: a scan of past activity must not stop a live process".into());
    }
    if command == "persist" && a.strict && a.since.is_none() {
        return Err("--strict needs --since".into());
    }
    if a.audit && a.json {
        return Err("--json and --audit-json cannot be used together".into());
    }
    if command == "persist" && a.audit && a.since.is_none() {
        return Err("--audit-json needs --since".into());
    }
    Ok(a)
}

/// Path -> result of every finding in an earlier `reach --json` report.
fn earlier_reach(file: &Path) -> Option<Earlier> {
    use glassbox::json::Value;
    let (text, _) = glassbox::safe::read_capped(file, 4 << 20).ok()?;
    let Value::Obj(top) = glassbox::json::parse(std::str::from_utf8(&text).ok()?)? else { return None };
    let Value::Arr(items) = top.get("findings")? else { return None };
    let mut map = Earlier::new();
    for item in items {
        let Value::Obj(o) = item else { return None };
        let (Some(Value::Str(path)), Some(Value::Str(reach))) = (o.get("path"), o.get("reach")) else { return None };
        map.insert(path.clone(), reach.clone());
    }
    Some(map)
}

type Earlier = std::collections::BTreeMap<String, String>;

/// A result string that means something at the location can be read.
fn was_open(reach: &str) -> bool {
    matches!(reach, "readable" | "partly-readable")
}

/// Something at the location can be read: all of it, or part of a directory.
fn open(reach: Reach) -> bool {
    matches!(reach, Reach::Readable | Reach::Partial)
}

fn display(f: &Finding) -> String {
    format!("~/{}", f.location.rel)
}

fn print_text(findings: &[Finding], chain: &[(u32, String)], earlier: Option<&Earlier>) {
    if chain.is_empty() {
        println!("Process chain: not available. The process list could not be read, as happens inside a sandbox.");
    } else {
        println!("Process chain (nearest first):");
    }
    for (pid, comm) in chain {
        println!("  {pid:>7}  {}", visible(comm));
    }
    if let Some(app) = reach::responsible_app(chain) {
        println!("Privacy grants on macOS follow the responsible app: {}", visible(&app));
    }
    println!();
    for f in findings.iter().filter(|f| f.reach != Reach::Absent) {
        let flag = if f.location.sensitive && open(f.reach) { "!" } else { " " };
        let was = earlier.and_then(|e| e.get(&esc(&display(f)))).filter(|w| **w != f.reach.as_str());
        let was = was.map_or(String::new(), |w| format!("  (was {})", visible(w)));
        println!("{flag} {:<20} {:<17} {}{was}", f.reach.as_str(), f.location.category, display(f));
    }
    let readable = findings.iter().filter(|f| f.location.sensitive && open(f.reach)).count();
    let absent = findings.iter().filter(|f| f.reach == Reach::Absent).count();
    let count = |r: Reach| findings.iter().filter(|f| f.reach == r).count();
    let mut extra = String::new();
    if count(Reach::Partial) > 0 {
        extra += &format!("; {} partly readable", count(Reach::Partial));
    }
    if count(Reach::Listable) > 0 {
        extra += &format!("; {} listable only", count(Reach::Listable));
    }
    println!("\n{readable} sensitive location(s) readable (marked !){extra}; {absent} not present.");
    if let Some(e) = earlier {
        let before = |f: &&Finding| e.get(&esc(&display(f))).is_some_and(|w| was_open(w));
        let closed = findings.iter().filter(|f| before(f) && !open(f.reach)).count();
        let still = findings.iter().filter(|f| before(f) && open(f.reach)).count();
        let newly = findings.iter().filter(|f| !before(f) && open(f.reach)).count();
        println!("Against the earlier report: {closed} closed, {still} still readable, {newly} newly readable.");
    }
}

fn esc(s: &str) -> String {
    glassbox::safe::json_escape(s)
}

/// One `reach --json` finding, as printed.
fn reach_record(f: &Finding, earlier: Option<&Earlier>) -> String {
    let was = earlier.and_then(|e| e.get(&esc(&display(f))));
    let was = was.map_or(String::new(), |w| format!(",\"was\":\"{w}\""));
    format!(
        "{{\"category\":\"{}\",\"sensitive\":{},\"path\":\"{}\",\"reach\":\"{}\"{was}}}",
        esc(f.location.category), f.location.sensitive, esc(&display(f)), f.reach.as_str()
    )
}

/// Print the audit result object and return the exit status it states.
fn print_audit(home: &str, findings: &[audit::Finding], errors: &[String]) -> ExitCode {
    println!("{}", audit::render(home, findings, errors));
    ExitCode::from(audit::exit_code(findings, errors))
}

/// With --audit-json, the error as the object's one error (exit 2); else exit 2 alone.
fn audit_error(audit: bool, home: &str, msg: String) -> ExitCode {
    if audit {
        return print_audit(home, &[], &[msg]);
    }
    ExitCode::from(2)
}

fn print_json(findings: &[Finding], chain: &[(u32, String)], earlier: Option<&Earlier>) {
    let app = reach::responsible_app(chain).map_or("null".into(), |a| format!("\"{}\"", esc(&a)));
    let chain: Vec<String> = chain.iter().map(|(p, c)| format!("{{\"pid\":{p},\"command\":\"{}\"}}", esc(c))).collect();
    let items: Vec<String> = findings.iter().map(|f| reach_record(f, earlier)).collect();
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
    let w = rows.iter().map(|(m, _)| visible(&m.name).chars().count()).max().unwrap_or(5).max(5);
    println!("{:<w$}  {:<11}  {:>9}  {:>9}  {:>4}", "MODEL", "SOURCE", "SIZE", "IN MEMORY", "");
    for (m, r) in &rows {
        let pct = if m.bytes == 0 { 0.0 } else { 100.0 * *r as f64 / m.bytes as f64 };
        println!("{:<w$}  {:<11}  {:>9}  {:>9}  {:>3.0}%", visible(&m.name), m.source, size(m.bytes), size(*r), pct);
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
                println!("{:<18} {}", visible(&i.category), visible(&i.path));
            }
            println!("\n{} persistence point(s). Save a snapshot with --save FILE before a run, then compare with --since FILE.", now.len());
        }
        return ExitCode::SUCCESS;
    };
    let before = match persist::load(file) {
        Ok(items) => items,
        Err(e) => {
            let msg = format!("cannot read snapshot {}: {e}", file.display());
            eprintln!("{msg}");
            return audit_error(a.audit, &home.display().to_string(), msg);
        }
    };
    let changes = persist::diff(&before, &now);
    if a.audit {
        let out: Vec<audit::Finding> = changes
            .iter()
            .map(|(c, i)| audit::persist_finding(c.as_str(), &i.category, &i.path, persist_record(c, i)))
            .collect();
        return print_audit(&home.display().to_string(), &out, &[]);
    }
    if a.json {
        let items: Vec<String> = changes.iter().map(|(c, i)| persist_record(c, i)).collect();
        println!("{{\"changes\":[{}]}}", items.join(","));
    } else if changes.is_empty() {
        println!("No persistence points added, changed or removed since {}.", file.display());
    } else {
        for (c, i) in &changes {
            let mark = match c { Change::Added => "+", Change::Changed => "~", Change::Removed => "-" };
            println!("{mark} {:<8} {:<18} {}", c.as_str(), visible(&i.category), visible(&i.path));
        }
    }
    let flagged = changes.iter().any(|(c, _)| *c != Change::Removed);
    if a.strict && flagged { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

/// One `persist --since --json` change, as printed.
fn persist_record(c: &Change, i: &persist::Item) -> String {
    format!("{{\"change\":\"{}\",\"category\":\"{}\",\"path\":\"{}\"}}", c.as_str(), esc(&i.category), esc(&i.path))
}

fn run_watch(home: &Path, a: &Args) -> ExitCode {
    use glassbox::watch::{self, Context, Detector, Follower};
    use std::io::Write;
    let Some(path) = a.transcript.clone().or_else(|| watch::latest_transcript(home)) else {
        eprintln!("no transcript given and none found under {}/.claude/projects", home.display());
        return ExitCode::from(2);
    };
    match watch::recognised_format(&path) {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("{}: not a transcript format glassbox knows (Claude Code JSON lines)", path.display());
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("cannot read {}: {e}", path.display());
            return ExitCode::from(2);
        }
    }
    let t = &a.thresholds;
    let lines_before = if a.once_scan { 0 } else { watch::line_count(&path).unwrap_or(0) };
    let mut detector = Detector::new(*t, lines_before);
    let mut seq = 0u64;
    let mut fired_any = false;
    let mut handle = |trigger: watch::Trigger, ctx: &Context, out: Option<&Path>| {
        fired_any = true;
        seq += 1;
        let mut line = format!("trigger {}", trigger.describe());
        if let Some(out) = out {
            match watch::capture(out, seq, &trigger, ctx) {
                Ok(dir) => line.push_str(&format!("; capture: {}", dir.display())),
                Err(e) => line.push_str(&format!("; capture failed: {e}")),
            }
        }
        if let (true, Some(pid)) = (a.pause, a.pid) {
            match watch::pause(pid) {
                Ok(()) => line.push_str(&format!("; paused {pid}, resume with: kill -CONT {pid}")),
                Err(e) => line.push_str(&format!("; could not pause {pid}: {e}")),
            }
        }
        println!("{line}");
        let _ = std::io::stdout().flush();
    };

    if a.once_scan {
        let ctx = Context { transcript: &path, home, repos: &a.repos, pid: a.pid, baseline: None };
        let scanned = watch::for_each_line(&path, |line| {
            for trigger in detector.feed(line) {
                handle(trigger, &ctx, a.out.as_deref());
            }
        });
        if let Err(e) = scanned {
            eprintln!("cannot read {}: {e}", path.display());
            return ExitCode::from(2);
        }
        return if fired_any { ExitCode::from(1) } else { ExitCode::SUCCESS };
    }

    let out = a.out.clone().unwrap_or_else(|| home.join(".local/state/glassbox/captures"));
    let baseline = glassbox::persist::collect(home, &a.repos);
    let mut follower = match Follower::at_end(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("cannot follow {}: {e}", path.display());
            return ExitCode::from(2);
        }
    };
    println!(
        "glassbox watch: watching {} (loop {}, spike {}, failures {}); captures in {}",
        visible(&path.display().to_string()),
        t.repeats,
        t.spike,
        t.failures,
        visible(&out.display().to_string())
    );
    let _ = std::io::stdout().flush();
    let ctx = Context { transcript: &path, home, repos: &a.repos, pid: a.pid, baseline: Some(&baseline) };
    loop {
        match follower.new_lines() {
            Ok(lines) => {
                for line in lines {
                    for trigger in detector.feed(&line) {
                        handle(trigger, &ctx, Some(&out));
                    }
                }
            }
            Err(e) => {
                eprintln!("stopped following {}: {e}", path.display());
                return ExitCode::from(2);
            }
        }
        if a.pid.is_some_and(|pid| !watch::alive(pid)) {
            println!("glassbox watch: process {} has exited", a.pid.unwrap_or_default());
            return ExitCode::SUCCESS;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}
