# glassbox

macOS and Linux tools for seeing what your machine, and the AI agents running on it, can actually do.

## `glassbox reach`: what can my agent touch?

An agent started from your terminal runs as you and inherits your terminal's access, including, on macOS, the privacy grants (Full Disk Access and similar) of the app it was launched from. `glassbox reach` reports what a process started that way can read, before you start the agent.

```
cargo build --release
./target/release/glassbox reach            # human-readable report
./target/release/glassbox reach --json     # for scripts and agent harnesses
./target/release/glassbox reach --strict   # exit 1 if any sensitive location is readable
```

Each result is one of:

| Result | Meaning |
|---|---|
| `readable` | The open succeeded |
| `denied-privacy` | EPERM: blocked by a privacy control or sandbox (macOS TCC, Seatbelt) |
| `denied-permissions` | EACCES: blocked by file permissions or ACLs |
| `absent` | Nothing at that path |

**Safety.** Each location is opened read-only. Directories are opened but never listed, files are opened but never read, and nothing is written. The report shows the process chain and, on macOS, the app whose privacy grants apply.

**Scope.** A curated list of credentials, shell history, browser profiles and private data (mail, messages, photos, contacts), plus macOS protected folders. It checks read access only.

Background and the choice of this tool: `docs/suite-study.md` and `docs/feasibility.md`.
