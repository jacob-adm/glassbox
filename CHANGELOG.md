# Changelog

## Unreleased

- `reach`: a directory is judged by its entries. Up to 32 are tried, and the directory is reported
  as `partly-readable` when some open and others are refused, or `listable-only` when all are
  refused. It used to be `readable` whenever the directory itself could be opened, which overstated
  what a sandbox leaves reachable. `--strict` counts `partly-readable` as readable. Entry names are
  never printed or kept.
- `reach --since FILE`: compares with an earlier `reach --json` report. Changed locations show what
  they were, and a last line counts what closed, what is still readable and what is newly readable.
  Meant for checking a sandbox: save a report outside it, compare inside it.
- `reach`: when the process list cannot be read, as inside a sandbox, the report says the process
  chain is not available instead of printing an empty heading.

## 0.4.0 - 2026-11-20

- `watch`: with `--pid`, each capture adds `connections.txt`, the open TCP and UDP connections
  of the agent's process tree when the capture was written. Read as the same user, with no extra
  privileges.

## 0.3.0 - 2026-11-20

- `persist` (macOS): watches login items and background services registered through the
  Service Management framework: launchd plists and login-item helpers inside app bundles in
  `/Applications` and `~/Applications`, and the Background Task Management database.

## 0.2.0 - 2026-11-19

- `watch`: follows a local agent session transcript (Claude Code JSON lines) and, when the agent
  repeats an identical tool call 4 times in a row, produces more than 16,000 output tokens in a turn,
  or fails 5 tool calls in a row, writes a capture with the trigger, the persistence points added or
  changed since watching began, uncommitted changes in `--repo` and the process tree under `--pid`.
  `--pause` stops the agent; `--once` scans a whole transcript. Thresholds are configurable.
  Transcript content is never copied into output or captures.

## 0.1.0 - 2026-11-19

First version. glassbox audits what AI agents and other local processes can access on macOS and
Linux, as a single Rust binary with no dependencies.

- `reach`: what a process started like this one can read, from a curated list of credentials,
  shell history, browser profiles, private data and macOS protected folders. Each location is
  opened read-only and never read; results distinguish privacy controls (EPERM) from file
  permissions (EACCES). Shows the process chain and, on macOS, the app whose privacy grants apply.
  `--strict` exits 1 when a sensitive location is readable.
- `models`: how many bytes of each local model (Ollama, LM Studio, the Hugging Face cache, `--dir`)
  are in the page cache, measured with `mincore` without loading the files.
- `persist`: SHA-256 snapshots of persistence points (launchd, systemd, cron, shell startup, SSH,
  git config and hooks, Python startup files, AI agent configurations) and a comparison that
  reports what was added, changed or removed. `--strict` exits 1 on added or changed points.
- `persist --repo` compares across clones and worktrees (labels are relative to the repository),
  follows linked worktrees and `core.hooksPath`, and watches agent instruction files (`CLAUDE.md`,
  `AGENTS.md`, `GEMINI.md`, Cursor and Copilot rules, Claude skills, agents and commands).
- Hardened against files planted by the audited agents: FIFOs, devices, sparse files, deeply nested
  or malformed JSON and terminal escape sequences cannot hang, crash, stall or mislead it.
- Tests run against real files, processes and the page cache on macOS and on Linux in Docker.
