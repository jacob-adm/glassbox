//! Hostile-input tests. glassbox audits files that the agents it watches can write,
//! so every reader of those files must return a result, never crash.
use glassbox::{json, models, persist};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Deterministic xorshift64* so failures reproduce from the seed.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn bytes(&mut self, max: usize) -> Vec<u8> {
        (0..self.below(max + 1)).map(|_| self.next() as u8).collect()
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glassbox-fuzz-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

const SAMPLE: &str = r#"{"numStartups":3,"projects":{"/w":{"mcpServers":{"x":{"command":"/bin/x","args":["-v","é"]}},"hooks":[1,2.5e3,true,null]}}}"#;

#[test]
fn deep_nesting_is_rejected_not_a_crash() {
    for open in ["[", "{\"a\":"] {
        let text = open.repeat(200_000);
        assert_eq!(json::parse(&text), None);
    }
}

#[test]
fn persist_survives_a_hostile_claude_json() {
    let home = scratch("deep");
    fs::write(home.join(".claude.json"), "[".repeat(200_000) + &"]".repeat(200_000)).unwrap();
    let snap = home.with_extension("snapshot");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_glassbox")).arg("persist").args(args).arg("--home").arg(&home).output().unwrap()
    };
    assert_eq!(run(&["--save", snap.to_str().unwrap()]).status.code(), Some(0));
    // A subtree past the depth limit could hide a hook, so changing it is reported, without crashing.
    fs::write(home.join(".claude.json"), "[".repeat(200_001) + &"]".repeat(200_001)).unwrap();
    let out = run(&["--since", snap.to_str().unwrap(), "--strict", "--json"]);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    // Still reported on a second run; restoring the original content is not a change.
    let out = run(&["--since", snap.to_str().unwrap(), "--strict", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    fs::write(home.join(".claude.json"), "[".repeat(200_000) + &"]".repeat(200_000)).unwrap();
    let out = run(&["--since", snap.to_str().unwrap(), "--strict", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stdout));
    // A hook added next to the deep subtree is caught.
    fs::write(home.join(".claude.json"), format!(r#"{{"x":{},"hooks":{{"Stop":"curl example.invalid"}}}}"#, "[".repeat(200_000) + &"]".repeat(200_000))).unwrap();
    let out = run(&["--since", snap.to_str().unwrap(), "--strict", "--json"]);
    let json = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{json}");
    assert!(json.contains("~/.claude.json#mcpServers,hooks"), "{json}");
}

#[test]
fn json_reader_survives_random_and_mutated_input() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    assert!(json::parse(SAMPLE).is_some(), "sample must parse");
    for _ in 0..20_000 {
        let random = String::from_utf8_lossy(&rng.bytes(256)).into_owned();
        let _ = json::parse(&random);
        let mut mutated = SAMPLE.as_bytes().to_vec();
        for _ in 0..1 + rng.below(4) {
            let i = rng.below(mutated.len());
            match rng.below(3) {
                0 => mutated[i] = rng.next() as u8,
                1 => {
                    mutated.remove(i);
                }
                _ => mutated.insert(i, b"{}[]\":,\\u0"[rng.below(10)]),
            }
        }
        if let Some(v) = json::parse(&String::from_utf8_lossy(&mutated)) {
            let _ = v.canonical();
            let _ = v.find(&["mcpServers", "hooks"]);
        }
    }
}

#[test]
fn snapshot_loader_survives_random_files() {
    let dir = scratch("snapshots");
    let mut rng = Rng(42);
    for i in 0..500 {
        let file = dir.join(format!("{i}.snapshot"));
        let mut body = if rng.below(2) == 0 { b"# glassbox persist v1\n".to_vec() } else { Vec::new() };
        body.extend(rng.bytes(300));
        fs::write(&file, &body).unwrap();
        let _ = persist::load(&file);
    }
}

#[test]
fn ollama_discovery_survives_random_manifests() {
    let home = scratch("ollama");
    let manifests = home.join(".ollama/models/manifests/registry.ollama.ai/library/m");
    fs::create_dir_all(&manifests).unwrap();
    fs::create_dir_all(home.join(".ollama/models/blobs")).unwrap();
    let mut rng = Rng(7);
    for i in 0..300 {
        let mut body = br#"{"layers":[{"mediaType":"application/vnd.ollama.image.model","digest":""#.to_vec();
        body.extend(rng.bytes(120));
        fs::write(manifests.join(format!("t{i}")), &body).unwrap();
    }
    let found = models::discover(&home, &[]);
    for m in &found {
        let _ = m.resident();
    }
}

#[test]
fn unknown_and_missing_arguments_are_usage_errors() {
    let cases: [&[&str]; 8] = [
        &[],
        &["nope"],
        &["reach", "--bogus"],
        &["models", "--strict"],
        &["persist", "--save"],
        &["persist", "--since"],
        &["models", "--dir"],
        &["reach", "--home"],
    ];
    for args in cases {
        let out = Command::new(env!("CARGO_BIN_EXE_glassbox")).args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?} must be a usage error");
    }
}
