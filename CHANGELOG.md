# Changelog

## Unreleased

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
- Tests run against real files, processes and the page cache on macOS and on Linux in Docker.
