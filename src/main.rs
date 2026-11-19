use glassbox::reach::{self, Finding, Reach};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: glassbox reach [--json] [--strict] [--home DIR]

Reports what a process started like this one (for example an agent launched
from this terminal) could read. Opens each location read-only; never reads
contents, never lists directories, never writes.

  --json     machine-readable output
  --strict   exit 1 if any sensitive location is readable
  --home DIR audit DIR instead of $HOME";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("reach") {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let (mut json, mut strict, mut home) = (false, false, std::env::var_os("HOME").map(PathBuf::from));
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => json = true,
            "--strict" => strict = true,
            "--home" => home = it.next().map(PathBuf::from),
            _ => {
                eprintln!("unknown argument: {a}\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(home) = home else {
        eprintln!("HOME is not set; pass --home");
        return ExitCode::from(2);
    };
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
