# Commands

## `glassbox reach`

```
glassbox reach [--json] [--strict] [--home DIR]
```

Reports what a process started like this one could read. Run it from the terminal, editor or
service you start agents from: an agent started there inherits the same user, permissions and, on
macOS, the privacy grants of the responsible app.

| Option | Effect |
|---|---|
| `--json` | Print JSON instead of the text report |
| `--strict` | Exit 1 if any sensitive location is `readable` or `partly-readable` |
| `--since FILE` | Compare with an earlier `reach --json` report in `FILE` |
| `--home DIR` | Resolve locations under `DIR` instead of `$HOME` |

### What it does to each location

| Kind | Probe | Never |
|---|---|---|
| Directory | `opendir` | lists entries |
| File | `open` read-only | reads bytes |
| Anything | – | writes, creates or deletes |

A symlink is followed, as an agent's open would follow it, so the result is for its target.

### Results

| Result | errno | Meaning |
|---|---|---|
| `readable` | – | The open succeeded. For a directory: every file tried in it opened, or it held none to try |
| `partly-readable` | – | A directory in which some entries opened and others were refused |
| `listable-only` | – | A directory that can be listed while every entry tried in it was refused |
| `denied-privacy` | EPERM | A privacy control or sandbox refused it (macOS TCC, Seatbelt, SIP) |
| `denied-permissions` | EACCES | File permissions or ACLs refused it |
| `absent` | ENOENT | Nothing at that path |
| `error-N` | N | Any other error |

### Text output

```
Process chain (nearest first):
     2011  /bin/zsh
      812  /System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal
        1  /sbin/launchd
Privacy grants on macOS follow the responsible app: Terminal.app

! readable             credentials       ~/.ssh
! readable             private data      ~/Library/Mail
  readable             protected folder  ~/Documents

2 sensitive location(s) readable (marked !); 11 not present.
```

Absent locations are counted but not listed.

### JSON output

```json
{"chain":[{"pid":2011,"command":"/bin/zsh"}],
 "responsible_app":"Terminal.app",
 "findings":[{"category":"credentials","sensitive":true,"path":"~/.ssh","reach":"readable"}]}
```

`responsible_app` is `null` when no `.app` bundle is in the chain (always on Linux).

### Checking a sandbox

Save a report from a plain terminal, then run `reach --since` inside the sandbox:

```
glassbox reach --json > before.json
SANDBOX-COMMAND glassbox reach --since before.json
```

Each location whose result changed shows what it was, and a last line counts the locations the
sandbox closed, those still readable and any newly readable:

```
! partly-readable      credentials       ~/.ssh  (was readable)
  denied-privacy       credentials       ~/.config/gcloud  (was readable)

1 sensitive location(s) readable (marked !); 1 partly readable; 11 not present.
Against the earlier report: 1 closed, 1 still readable, 0 newly readable.
```

With `--json`, each finding also carries `"was"`. The sandbox must let glassbox read `FILE`. A
file that is not a `reach --json` report is refused with exit status 2.

Inside a sandbox the process list usually cannot be read. The report then says the process chain
is not available, and the JSON `chain` is empty.

### Exit status

| Status | When |
|---|---|
| 0 | Report printed. With `--strict`, also: no sensitive location readable |
| 1 | `--strict` and at least one sensitive location readable |
| 2 | Usage error, or `--since` given a file that is not a `reach --json` report |
| 2 | Usage error, or `HOME` unset without `--home` |

### Locations

Paths are relative to the home directory. Sensitive locations count towards `--strict`.

| Category | Sensitive | macOS | Linux |
|---|---|---|---|
| credentials | yes | `.ssh`, `.aws/credentials`, `.config/gcloud`, `.azure`, `.kube/config`, `.docker/config.json`, `.netrc`, `.npmrc`, `.pypirc`, `.git-credentials`, `.config/gh/hosts.yml`, `.gnupg`, `.password-store`, `Library/Keychains` | the same, with `.local/share/keyrings` instead of `Library/Keychains` |
| history | yes | `.zsh_history`, `.bash_history`, `.python_history` | the same |
| private data | yes | `Library/Mail`, `Library/Messages`, `Library/Safari`, `Library/Application Support/AddressBook`, `Library/Calendars`, `Pictures/Photos Library.photoslibrary` | `.thunderbird`, `.local/share/evolution` |
| browser profile | yes | Chrome, Firefox and Brave under `Library/Application Support` | `.mozilla/firefox`, `.config/google-chrome`, `.config/chromium`, `.config/BraveSoftware` |
| protected folder | no | `Desktop`, `Documents`, `Downloads`, `Library/Mobile Documents` | – |

### Limits

- Read access only. Write access is not probed, because a non-destructive write test is not
  possible for every location.
- A sandbox applied *to the agent* (for example sandbox-runtime or nono) is not seen unless glassbox
  runs inside the same sandbox. Run it there to check the policy.
- On Linux, confinement by AppArmor, SELinux or Landlock shows up only as a refused open; glassbox
  does not read the policy.

## `glassbox models`

```
glassbox models [--json] [--home DIR] [--dir DIR]...
```

Lists local model weight files and how many bytes of each are in the operating system's page cache.
A model in the page cache loads from memory; a model that is not loads from disk.

| Option | Effect |
|---|---|
| `--json` | Print JSON instead of the table |
| `--home DIR` | Look for model stores under `DIR` instead of `$HOME` |
| `--dir DIR` | Also search `DIR` (four levels deep) for `.gguf` and `.safetensors` files; repeatable |

### Where it looks

| Source | Location under the home directory | Model name |
|---|---|---|
| `ollama` | `.ollama/models/manifests/…` → `blobs/` (layers of type `image.model` and `image.projector`) | `model:tag`, with namespace and registry when not the default |
| `lmstudio` | `.lmstudio/models/<publisher>/<model>/`, `.cache/lm-studio/models/…` | `publisher/model` |
| `huggingface` | `.cache/huggingface/hub/models--<org>--<name>/snapshots/` | `org/name` |
| `dir` | each `--dir` | file name |

A file reached twice (for example by two snapshots, or a symlink from one tool to another) is counted
once, under the first model that claims it. Sharded models are summed across their shards.

### How it measures

Each file is mapped read-only and the kernel reports, page by page, which pages are resident
(`mincore`). No page is touched, so measuring does not load the file. Results can change between
measurement and use as the kernel evicts or loads pages.

### Text output

```
MODEL                                               SOURCE            SIZE  IN MEMORY
lmstudio-community/gemma-3-270m-it-MLX-8bit         lmstudio       442 MiB    442 MiB  100%
gpt-oss:20b                                         ollama        12.8 GiB      0 MiB    0%

2 model(s), 13.2 GiB on disk, 442 MiB in the page cache.
```

Models are sorted by bytes in memory, then name.

### JSON output

```json
{"models":[{"name":"gpt-oss:20b","source":"ollama","bytes":13793422816,"resident":0,
  "files":["/Users/me/.ollama/models/blobs/sha256-…"]}]}
```

### Exit status

0 when the list is printed (including when no models are found); 2 for a usage error.

### Limits

- Page cache only. A model already loaded by a running server (Ollama, LM Studio, llama.cpp) may hold
  its weights in GPU or other anonymous memory, which is not counted. Use the server's own status
  (`ollama ps`, `lms ps`) for that.
- Only `.gguf` and `.safetensors` files are counted; PyTorch `.bin` checkpoints are not.
- Only the ranges of a file that hold data are measured (`SEEK_DATA`/`SEEK_HOLE`); holes in a sparse
  file are skipped. Model files are not sparse, so this changes nothing for them, and a huge sparse
  file cannot stall the command.
- Ollama blobs are found only through digests of the form `sha256:` plus 64 hex digits.

## `glassbox persist`

```
glassbox persist [--json] [--home DIR] [--repo DIR]...
glassbox persist --save FILE [--home DIR] [--repo DIR]...
glassbox persist --since FILE [--strict] [--json] [--home DIR] [--repo DIR]...
```

Lists persistence points, the places where something can be set up to run again later, with a
SHA-256 of each. Save a snapshot before an agent session and compare with it afterwards.

| Option | Effect |
|---|---|
| `--save FILE` | Write a snapshot (paths, categories and hashes; no contents) |
| `--since FILE` | Compare the current state with the snapshot |
| `--strict` | With `--since`: exit 1 if anything was added or changed |
| `--json` | Print JSON |
| `--home DIR` | Use `DIR` instead of `$HOME` for per-user locations |
| `--repo DIR` | Also watch `DIR`'s git hooks (following linked worktrees and `core.hooksPath`), git config, agent configs and agent instruction files; repeatable. Points are labelled `<repo>/…` (`<repo2>/…` for the second), so a snapshot of one clone or worktree compares with another |

### What is watched

| Category | macOS | Linux |
|---|---|---|
| autostart | `~/Library/LaunchAgents`, `/Library/LaunchAgents`, `Contents/Library/LaunchAgents` in each app bundle in `/Applications` and `~/Applications` | `~/.config/systemd/user`, `~/.local/share/systemd/user`, `~/.config/autostart`, `/etc/xdg/autostart` |
| system service | `/Library/LaunchDaemons`, `Contents/Library/LaunchDaemons` in each app bundle | `/etc/systemd/system`, `/etc/rc.local` |
| login items | the Background Task Management database (`/private/var/db/com.apple.backgroundtaskmanagement`), and `Contents/Info.plist` of each helper in an app bundle's `Contents/Library/LoginItems` | – |
| cron | `crontab -l` | `crontab -l`, `/etc/crontab`, `/etc/cron.d` |
| preload | – | `/etc/ld.so.preload` |
| shell startup | `.zshrc`, `.zprofile`, `.zshenv`, `.zlogin`, `.zlogout`, `.bashrc`, `.bash_profile`, `.bash_login`, `.bash_logout`, `.profile`, fish `config.fish` and `conf.d` | the same, plus `/etc/profile.d` |
| ssh | `~/.ssh/rc`, `environment`, `authorized_keys`, `authorized_keys2`, `config` | the same |
| git config | `~/.gitconfig`, `~/.config/git/config`, each `--repo`'s `.git/config` | the same |
| git hook | each `--repo`'s `.git/hooks` | the same |
| python startup | `.pth`, `usercustomize.py`, `sitecustomize.py` in `~/Library/Python/*/…/site-packages` | the same in `~/.local/lib/python*/site-packages` |
| agent instructions | `~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`, `~/.gemini/GEMINI.md`, `~/.claude/{skills,agents,commands}`; in each `--repo`: `CLAUDE.md`, `AGENTS.md`, `GEMINI.md`, `.cursorrules`, `.windsurfrules`, `.github/copilot-instructions.md`, `.claude/{skills,agents,commands}`, `.cursor/rules` | the same |
| agent config | `~/.claude/settings.json`, `~/.claude/settings.local.json`, `~/.cursor/mcp.json`, `~/.codex/config.toml`, `~/.gemini/settings.json`, and the `mcpServers` and `hooks` sections of `~/.claude.json` | the same |

`~/.claude.json` is rewritten with usage data on every session, so only its `mcpServers` and `hooks`
sections are hashed, at any depth and with keys sorted. Usage counters and key order do not register
as changes. If the file is not valid JSON, the whole file is hashed.

The Background Task Management database changes when a login item or background service is
registered, approved or removed. It is compared as a whole; run `sfltool dumpbtm` (administrator)
to see which item changed. Symlinked app bundles are not followed.

### Repositories

- Points under a `--repo` are labelled `<repo>/…` (`<repo2>/…` for the second), so pass the `--repo`
  options in the same order to `--save` and `--since`.
- Hooks are read from the directory git uses: `core.hooksPath` from `/etc/gitconfig`, `~/.config/git/config`,
  `~/.gitconfig`, the repository's config and, when `extensions.worktreeConfig` is on, `config.worktree`
  (later ones win; `~` is expanded). Files pulled in with `include.path` are not read.
- In a linked worktree, the shared hooks and config of the main repository are watched.
- A repository's local, untracked Claude instruction file is not watched.

### Results

| Change | Meaning | Fails `--strict` |
|---|---|---|
| `added` | A persistence point exists that was not in the snapshot | yes |
| `changed` | Its content hash differs from the snapshot | yes |
| `removed` | It was in the snapshot and is gone | no |

Content is compared by SHA-256, so rewriting a file with the same content, or changing only its
modification time, is not a change.

### Text output

```
+ added    autostart       ~/Library/LaunchAgents/com.example.agent.plist
~ changed  shell startup   ~/.zshrc
- removed  shell startup   ~/.profile
```

### JSON output

```json
{"changes":[{"change":"added","category":"autostart","path":"~/Library/LaunchAgents/com.example.agent.plist"}]}
```

Without `--since`, `--json` prints `{"items":[{"category":…,"path":…,"sha256":…}]}`.

### Exit status

| Status | When |
|---|---|
| 0 | Listed, saved, or compared. With `--strict`, also: nothing added or changed |
| 1 | `--since --strict` and something was added or changed |
| 2 | Usage error, or the snapshot cannot be read or written (including a snapshot saved by a different glassbox version: save a new one) |

### Limits

- macOS login items registered through Background Task Management (`sfltool dumpbtm`) are not read,
  because reading them requires administrator authentication.
- Only your own crontab is read; other users' crontabs require root.
- Files are hashed in full. A file larger than 1 GiB is compared by size, inode and change time
  (ctime, which a normal user cannot set) instead, so a huge file cannot stall the audit.
- FIFOs, devices and sockets are never read; they are compared by type and symlink target.
- A file that cannot be read is compared by size and modification time instead of content.
- In `~/.claude.json`, any part nested more than 256 levels deep is included whole and compared as
  written, because it could hide a hook. Changing, reformatting or reordering it reports a change.
  This can only add a report, never hide one.
- A file larger than 1 MiB that is mostly holes (sparse) is compared by size, inode and change time.
- A directory where a file is expected is compared by the names it contains.
- `--save` writes a new file and renames it into place, so a FIFO or symlink at the target is replaced,
  never written through.
- An agent that can write to the snapshot file can hide its changes. Keep the snapshot where the agent
  cannot write, for example outside its sandbox or working directory.

## `glassbox watch`

```
glassbox watch [TRANSCRIPT] [--once] [--out DIR] [--loop N] [--spike TOKENS] [--failures N]
               [--pid PID [--pause]] [--repo DIR]... [--home DIR]
```

Follows a local agent session and captures the machine's state when the agent goes wrong.

| Option | Effect |
|---|---|
| `TRANSCRIPT` | The session file to follow. Default: the most recently modified `~/.claude/projects/*/*.jsonl` |
| `--once` | Scan the whole transcript instead of following it; exit 1 if any trigger fires |
| `--out DIR` | Where captures go. Default when following: `~/.local/state/glassbox/captures`. With `--once`, captures are written only if `--out` is given |
| `--loop N` | Identical tool calls (same tool, same arguments) in a row that count as a loop (default 4) |
| `--spike TOKENS` | Output tokens in one assistant turn that count as a spike (default 16000) |
| `--failures N` | Failed tool calls in a row (default 5) |
| `--pid PID` | The agent's process: its process tree and their open connections go into each capture, and watching stops when it exits |
| `--pause` | With `--pid`, while following: stop the agent (SIGSTOP) when a trigger fires. Resume it with `kill -CONT PID`. Refused with `--once`, so a scan of past activity never stops a live process |
| `--repo DIR` | Also watch `DIR`'s persistence points and record its uncommitted changes; repeatable |
| `--home DIR` | Use `DIR` instead of `$HOME` |

### How it works

When following, glassbox starts at the end of the transcript, so what is already there does not
fire, and takes a `persist` snapshot. Each new line is parsed for structure only: record type, tool
name, a SHA-256 of the tool name and arguments, output token count and the tool result's error flag.
A loop fires once when a run of identical calls reaches the threshold; failures fire once when a run
of failed results reaches it; a spike fires once per model response over the threshold (Claude Code
writes one response as several lines sharing an id). Line numbers in triggers are line numbers in the
transcript file. A transcript replaced by rename is followed from the start of the new file.

The default thresholds come from real sessions: across 1,309 tool calls in three sessions, no
identical call ran more than twice in a row, and no turn produced more than 9,821 output tokens.
Across 10 real sessions, `--once` with the defaults fired nothing.

### Captures

Each trigger writes a new directory `<seconds>-<n>-<kind>` (with a numeric suffix if that name exists;
an existing directory is never reused, and files are always created new, so nothing is written
through a planted link) with:

| File | Contents |
|---|---|
| `trigger.json` | Kind (`loop`, `spike`, `failures`), observed value, threshold, tool name and call hash (loops), transcript line and path |
| `persist.json` | Persistence points added, changed or removed since watching began (`persist --since` format) |
| `repos.txt` | `git status --porcelain` for each `--repo`: file names and states, no diffs |
| `processes.txt` | pid, parent, state and command name for `--pid` and every process below it |
| `connections.txt` | Open TCP and UDP connections of the same processes when the capture was written: pid, protocol, local and remote address, state. A point-in-time list; connections that opened and closed earlier are missing. Read as the same user (`lsof` on macOS, `/proc` on Linux) |

No message text or tool arguments are copied into a capture or printed.

### Output

```
glassbox watch: watching ~/.claude/projects/…/session.jsonl (loop 4, spike 16000, failures 5); captures in ~/.local/state/glassbox/captures
trigger loop: 4 identical Bash calls in a row (line 812); capture: ~/.local/state/glassbox/captures/1795540000-1-loop; paused 4242, resume with: kill -CONT 4242
```

### Exit status

| Status | When |
|---|---|
| 0 | `--once` and nothing fired; or following stopped because `--pid` exited |
| 1 | `--once` and at least one trigger fired |
| 2 | Usage error, no transcript found, an unreadable transcript, or a format glassbox does not know |

### Limits

- Claude Code transcripts only. Their format is not a documented interface; a file with no record
  glassbox recognises is refused (exit 2) rather than watched silently.
- Captures go under your home directory by default, outside any repository you watch. An `--out`
  inside a watched repository shows up in that repository's uncommitted changes.
- Thresholds were set from a small number of real sessions; adjust them for your own work.
