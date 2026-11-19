# Sysinternals Suite study: what to port, what to learn from, what to refuse

Scope: every tool in the current Sysinternals Suite (79 programs, one row per tool; 64-bit and ARM variants are folded together). Each row answers three questions:

1. **Reverse engineering.** May we reverse engineer it, and if not, what lawful source do we learn from instead?
2. **macOS/Linux port.** Is there a real gap on macOS or Linux that a port would fill?
3. **Agent harness / inference.** Is the *concept* behind the tool useful for building agent harnesses or running inference, even when the tool itself is not worth porting?

The audience test comes from a developer question: someone with ten years of managed-runtime experience who wants to understand the operating system without reading the whole of *Windows Internals*. A port or concept earns a place only if it gives that person real insight they cannot already get from a built-in command.

## 1. Reverse engineering: refused for every closed Suite binary

The Sysinternals licence terms (embedded in each binary and published at <https://learn.microsoft.com/en-us/sysinternals/license-terms>) say:

> You may not … reverse engineer, decompile or disassemble the software, except and only to the extent that applicable law expressly permits, despite this limitation.

Interoperability exceptions in law (for example Article 6 of the EU Software Directive) cover making an independently created program work *with* the original. A macOS/Linux port does not interoperate with the Windows tool, so we do not rely on them. This is a reading of the licence, not legal advice.

**Decision:** no reverse engineering, decompiling or disassembling of any of the 79 closed Suite binaries. Every port is written clean-room from lawful sources:

| Lawful source | What it gives us |
|---|---|
| Microsoft's public tool pages and usage text | Documented behaviour and options |
| *Windows Internals* 7th ed. (Part 1 ch1–7; Part 2 ch8–12) | The OS mechanism each tool exposes |
| Official ports, MIT licence: [microsoft/ProcMon-Linux](https://github.com/microsoft/ProcMon-Linux), [microsoft/Sysmon-Linux](https://github.com/microsoft/Sysmon-Linux), [microsoft/ProcDump-Linux](https://github.com/microsoft/ProcDump-Linux), [microsoft/ProcDump-Mac](https://github.com/microsoft/ProcDump-Mac), [microsoft/Zoomit-Mac](https://github.com/microsoft/Zoomit-Mac); LGPL-2.1: [microsoft/SysinternalsEBPF](https://github.com/microsoft/SysinternalsEBPF) | Real source we may read and learn from, within those licences |
| macOS/Linux man pages, kernel docs, RFCs, papers | The implementation on the target OS |

**Where REA is used:** on targets we are allowed to inspect, such as the MIT-licensed ports above, open-source macOS/Linux tools, and our own builds (to verify that what we ship matches what we claim).

**Removed:** an earlier pass read the Suite binaries' import tables and resources with REA. Because that is plausibly "reverse engineering" under the licence, its output was deleted and nothing in this study depends on it. The tool roster (`research/tools.txt`) comes from the file names in Microsoft's public Suite download.

## 2. Verdict per tool

Port verdicts:

- **PORT**: a real gap with real value; clean-room port planned.
- **OFFICIAL**: Microsoft already ships a macOS/Linux version.
- **NATIVE**: a built-in command already covers it (named).
- **N/A**: depends on a Windows-only concept.
- **LOW**: a gap exists, but only third-party tools fill it, or the value is small.
- **DROP**: a port would mislead on modern hardware.

Concept value: **H** high, **M** medium, **L** low, **–** none for agent harnesses or inference.

| Tool | What it does | macOS / Linux today | Port | Agent harness / inference concept | Concept |
|---|---|---|---|---|---|
| `accesschk` | Effective rights of a user or group on files, keys, services | `ls -le`, `getfacl`, `namei -l`; nothing answers "what can user X write, everywhere" | **PORT** | *What can my agent touch?* Least-privilege audit of the identity an agent runs as | H |
| `AccessEnum` | Finds permission inconsistencies across a tree | `find -perm`, `getfacl -R` | LOW (fold into accesschk port) | Same audit, tree-wide | M |
| `ADExplorer` | Active Directory browser with offline snapshots and snapshot diff | `ldapsearch` (AD-specific) | N/A | Snapshot then diff: a before/after record of what an agent run changed | M |
| `ADInsight` | Traces an application's LDAP calls | N/A | N/A | Per-application call tracing, the same shape as tool-call tracing | L |
| `adrestore` | Restores deleted AD objects | N/A | N/A | – | – |
| `Autologon` | Stores credentials for automatic logon | N/A (and a security anti-pattern) | N/A | – | – |
| `Autoruns` | Everything configured to start automatically, with signature checks and baseline compare | macOS `sfltool dumpbtm`, Linux `systemctl list-unit-files`; fragmented, no single view | **PORT** | *What did the agent leave behind?* Baseline before a run, compare after: shell rc files, cron, launchd, systemd, git hooks | H |
| `autorunsc` | Command-line Autoruns | as above | **PORT** (same tool) | as above | H |
| `Bginfo` | Writes system facts onto the desktop wallpaper | Third-party only (conky, GeekTool) | LOW | Ambient context: always-visible identity of the machine or session | L |
| `Cacheset` | Sets the file-cache working-set size | Linux `vm.*` sysctls; macOS none | N/A | Feeds the RAMMap concept | L |
| `Clockres` | Reports system timer resolution | APIs only (`clock_getres`, `mach_timebase_info`); no command | LOW (fold into a benchmark-hygiene check) | Timer granularity bounds every latency number you report | M |
| `Contig` | Defragments single files | `e4defrag`; meaningless on SSD/APFS | DROP | – | – |
| `Coreinfo` | CPU topology, features, caches, NUMA | `lscpu`, `sysctl hw machdep.cpu` | NATIVE | Placing inference threads on the right cores (P/E cores, NUMA nodes) | M |
| `CoreinfoEx` | Extended Coreinfo | as above | NATIVE | as above | M |
| `CPUSTRES` | Per-thread CPU load generator | Third-party (`stress-ng`) | LOW | Noisy-neighbour injection to check that a benchmark result holds under load | M |
| `ctrl2cap` | Remaps Caps Lock to Ctrl (driver) | Keyboard settings, `setxkbmap` | NATIVE | – | – |
| `dbgview` | Captures debug output from apps and kernel | `log stream`, `dmesg`, `journalctl` | NATIVE | Tapping a side-channel log without attaching to the process | M |
| `dbgviewcli` | Command-line DebugView | as above | NATIVE | as above | M |
| `Desktops` | Virtual desktops | Spaces, workspaces | NATIVE | Isolated workspaces per task (worktrees serve this for agents) | L |
| `disk2vhd` | Images a live machine to a virtual disk | `dd`, `asr`, LVM/APFS snapshots | NATIVE | Capturing a whole environment so an agent eval can be replayed | M |
| `diskext` | Maps volumes to disk extents | `diskutil`, `lsblk` | NATIVE | – | – |
| `Diskmon` | Logs disk sector activity | `fs_usage -f diskio`, `iostat`, `/proc/diskstats` | NATIVE | – | L |
| `DiskView` | Graphical map of file clusters on disk | Linux `filefrag`; physical layout hidden by the SSD's translation layer | DROP | – | – |
| `du` | Directory disk usage | `du` | NATIVE | – | – |
| `efsdump` | Lists EFS encrypted-file users | N/A | N/A | – | – |
| `FindLinks` | Lists hard links to a file | `find -samefile` | NATIVE | – | – |
| `handle` | Open handles per process | `lsof` | NATIVE | Resource leaks in long-lived agent or MCP server processes | M |
| `hex2dec` | Hex/decimal conversion | `printf` | NATIVE | – | – |
| `junction` | NTFS junctions | `ln -s` | NATIVE | – | – |
| `ldmdump` | Dynamic-disk database dump | N/A | N/A | – | – |
| `Listdlls` | Loaded DLLs per process | `vmmap`, `otool -L`, `/proc/PID/maps`, `pldd` | NATIVE | Provenance of loaded code: which plugins and libraries an agent process actually runs | L |
| `livekd` | Kernel debugging of the running system | Linux `crash` with `/proc/kcore`; macOS KDK | N/A | Observing a live system without stopping it | M |
| `LoadOrd` | Driver and service load order | `kmutil showloaded`, `systemd-analyze critical-chain`, `lsmod` | NATIVE | Startup ordering, the same problem as hook execution order | L |
| `LoadOrdC` | Command-line LoadOrder | as above | NATIVE | as above | L |
| `logonsessions` | Active logon sessions | `who`, `last`, `loginctl` | NATIVE | Session inventory, like a registry of running agent sessions | L |
| `movefile` | Schedules a file move or delete for next boot | N/A | N/A | Staged actions applied only at a gate: plan, then approve, then apply | M |
| `notmyfault` | Deliberately crashes, hangs or leaks via a driver | Linux `/proc/sysrq-trigger`; macOS none | N/A | Fault injection to prove recovery works: chaos testing for the harness itself | H |
| `notmyfaultc` | Command-line NotMyFault | as above | N/A | as above | H |
| `ntfsinfo` | NTFS volume internals | N/A | N/A | – | – |
| `pendmoves` | Lists scheduled boot-time moves | N/A | N/A | Shows what is staged and waiting for the gate (pairs with movefile) | M |
| `pipelist` | Named pipes | `lsof -U`, `ss -x` | NATIVE | IPC surface inventory (MCP over stdio or sockets) | L |
| `portmon` | Serial/parallel port monitor (driver) | Third-party only | LOW | – | L |
| `procdump` | Dumps a process when a trigger fires (CPU, memory, exception) | **Microsoft ports for Linux and macOS** | OFFICIAL | Trigger-conditioned capture: snapshot an agent's state when a token spike, loop or error fires. MIT source is readable | H |
| `procexp` | Process tree with handles, DLLs, signatures | Activity Monitor; `htop`/`btop` third-party | NATIVE | Attribution: which agent spawned which subprocess | M |
| `Procmon` | Live trace of file, registry, process and network activity with filters | **Microsoft port for Linux**; macOS `fs_usage`, `eslogger` | OFFICIAL | Full action trace of an agent run with a filter model. MIT source is readable | H |
| `PsExec` | Remote execution by installing a temporary service | `ssh` | NATIVE | Remote agent execution; the service-install method is a warning, not a model | L |
| `psfile` | Files opened remotely | `lsof`, `smbstatus` | NATIVE | – | – |
| `PsGetsid` | Account SIDs | `id`, `dscl` | NATIVE | – | – |
| `PsInfo` | System information | `uname`, `sysctl`, `system_profiler`, `hostnamectl` | NATIVE | – | – |
| `pskill` | Kills processes | `kill` | NATIVE | – | – |
| `pslist` | Lists processes | `ps` | NATIVE | – | – |
| `PsLoggedon` | Who is logged on | `who` | NATIVE | – | – |
| `psloglist` | Event log dump | `log show`, `journalctl` | NATIVE | – | L |
| `pspasswd` | Changes passwords | `passwd` | NATIVE | – | – |
| `psping` | ICMP and TCP latency with histograms, plus bandwidth | `ping` is ICMP only; no TCP connect-latency histogram built in | **PORT** | Inference endpoint latency: open-loop arrivals so stalls are not hidden, percentiles, time to first token | H |
| `PsService` | Service control | `launchctl`, `systemctl` | NATIVE | – | – |
| `psshutdown` | Shutdown and reboot | `shutdown` | NATIVE | – | – |
| `pssuspend` | Suspends and resumes processes | `kill -STOP` / `kill -CONT` | NATIVE | Pause a runaway agent instead of killing it, so its state survives for inspection | M |
| `RAMMap` | Physical memory by category, page list and file | Linux `/proc/meminfo`, `fincore` (one file at a time); macOS `vm_stat`. No "which files are in RAM" view | **PORT** | *Is the model resident?* Memory-mapped weights in the page cache against anonymous KV-cache memory; why cold start is slow | H |
| `RDCMan` | Manages many remote desktop connections | Third-party (Windows App, Remmina) | N/A | – | L |
| `RegDelNull` | Finds registry keys whose names contain NUL, which Win32 tools cannot display | N/A | N/A | Content the human cannot see but the machine still reads: invisible Unicode tag characters used for prompt injection | H |
| `regjump` | Opens Regedit at a path | N/A | N/A | – | – |
| `ru` | Registry usage per key | N/A | N/A | – | – |
| `sdelete` | Overwrites files to delete them securely | Linux `shred`; unreliable on SSDs | DROP | Deleting is not erasing: agent "forgetting" leaves copies in caches, logs and transcripts | M |
| `ShareEnum` | Finds SMB shares and their permissions across a network | Third-party (`nmap` scripts, `smbclient`) | LOW | – | – |
| `ShellRunas` | Runs a program as another user from the shell menu | `sudo -u` | NATIVE | Run the agent as its own low-privilege identity (pairs with accesschk) | M |
| `sigcheck` | File version, signature and VirusTotal check | macOS `codesign`, `spctl`; Linux has no ELF signing (`rpm -V`, `debsums` for packages) | NATIVE (macOS) / LOW (Linux) | Provenance of skills, plugins and model weights: hash, signature, reputation | H |
| `streams` | Lists NTFS alternate data streams | `xattr`, `getfattr` | NATIVE | Hidden channels next to visible content, the root of indirect prompt injection | H |
| `strings` | Extracts printable strings | `strings` | NATIVE | Secret and leak scanning of artifacts | L |
| `sync` | Flushes file-system buffers | `sync` | NATIVE | – | – |
| `Sysmon` | Persistent system activity logging with a stable event schema | **Microsoft port for Linux**; macOS `eslogger` | OFFICIAL | A stable event schema (IDs, hashes, parent/child) for agent action audit logs. MIT source is readable | H |
| `tcpvcon` | Command-line TCPView | `lsof -i`, `ss -tp`, `nettop` | NATIVE | Egress audit: which endpoints this agent contacted | M |
| `tcpview` | Connections per process | as above | NATIVE | as above | M |
| `Testlimit` | Pushes handles, memory and threads until something fails | Third-party (`stress-ng`) | LOW | Find real limits by pushing until failure (context size, rate limits, concurrent tool calls) instead of trusting the documented ones | M |
| `vmmap` | Virtual memory map of a process | macOS `vmmap`, Linux `pmap` | NATIVE | Memory map of an inference process: mapped weights, heap and GPU-shared memory | M |
| `Volumeid` | Changes a volume serial number | `tune2fs -U` | NATIVE | – | – |
| `whois` | Domain registration lookup | `whois` | NATIVE | – | – |
| `Winobj` | Browses the object manager namespace | N/A | N/A | Namespace browser, like inspecting an MCP tool namespace | L |
| `ZoomIt` | Screen zoom, annotation, timer, recording | **Microsoft port for macOS** | OFFICIAL | – | L |

## 3. What survives, and for what

### Track A: macOS/Linux ports (clean-room)

| Rank | Port | Gap it fills | Teaches (Windows Internals) | Inference / harness use | Grounding |
|---|---|---|---|---|---|
| 1 | **RAMMap** | "Which files are in RAM, and what is memory actually doing?" Linux `cachestat()` (6.5+) and `mincore`; macOS `mincore` and `host_statistics64` | P1 ch5 memory, P2 ch11 cache | Model-weight residency, cold against warm start | [cachestat(2)](https://man.archlinux.org/man/cachestat.2.en); Kwon et al., *PagedAttention*, SOSP 2023, [doi:10.1145/3600006.3613165](https://doi.org/10.1145/3600006.3613165) |
| 2 | **Autoruns** | One view of persistence across BTM, launchd, extensions, systemd, cron, XDG, `ld.so.preload`, shell rc, git hooks; baseline compare | P2 ch12 startup, ch10 services | "What did the agent leave behind?" | [MITRE ATT&CK TA0003](https://attack.mitre.org/tactics/TA0003/); [Elastic Linux persistence primer](https://www.elastic.co/security-labs/primer-on-persistence-mechanisms); prior art [KnockKnock](https://objective-see.com/products/knockknock.html) (macOS only) |
| 3 | **PsPing** | TCP connect and HTTP time-to-first-byte latency with correct percentiles; open-loop scheduling corrects coordinated omission | P2 ch8 timers | Inference endpoint latency, time to first token | [RFC 2681](https://www.rfc-editor.org/rfc/rfc2681); Reddi et al., *MLPerf Inference*, ISCA 2020, [doi:10.1109/ISCA45697.2020.00045](https://doi.org/10.1109/ISCA45697.2020.00045); [coordinated omission](https://groups.google.com/g/mechanical-sympathy/c/icNZJejUHfE) |
| 4 | **AccessChk** (+AccessEnum, ShellRunas) | "What can user X read and write, everywhere?" | P1 ch7 security | "What can my agent touch?" | Saltzer & Schroeder, *The Protection of Information in Computer Systems*, Proc. IEEE 1975, [doi:10.1109/PROC.1975.9939](https://doi.org/10.1109/PROC.1975.9939) |

### Track B: agent harness concepts (become harness features, not ports)

| Concept | From | What it becomes |
|---|---|---|
| Persistence baseline and diff | Autoruns, ADExplorer snapshots | Snapshot persistence points before an agent run, diff after, fail the run on unexplained additions |
| Action trace with a stable schema | Procmon, Sysmon (MIT source) | An event schema for agent tool calls, file writes, process spawns and network egress |
| Trigger-conditioned capture | ProcDump (MIT source) | Capture transcript and state when a trigger fires: token spike, loop, repeated failure |
| Fault injection | NotMyFault | Deliberately break the harness (kill the model call, corrupt a tool result) to prove recovery. Basiri et al., *Chaos Engineering*, IEEE Software 2016, [doi:10.1109/MS.2016.60](https://doi.org/10.1109/MS.2016.60) |
| Hidden-content scanner | RegDelNull, Streams | Detect content invisible to humans but read by models: Unicode tag characters, zero-width text, hidden HTML. Greshake et al., *Not What You've Signed Up For*, AISec 2023, [doi:10.1145/3605764.3623985](https://doi.org/10.1145/3605764.3623985); [Unicode tag smuggling](https://embracethered.com/blog/posts/2024/hiding-and-finding-text-with-unicode-tags/) |
| Provenance check | Sigcheck | Hash and signature check for skills, plugins and model weights before load |
| Staged actions | MoveFile, PendMoves | Changes queued and listed, applied only after approval |
| Least-privilege identity | AccessChk, ShellRunas | Agent runs as its own user; audit what that user can reach |
| Egress audit | TCPView | Record and allowlist the endpoints an agent contacts |
| Empirical limits | Testlimit | Probe real context, rate and concurrency limits instead of trusting the documented ones |

### Track C: inference engineering

| Concept | From | Use |
|---|---|---|
| Weight residency | RAMMap, VMMap | Is the GGUF or safetensors file in the page cache? Explains cold-start latency |
| Latency measurement | PsPing, Clockres | Open-loop time to first token and time per output token, percentiles, timer granularity stated |
| Placement | Coreinfo | Pin inference threads to performance cores or one NUMA node |
| Benchmark hygiene | CPUSTRES, Clockres | Repeat measurements under controlled background load |

### Dropped, with reasons

- **DiskView, Contig**: on SSDs the translation layer hides physical placement, so a block map shows logical addresses only.
- **SDelete**: overwriting a single file does not reliably erase it on flash. Wei et al., *Reliably Erasing Data From Flash-Based Solid State Drives*, FAST 2011, <https://www.usenix.org/conference/fast11/reliably-erasing-data-flash-based-solid-state-drives>. A port would give false assurance.
- **ZoomIt, ProcDump, Procmon (Linux), Sysmon (Linux)**: Microsoft already ships them.
- **About 30 tools**: a built-in command already covers them (see the table).
- **About 20 tools**: they depend on Windows-only concepts (Active Directory, registry, NTFS, EFS, the object manager).

## 4. Starting list

1. **RAMMap** port. It has the clearest value for the developer question, serves inference directly, and has no built-in equivalent.
2. **Autoruns** port with baseline compare. It doubles as the first harness feature, "what did the agent leave behind?"
3. **PsPing** port with open-loop latency. It doubles as the inference latency tool.
4. **Hidden-content scanner** (Track B). New code with no port needed, and it is the strongest harness concept.
5. **AccessChk** port, then the remaining Track B concepts as harness features.

Every item starts from lawful sources only (section 1) and has a failing test against real OS state before its implementation.
