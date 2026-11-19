# glassbox

glassbox audits what AI agents and other local processes can access on macOS and Linux.

An agent started from your terminal runs as you. It inherits your files, your credentials and, on
macOS, the privacy grants (such as Full Disk Access) of the app it was launched from. glassbox shows
that reach before you start the agent. It is a single Rust binary with no dependencies.

Status: 0.1.0, unreleased. One command so far: `reach`.

## Install

From a clone of this repository:

```
cargo install --path .
```

## Requirements

- macOS or Linux.
- Rust 1.85 or later to build.
- Nothing else at run time. On macOS the process chain comes from `ps`; on Linux, from `/proc`.

## How it works

`glassbox reach` opens each location on a curated list (credentials, shell history, browser
profiles, mail, messages, photos, and the macOS protected folders) read-only, from its own process.
A child process started the same way, such as an agent, inherits the same user, permissions and
privacy grants, so glassbox's reach is the agent's reach.

Directories are opened but never listed, files are opened but never read, and nothing is written.
Each location is reported as `readable`, `denied-privacy` (EPERM: a privacy control or sandbox),
`denied-permissions` (EACCES: file permissions) or `absent`. The report also shows the process chain
and, on macOS, the app whose privacy grants apply.

## Commands

| Command | What it does |
|---|---|
| `glassbox reach` | Report what a process started like this one can read |
| `glassbox reach --json` | The same report as JSON, for scripts and agent harnesses |
| `glassbox reach --strict` | Exit 1 if any sensitive location is readable |
| `glassbox reach --home DIR` | Audit `DIR` instead of `$HOME` |

See [Commands](docs/commands.md) for the full output, exit status and location list.

## Quick start

Run it from the terminal you start agents from:

```sh
glassbox reach
```

Lines marked `!` are sensitive locations an agent started from that terminal could read. To refuse
to start an agent while any are readable:

```sh
glassbox reach --strict --json > reach.json && my-agent
```

## Documentation

- [Commands](docs/commands.md): each command's options, output and exit status.
- [Suite study](docs/suite-study.md): every Sysinternals tool assessed for a macOS/Linux port and for
  its value to agent harnesses and inference.
- [Feasibility](docs/feasibility.md): prior art and feasibility for the five candidate tools, and
  why `reach` came first.

## License

Apache-2.0; see `LICENSE`.
