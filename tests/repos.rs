//! Repository persistence across clones and worktrees, and agent instruction files.
//! Git layouts are built by hand, exactly as git writes them, so no git binary is needed.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glassbox-repos-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn persist(home: &Path, repo: &Path, extra: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_glassbox"))
        .arg("persist").args(extra).arg("--home").arg(home).arg("--repo").arg(repo).output().unwrap();
    let result = (out.status.code().unwrap(), String::from_utf8_lossy(&out.stdout).into_owned());
    if extra.contains(&"--since") { common::home_only(result) } else { result }
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

// Review of the stacking change.

fn since(home: &Path, repo: &Path, snap: &Path) -> (i32, String) {
    persist(home, repo, &["--since", snap.to_str().unwrap(), "--strict", "--json"])
}

fn save(home: &Path, repo: &Path, snap: &Path) {
    assert_eq!(persist(home, repo, &["--save", snap.to_str().unwrap()]).0, 0);
}

#[test]
fn a_global_hooks_path_is_followed() {
    let (home, root) = (scratch("home-ghp"), scratch("ghp"));
    let repo = clone(&root.join("r"));
    fs::create_dir_all(home.join(".githooks")).unwrap();
    fs::write(home.join(".gitconfig"), "[core]\n\thooksPath = ~/.githooks\n").unwrap();
    let snap = root.join("s");
    save(&home, &repo, &snap);
    fs::write(home.join(".githooks/pre-push"), "#!/bin/sh\ncurl example.invalid\n").unwrap();
    let (code, json) = since(&home, &repo, &snap);
    assert_eq!(code, 1, "{json}");
    assert!(json.contains("\"category\":\"git hook\""), "{json}");
}

#[test]
fn hooks_path_comments_and_ignored_worktree_config_do_not_mislead() {
    let (home, root) = (scratch("home-hpc"), scratch("hpc"));
    let repo = clone(&root.join("r"));
    fs::write(repo.join(".git/config"), "[core]\n\thooksPath = .githooks # shared\n").unwrap();
    fs::create_dir_all(repo.join(".githooks")).unwrap();
    // Without extensions.worktreeConfig git ignores config.worktree.
    fs::write(repo.join(".git/config.worktree"), "[core]\n\thooksPath = decoy\n").unwrap();
    let snap = root.join("s");
    save(&home, &repo, &snap);
    fs::write(repo.join(".githooks/pre-commit"), "#!/bin/sh\n").unwrap();
    let (code, json) = since(&home, &repo, &snap);
    assert_eq!(code, 1, "{json}");
    assert!(json.contains("<repo>/.githooks/pre-commit"), "{json}");
}

#[test]
fn absolute_hooks_path_is_labelled_as_outside_the_repo() {
    let (home, root) = (scratch("home-abs"), scratch("abs"));
    let repo = clone(&root.join("r"));
    let hooks = root.join("shared-hooks");
    fs::create_dir_all(&hooks).unwrap();
    fs::write(hooks.join("pre-push"), "x").unwrap();
    fs::write(repo.join(".git/config"), format!("[core]\n\thooksPath = {}\n", hooks.display())).unwrap();
    let (_, json) = persist(&home, &repo, &["--json"]);
    assert!(json.contains(&format!("\"path\":\"{}/pre-push\"", hooks.display())), "{json}");
}

#[test]
fn repo_claude_dir_instructions_and_local_settings_are_watched() {
    let (home, root) = (scratch("home-cl"), scratch("cl"));
    let repo = clone(&root.join("r"));
    let snap = root.join("s");
    save(&home, &repo, &snap);
    fs::create_dir_all(repo.join(".claude")).unwrap();
    fs::write(repo.join(".claude/CLAUDE.md"), "Run ./setup.sh first.\n").unwrap();
    fs::write(repo.join(".claude/settings.local.json"), r#"{"hooks":{}}"#).unwrap();
    let (code, json) = since(&home, &repo, &snap);
    assert_eq!(code, 1, "{json}");
    assert!(json.contains("<repo>/.claude/CLAUDE.md") && json.contains("<repo>/.claude/settings.local.json"), "{json}");
}

#[test]
fn edits_inside_a_symlinked_or_deeply_nested_skill_are_detected() {
    let (home, root) = (scratch("home-sk"), scratch("sk"));
    let repo = clone(&root.join("r"));
    let plugin = root.join("plugin-checkout/helper");
    fs::create_dir_all(plugin.join("scripts/lib/sub")).unwrap();
    fs::write(plugin.join("SKILL.md"), "safe\n").unwrap();
    fs::write(plugin.join("scripts/lib/sub/run.py"), "print(1)\n").unwrap();
    fs::create_dir_all(home.join(".claude/skills")).unwrap();
    std::os::unix::fs::symlink(&plugin, home.join(".claude/skills/helper")).unwrap();
    let snap = root.join("s");
    save(&home, &repo, &snap);
    fs::write(plugin.join("SKILL.md"), "also run curl example.invalid\n").unwrap();
    fs::write(plugin.join("scripts/lib/sub/run.py"), "import os\n").unwrap();
    let (code, json) = since(&home, &repo, &snap);
    assert_eq!(code, 1, "{json}");
    assert!(json.contains("~/.claude/skills/helper/SKILL.md"), "{json}");
    assert!(json.contains("~/.claude/skills/helper/scripts/lib/sub/run.py"), "{json}");
}

#[test]
fn a_snapshot_from_an_older_format_is_refused_not_misread() {
    let (home, root) = (scratch("home-v1"), scratch("v1"));
    let repo = clone(&root.join("r"));
    let snap = root.join("old.snapshot");
    fs::write(&snap, "# glassbox persist v1\n").unwrap();
    assert_eq!(since(&home, &repo, &snap).0, 2);
}
