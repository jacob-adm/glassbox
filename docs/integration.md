# Using glassbox with other tools

Each section below comes from a real run on 2026-11-19 (macOS 26, Apple silicon), with the commands
and output as they appeared, scratch paths shortened. Each ends with a verdict.

## git-overlap: parallel agent branches

[git-overlap](https://github.com/jacob-adm/git-overlap) answers "which agent branches will conflict?".
glassbox answers "what will each branch set up to run?". They need nothing from each other: run
git-overlap on the branches, and `glassbox persist --repo` on a worktree of each.

Three agent branches off `main`: `agent/a` edits `src/main.rs`; `agent/b` edits the same file and
commits an MCP server in `.mcp.json`; `agent/c` adds a `CLAUDE.md` instruction and a Claude hook.

```
$ git for-each-ref --format='%(refname:short)' refs/heads/agent/ | git footprint main | git overlap | git pairmerge
agent/a	agent/b	conflict	src/main.rs
(exit 1)

$ glassbox persist --repo repo --save main.snapshot
$ git worktree add wt-a agent/a    # and wt-b, wt-c
$ glassbox persist --repo wt-a --since main.snapshot --strict
No persistence points added, changed or removed since main.snapshot.
(exit 0)
$ glassbox persist --repo wt-b --since main.snapshot --strict
+ added    agent config       <repo>/.mcp.json
(exit 1)
$ glassbox persist --repo wt-c --since main.snapshot --strict
+ added    agent config       <repo>/.claude/settings.json
+ added    agent instructions <repo>/CLAUDE.md
(exit 1)
```

**Verdict: works together.** Between them, the two tools show which branches conflict and what each
one installs. Running them together the first time found two glassbox defects, both fixed before this
run: `--repo` labels were absolute paths, so a snapshot of one checkout could not be compared with a
worktree, and inside a worktree (where `.git` is a file) no hooks were found.

## pr-merge-audit: merge-testing pull requests

[pr-merge-audit](https://github.com/jacob-adm/pr-merge-audit) runs the tests of every open pull request
as you, and its security notes say to run builds on a disposable machine that holds no credentials.
glassbox checks that condition before the build and checks afterwards that the tests left nothing
behind.

```
$ glassbox reach --strict                        # on a workstation
(exit 1)                                         # credentials readable: not disposable
$ glassbox reach --strict                        # in a container: --network none, unprivileged user
0 sensitive location(s) readable (marked !); 23 not present.
(exit 0)

$ glassbox persist --save before-build.snapshot
$ pr-merge-audit --config config.json build --snapshot snapshots/2026-11-19.json --out results/2026-11-19.json
merging 1 PRs in order
1 merged, 0 held; running follow-ups
$ pr-merge-audit report --results results/2026-11-19.json --out reports/2026-11-19.md
### Result: 1 of 1 PRs merged together with every test passing
$ glassbox persist --since before-build.snapshot --strict
No persistence points added, changed or removed since before-build.snapshot.
(exit 0)
```

A wrapper that refuses to build on a machine with readable credentials:

```sh
glassbox reach --strict --json > reach.json || { echo "credentials readable: use a disposable machine"; exit 1; }
glassbox persist --save before-build.snapshot
pr-merge-audit --config config.json build --snapshot "$SNAPSHOT"
glassbox persist --since before-build.snapshot --strict
```

**Verdict: works together.** Two points of friction in pr-merge-audit, both by design but worth knowing:

- Its `snapshot` step runs git without credentials or global configuration, so it cannot fetch a
  private GitHub repository. Its documented route for other hosts works: write
  `snapshots/DATE.json` and a `git bundle` with `refs/audit/base` and `refs/audit/pr/N` by hand, then
  run `build` offline.
- Tests run with a private `HOME`. With a rustup-managed toolchain, `cargo` then finds no toolchain and
  the base fails. Pass the toolchain location in `test_cmd`:
  `["/usr/bin/env", "RUSTUP_HOME=/Users/me/.rustup", "CARGO_HOME=/Users/me/.cargo", "/Users/me/.cargo/bin/cargo", "test", "--locked"]`.

## An agent tool installer: what it adds to a repository

An agent tool installer that enrols a repository (it writes git hooks, agent instruction files and
a git config key) was run on an empty repository between a snapshot and a comparison:

```
$ glassbox persist --repo demo --save before.snapshot
$ <installer> inject demo --preset minimal
$ glassbox persist --repo demo --since before.snapshot --strict
+ added    git hook           <repo>/.git/hooks/pre-commit
+ added    git hook           <repo>/.git/hooks/pre-push
+ added    agent instructions <repo>/AGENTS.md
~ changed  git config         <repo>/.git/config
(exit 1)
```

After the installer's own clean-up command, the comparison still showed both hooks and the config key.
The clean-up leaves them behind, which was reported to that tool.

**Verdict: works together.** A legitimate installer and an agent doing the same thing unasked look
identical on disk; glassbox shows exactly what was added, so the operator can tell which is which and
check that an uninstall really removed everything.

## devdrop: installing builds on devices

```
$ glassbox persist --save devdrop.snapshot
$ devdrop doctor
ok  adb …
ok  xcode: Xcode 26.2
ok  ios …: ready
$ glassbox persist --since devdrop.snapshot --strict
No persistence points added, changed or removed since devdrop.snapshot.
(exit 0)
```

**Verdict: compatible, little to gain.** devdrop sets nothing up to run again on the computer (adb's
server is a running process, not a persistence point), so glassbox has nothing to report. `reach` is
not relevant: devdrop does not run agents.

## bug-age

**Verdict: no pairing; not run.** bug-age measures how long bugs live in Rust crates from their git
history. It runs no agents and installs nothing, so none of glassbox's three questions applies to it.
