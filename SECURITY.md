# Security

glassbox inspects your own machine. `glassbox reach` opens files and directories in your home
directory that may hold credentials and private data, to find out whether they *could* be read.

## What glassbox does to stay harmless

- **Opens, never reads.** Files are opened read-only and closed without reading a byte; directories
  are opened and closed without listing their entries. No content reaches glassbox's output.
- **Never writes.** No file is created, modified or deleted. The tests check that a probed file's
  bytes and modification time are unchanged.
- **Reports paths, not contents.** Output contains the location (as `~/...`), its category and the
  result. On macOS, opening a privacy-protected location may itself be recorded by the system, as any
  open would be.
- **`models` maps, never reads.** Model files are mapped read-only and unmapped without touching a
  page, so measuring neither reads model data nor loads it into memory. Ollama manifests are the only
  files whose contents are read, to find each model's weight file.
- **`persist` hashes, never prints.** Persistence files are read to compute a SHA-256 of each; output
  and snapshots contain paths, categories and hashes only, never file contents. For `~/.claude.json`
  only the `mcpServers` and `hooks` sections are hashed.
- **Runs nothing from the paths it probes.** `persist` runs `crontab -l` to read your own crontab; on macOS `reach` runs `ps` (from `PATH`) to read the process
  chain; on Linux it reads `/proc`. Arguments are passed directly, never through a shell.
- **Escapes what it prints.** Command names from the process chain are escaped in JSON output.

## Reporting a vulnerability

Please report security problems privately, through GitHub's "Report a vulnerability" button on the
repository's Security tab. Please do not open a public issue.
