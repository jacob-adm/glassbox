# Feasibility and prior art: five AI tooling candidates

The five candidates come from `docs/suite-study.md`. Each was checked for:

- **Prior art:** existing projects, libraries and products. GitHub stars, licence and last push were read from the GitHub API on 2026-11-19.
- **Feasibility on macOS and Linux:** where possible, a direct check on a real machine (macOS, Apple silicon, APFS).

Lawful sources only: no Sysinternals binary was reverse engineered.

## Summary

| # | Candidate | Verdict | One-line reason |
|---|---|---|---|
| 1 | Model residency ("RAMMap for models") | **BUILD (small)** | The mechanism exists (vmtouch, pcstat), but nothing ties page-cache residency to *models*. Checked on a macOS test machine. |
| 2 | Persistence diff for agent runs ("Autoruns diff") | **BUILD (narrow)**, or contribute upstream | nono already snapshots and rolls back *tracked directories*. Nothing found watches *persistence points* outside the workspace for unsandboxed runs. |
| 3 | Inference latency ("PsPing") | **SKIP** | oha already corrects for coordinated omission; guidellm already does rate-based and sweep load profiles. |
| 4 | Hidden-text scanner | **SKIP** (adopt existing) | Snyk Agent Scan, garak and AI-Infra-Guard already cover skills, MCP servers and ASCII smuggling, and 2026 has brought many small scanners. |
| 5 | Agent reach audit ("AccessChk for agents") | **BUILD** | The sandboxes *enforce* access; none was found that *reports* what an unsandboxed agent can reach. A check on a macOS test machine shows why it matters. |

Recommended start: **5, then 1, then 2.**

---

## 1. Model residency: "is my model in RAM?"

**Question it answers.** Before loading a local model, how much of its weight file is already in the page cache? That predicts cold start against warm start. Across all installed models (Ollama, LM Studio, Hugging Face cache), which are warm?

**Prior art**

| Project | Stars | Licence | Last push | Covers |
|---|---|---|---|---|
| [hoytech/vmtouch](https://github.com/hoytech/vmtouch) | 1,952 | BSD-3 | 2024-03-02 | Per-file residency, touch and evict; Unix-portable; Homebrew 1.3.1 |
| [tobert/pcstat](https://github.com/tobert/pcstat) | 1,308 | Apache-2.0 | 2024-07-25 | Per-file residency (Go) |
| [brk0v/cgtouch](https://github.com/brk0v/cgtouch) | 23 | – | 2025-10-08 | Per-cgroup page cache (Linux) |
| `ollama ps` | – | – | – | Models loaded into a running server, not the page cache |
| Linux `cachestat(2)` (6.5+) | – | – | – | Fast per-file cache statistics; [man page](https://man.archlinux.org/man/cachestat.2.en) |

**Gap.** All of these work per file. None knows where model files live, which blob belongs to which model name, or how residency relates to load time. GitHub searches for model and page-cache residency returned nothing.

**Feasibility, checked on a macOS test machine (Apple silicon, APFS) on 2026-11-19.** `mincore(2)` through stdlib `ctypes` on a memory-mapped 12.85 GiB Ollama blob returned rc=0 and 0 MiB resident. After reading the first 256 MiB, it reported 264 MiB resident (the extra is read-ahead). So the measurement works on APFS and responds to real cache state. On Linux, `cachestat` avoids building a per-page bitmap for large files.

**Risks**
- Unified memory on Apple silicon: once a model is loaded into the GPU, its weights may be held in wired or anonymous memory, not the page cache.
- The tool must report the model's state inside the running server (from `ollama ps` or the server API) separately from page-cache state, and not claim more than it measures.

**Verdict: BUILD (small).** A model-aware layer: discover model files, map blobs to model names, report residency and server state. Optionally warm or evict a model. Expect a few hundred lines, with no driver or root needed.

## 2. Persistence diff for agent runs: "what did the agent leave behind?"

**Question it answers.** After an agent session, did anything new get set up to run later: launch agents, cron entries, shell rc edits, git hooks, systemd user units, login items?

**Prior art**

| Project | Stars | Licence | Last push | Covers |
|---|---|---|---|---|
| [nolabs-ai/nono](https://github.com/nolabs-ai/nono) | 5,047 | Apache-2.0 | 2026-11-19 | Agent sandbox. [`--rollback`](https://nono.sh/docs/cli/features/atomic-rollbacks) snapshots *tracked directories* before and after, then diffs and restores |
| [osquery/osquery](https://github.com/osquery/osquery) | 23,785 | dual Apache-2.0 / GPL-2.0 (GitHub API reports NOASSERTION) | 2026-11-19 | Persistence tables (`launchd`, `crontab`, `startup_items`, systemd); heavyweight |
| [objective-see/KnockKnock](https://github.com/objective-see/KnockKnock) | 839 | GPL-3.0 | 2026-09-15 | macOS persistence scan; macOS only |
| [Pinperepette/MacPersistenceChecker](https://github.com/Pinperepette/MacPersistenceChecker) | 275 | MIT | 2026-06-26 | macOS persistence scan with signature checks |
| [Achiefs/fim](https://github.com/Achiefs/fim) | 209 | GPL-3.0 | 2026-06-17 | Generic file-integrity monitoring |

**Gap.** nono's rollback is the closest match, but it diffs the directories the agent is allowed to write. Persistence lives elsewhere: `~/Library/LaunchAgents`, `~/.zshrc`, crontab, `~/.config/systemd/user`, `.git/hooks`. Under nono's sandbox the agent typically can't write there at all. The gap is the common *unsandboxed* session, where persistence is exactly the risk. Searches found no tool that covers "persistence points, scoped to one agent run, macOS and Linux".

**Feasibility.** High for file-based persistence points: enumerate them, hash them, diff before and after. Most need no root.

**Risks**
- macOS Background Task Management (`sfltool dumpbtm`) asks for admin authentication.
- Reading other users' crontabs needs root.
- Both are optional extras, not blockers.

**Verdict: BUILD (narrow).** A before-and-after persistence diff that runs as a hook around an agent session. Also offer the list of persistence locations to nono as a "persistence profile" for its tracked paths. If they adopt it, our tool shrinks to a hook.

## 3. Inference latency: "PsPing for model endpoints"

**Prior art**

| Project | Stars | Licence | Last push | Covers |
|---|---|---|---|---|
| [hatoo/oha](https://github.com/hatoo/oha) | 10,698 | MIT | 2026-11-18 | HTTP load with `--latency-correction` for coordinated omission (README) |
| [vllm-project/guidellm](https://github.com/vllm-project/guidellm) | 1,796 | Apache-2.0 | 2026-11-19 | LLM-specific: synchronous, concurrent and rate-based modes, sweeps |
| [giltene/wrk2](https://github.com/giltene/wrk2) | 4,643 | Apache-2.0 | 2024-03-03 | Constant-throughput load with coordinated-omission correction |
| [Hyperfoil/Hyperfoil](https://github.com/Hyperfoil/Hyperfoil) | 164 | Apache-2.0 | 2026-11-19 | Load driver built around coordinated omission |
| [ray-project/llmperf](https://github.com/ray-project/llmperf) | 1,128 | Apache-2.0 | 2024-12-09 | **Archived** |

**Verdict: SKIP.** The two properties we would have offered, open-loop arrivals and coordinated-omission correction, already exist in maintained tools: oha for HTTP and guidellm for LLM serving. Use guidellm for inference benchmarks. The concept stays as a rule for agent tooling: report open-loop percentiles, never closed-loop averages.

## 4. Hidden-text scanner

**Prior art**

| Project | Stars | Licence | Last push | Covers |
|---|---|---|---|---|
| [invariantlabs-ai/mcp-scan](https://github.com/invariantlabs-ai/mcp-scan) (now Snyk Agent Scan) | 3,258 | Apache-2.0 | 2026-11-19 | Scans installed agent tools, MCP servers and skills for prompt injection and other threats |
| [NVIDIA/garak](https://github.com/NVIDIA/garak) | 10,101 | Apache-2.0 | 2026-11-18 | `probes/smuggling.py`: ASCII smuggling probes |
| [Tencent/AI-Infra-Guard](https://github.com/Tencent/AI-Infra-Guard) | 7,577 | Apache-2.0 | 2026-11-19 | Agent, skill and MCP scanning |
| [protectai/llm-guard](https://github.com/protectai/llm-guard) | 3,220 | MIT | 2026-07-08 | **Archived** |
| 2026 small tools: [amagi-sniff](https://github.com/amagilabs/amagi-sniff) (office documents and PDFs), [mcp-shield](https://github.com/ATH-hacker/mcp-shield), [inkless](https://github.com/rashedhasan090/inkless), [palisade](https://github.com/RootVandal/palisade) | under 5 each | mostly MIT | 2026 | Unicode tags, zero-width text, tool poisoning |

**Verdict: SKIP; adopt instead.** At least seven maintained projects cover this, three of them with more than 3,000 stars, and several more appeared in 2026. The useful action is integration: run Snyk Agent Scan on skills and MCP configuration before an agent session. The grounding (Greshake et al., AISec 2023, [doi:10.1145/3605764.3623985](https://doi.org/10.1145/3605764.3623985)) still applies to how we use it.

## 5. Agent reach audit: "what can my agent touch?"

**Question it answers.** Before starting an unsandboxed agent, what can it read, write and execute, given the identity it runs as and the permissions it inherits from its parent process?

**Prior art**

| Project | Stars | Licence | Last push | Covers |
|---|---|---|---|---|
| [anthropic-experimental/sandbox-runtime](https://github.com/anthropic-experimental/sandbox-runtime) | 7,188 | Apache-2.0 | 2026-11-13 | *Enforces* access (Seatbelt on macOS, bubblewrap on Linux); macOS violation monitoring |
| [nolabs-ai/nono](https://github.com/nolabs-ai/nono) | 5,047 | Apache-2.0 | 2026-11-19 | *Enforces* policy; tamper-evident audit of decisions |
| [peass-ng/PEASS-ng](https://github.com/peass-ng/PEASS-ng) | 20,856 | other | 2026-11-17 | Privilege-escalation enumeration (attacker view, not agent reach) |

**Gap.** The sandboxes answer "what is this agent *allowed* to do under my policy?" They report violations after the fact. None found answers, before launch and without a sandbox, "what would this agent be able to reach?" That is the question for the common unsandboxed default, and it is the evidence you need to write a sandbox policy in the first place.

**Feasibility, checked on a macOS test machine on 2026-11-19.** From a terminal app that had been granted Full Disk Access, `os.access(R_OK)` and a real directory listing both succeeded on `~/Library/Mail`, `~/Library/Messages`, `~/Library/Safari`, `~/Desktop` and the AddressBook folder. Only directory names were listed; no contents were read. Any agent started from such a terminal inherits read access to mail and messages. That result is the case for the tool.

**Risks**
- macOS privacy permissions (TCC) are granted per app and inherited by child processes, and `access(2)` may not reflect a TCC denial. The audit must probe with real read-only opens, attribute access to the responsible app (Terminal, iTerm, VS Code), and never write.
- On Linux, ordinary permissions and ACLs are straightforward. LSM confinement (AppArmor, SELinux, Landlock) must be reported as present or absent rather than modelled.
- Keep it to read-only probes of a curated list of sensitive locations (mail, messages, browser profiles, SSH and cloud credentials, password stores, shell history). Never walk the whole disk by default.

**Verdict: BUILD.** It has the clearest gap, a demonstrated real-world finding on the first probe, and it feeds directly into sandbox policy (sandbox-runtime, nono) rather than competing with it. Least-privilege grounding: Saltzer & Schroeder 1975, [doi:10.1109/PROC.1975.9939](https://doi.org/10.1109/PROC.1975.9939).

---

## Method and limits

- Prior-art search used GitHub repository search plus the most-starred projects in each space. Absence of evidence is limited to those searches; commercial products (for example endpoint security suites) were not surveyed.
- Star counts, licences and push dates come from the GitHub API on 2026-11-19. Feature claims come from each project's README or documentation, linked in place.
- Both feasibility checks ran on one macOS machine and are not yet repeated on Linux.
