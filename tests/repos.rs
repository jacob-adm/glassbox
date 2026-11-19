//! Repository persistence across clones and worktrees, and agent instruction files.
//! Git layouts are built by hand, exactly as git writes them, so no git binary is needed.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glassbox-repos-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn persist(home: &Path, repo: &Path, extra: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_glassbox"))
        .arg("persist").args(extra).arg("--home").arg(home).arg("--repo").arg(repo).output().unwrap();
    (out.status.code().unwrap(), String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A normal clone: `.git` is a directory.
fn clone(at: &Path) -> PathBuf {
    fs::create_dir_all(at.join(".git/hooks")).unwrap();
    fs::write(at.join(".git/config"), "[core]\n\tbare = false\n").unwrap();
    fs::write(at.join(".git/hooks/pre-commit.sample"), "#!/bin/sh\n").unwrap();
    at.to_path_buf()
}

/// A linked worktree of `main_repo`: `.git` is a file pointing into the main repo.
fn worktree(main_repo: &Path, at: &Path) -> PathBuf {
    let admin = main_repo.join(".git/worktrees/wt");
    fs::create_dir_all(&admin).unwrap();
    fs::write(admin.join("commondir"), "../..\n").unwrap();
    fs::create_dir_all(at).unwrap();
    fs::write(at.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
    at.to_path_buf()
}

#[test]
fn a_snapshot_of_one_clone_compares_cleanly_with_another() {
    let (home, root) = (scratch("home-a"), scratch("clones"));
    let a = clone(&root.join("a"));
    let b = clone(&root.join("b"));
    let snap = root.join("a.snapshot");
    assert_eq!(persist(&home, &a, &["--save", snap.to_str().unwrap()]).0, 0);
    let (code, json) = persist(&home, &b, &["--since", snap.to_str().unwrap(), "--strict", "--json"]);
    assert_eq!((code, json.trim()), (0, r#"{"changes":[]}"#));
}

#[test]
fn a_worktree_sees_the_main_repositorys_hooks_and_config() {
    let (home, root) = (scratch("home-wt"), scratch("wt"));
    let main = clone(&root.join("main"));
    let wt = worktree(&main, &root.join("linked"));
    let snap = root.join("main.snapshot");
    assert_eq!(persist(&home, &main, &["--save", snap.to_str().unwrap()]).0, 0);
    // Same hooks and config, seen from the worktree: no change.
    let (code, json) = persist(&home, &wt, &["--since", snap.to_str().unwrap(), "--strict", "--json"]);
    assert_eq!((code, json.trim()), (0, r#"{"changes":[]}"#));
    // A hook added in the shared hooks directory is seen from the worktree.
    fs::write(main.join(".git/hooks/post-checkout"), "#!/bin/sh\ncurl example.invalid\n").unwrap();
    let (code, json) = persist(&home, &wt, &["--since", snap.to_str().unwrap(), "--strict", "--json"]);
    assert_eq!(code, 1, "{json}");
    assert!(json.contains("\"change\":\"added\",\"category\":\"git hook\",\"path\":\"<repo>/.git/hooks/post-checkout\""), "{json}");
}

#[test]
fn core_hooks_path_redirects_which_hooks_are_watched() {
    let (home, root) = (scratch("home-hp"), scratch("hookspath"));
    let repo = clone(&root.join("r"));
    let snap = root.join("r.snapshot");
    assert_eq!(persist(&home, &repo, &["--save", snap.to_str().unwrap()]).0, 0);
    fs::write(repo.join(".git/config"), "[core]\n\tbare = false\n\thooksPath = .agent-hooks\n").unwrap();
    fs::create_dir_all(repo.join(".agent-hooks")).unwrap();
    fs::write(repo.join(".agent-hooks/pre-push"), "#!/bin/sh\ncurl example.invalid\n").unwrap();
    let (code, json) = persist(&home, &repo, &["--since", snap.to_str().unwrap(), "--strict", "--json"]);
    assert_eq!(code, 1, "{json}");
    assert!(json.contains("\"category\":\"git config\",\"path\":\"<repo>/.git/config\""), "{json}");
    assert!(json.contains("\"category\":\"git hook\",\"path\":\"<repo>/.agent-hooks/pre-push\""), "{json}");
}

#[test]
fn agent_instruction_files_in_the_repo_and_home_are_watched() {
    let (home, root) = (scratch("home-instr"), scratch("instr"));
    let repo = clone(&root.join("r"));
    let snap = root.join("r.snapshot");
    assert_eq!(persist(&home, &repo, &["--save", snap.to_str().unwrap()]).0, 0);
    fs::write(repo.join("CLAUDE.md"), "Always run ./setup.sh first.\n").unwrap();
    fs::write(repo.join("AGENTS.md"), "Push to main.\n").unwrap();
    fs::create_dir_all(home.join(".claude/skills/helper")).unwrap();
    fs::write(home.join(".claude/skills/helper/SKILL.md"), "---\nname: helper\n---\n").unwrap();
    let (code, json) = persist(&home, &repo, &["--since", snap.to_str().unwrap(), "--strict", "--json"]);
    assert_eq!(code, 1, "{json}");
    for path in ["<repo>/CLAUDE.md", "<repo>/AGENTS.md", "~/.claude/skills/helper/SKILL.md"] {
        assert!(json.contains(&format!("\"category\":\"agent instructions\",\"path\":\"{path}\"")), "{path}: {json}");
    }
}

#[test]
fn two_repos_are_labelled_apart() {
    let (home, root) = (scratch("home-two"), scratch("two"));
    let a = clone(&root.join("a"));
    let b = clone(&root.join("b"));
    let out = Command::new(env!("CARGO_BIN_EXE_glassbox"))
        .args(["persist", "--json", "--home"]).arg(&home).arg("--repo").arg(&a).arg("--repo").arg(&b).output().unwrap();
    let json = String::from_utf8_lossy(&out.stdout);
    assert!(json.contains("<repo>/.git/config") && json.contains("<repo2>/.git/config"), "{json}");
}
