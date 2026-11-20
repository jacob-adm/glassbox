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

---

# Second round: trigger capture and fault injection

Two further candidates from `docs/suite-study.md`, checked the same way on 2026-11-19.

| Candidate | Inspired by | Verdict | One-line reason |
|---|---|---|---|
| Trigger capture | ProcDump | **BUILD (narrow)** | Observability platforms record model and tool calls; none was found that snapshots *machine state* when a local agent trips a trigger. Signals verified on real transcripts. |
| Fault injection | NotMyFault | **SKIP; adopt** | Model-API faults (llmposter, MockServer) and MCP faults (agentchaos, AgentBuster, MCP-Injector) are already covered, with peer-reviewed studies behind them. |

## Trigger capture: "what was the machine doing when the agent went wrong?"

**Question it answers.** When an agent loops, burns tokens or keeps failing, capture the state of the machine at that moment, next to the agent's own record. ProcDump does this for a process: dump on a trigger (CPU, memory, exception).

**Prior art**

| Project | Stars | Licence | Last push | Covers |
|---|---|---|---|---|
| [langfuse/langfuse](https://github.com/langfuse/langfuse) | 37,635 | other | 2026-11-19 | Tracing of model and tool calls through an SDK; hosted or self-hosted |
| [Arize-ai/phoenix](https://github.com/Arize-ai/phoenix) | 12,356 | other | 2026-11-19 | Tracing and evaluation |
| [traceloop/openllmetry](https://github.com/traceloop/openllmetry) | 7,544 | Apache-2.0 | 2026-11-19 | OpenTelemetry instrumentation for LLM calls |
| [AgentOps-AI/agentops](https://github.com/AgentOps-AI/agentops) | 5,965 | MIT | 2026-06-25 | Agent session recording through an SDK |
| [liuchunwei732-cmyk/loopbuster](https://github.com/liuchunwei732-cmyk/loopbuster) | 83 | MIT | 2026-06-05 | Loop detection and budget ceilings inside an agent framework |
| [joe-re/eyes-on-claude-code](https://github.com/joe-re/eyes-on-claude-code) and similar | 27 | MIT | 2026-11-18 | Live session status displays |

The [OpenTelemetry GenAI conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/) define agent, model-call and tool spans with token counts, so the *agent side* of a trace is a solved, standard problem.

**Gap.** Everything found records the agent's own calls, needs an SDK or a framework, and stops at the model boundary. None captures what the machine looked like when a trigger fired: persistence added since the session began (`glassbox persist`), the agent's process tree, and the working tree's uncommitted changes. None works from the transcripts command-line agents already write locally.

**Feasibility, checked on a macOS test machine on 2026-11-19.** Claude Code writes each session as JSON lines. Three recent sessions held 9,856 records with per-message token usage (`input_tokens`, `output_tokens`, cache fields) and 1,309 `tool_use` blocks. Fingerprinting each call (tool name plus a hash of its arguments, no content read) gave this baseline from real work:

| Session | Tool calls | Distinct | Most repeats of one identical call | Longest identical run | Output tokens per turn (median / p99 / max) |
|---|---|---|---|---|---|
| 1 | 17 | 9 | 4 | 1 | 4,278 / 7,726 / 7,726 |
| 2 | 820 | 785 | 3 | 2 | 550 / 4,281 / 9,821 |
| 3 | 472 | 438 | 5 | 2 | 851 / 6,108 / 8,661 |

A trigger of four identical calls in a row would have fired zero times on this real work, so loop detection from local transcripts is feasible with a low false-alarm rate. No Codex sessions were present on the test machine, so Codex support is unverified.

**Risks**
- The transcript format is not a documented interface and can change between agent versions; the reader must fail loudly on an unknown format, not guess.
- Thresholds from three sessions are a starting point, not a calibration; they need to be configurable and reported with the trigger.
- Capturing must never read transcript content into the report beyond what is needed to name the trigger, because transcripts can hold secrets.

**Verdict: BUILD (narrow).** `glassbox watch`: follow a local agent session; on a loop, token spike or run of failed tool calls, write a capture bundle with the trigger, the persistence diff since the session began, the agent's process tree and the working tree's change summary; optionally pause the agent (SIGSTOP, as Sysinternals PsSuspend does) so a person can look before it continues.

## Fault injection: "does the agent tooling survive a broken model or tool?"

**Prior art**

| Project | Status (2026-11-19) | Covers |
|---|---|---|
| [llmposter](https://crates.io/crates/llmposter) ([repo](https://github.com/SkillDoAI/llmposter)) | 0.5.0, about 1,030 downloads, updated 2026-07-07 | Mock OpenAI, Anthropic and Gemini APIs with seeded failures: 429 and 529 status codes, streaming delays, duplicated and truncated frames |
| [MockServer `LlmChaosProfile`](https://mock-server.com/versions/7.1.0/apidocs/org/mockserver/model/LlmChaosProfile.html) | in MockServer 7.1.0 | Probabilistic provider errors with Retry-After, mid-stream truncation, malformed SSE chunks |
| [dta-au/errorworks](https://github.com/dta-au/errorworks) (ChaosLLM) | 0 stars, pushed 2026-10-11 | Fake OpenAI-compatible server with rate limits, timeouts and malformed JSON |
| [seanrobmerriam/agentchaos](https://github.com/seanrobmerriam/agentchaos) | 1 star, pushed 2026-07-20 | MCP proxy: drop, duplicate, reorder, kill and corrupt messages |
| [yuchengw001-hub/AgentBuster](https://github.com/yuchengw001-hub/AgentBuster) | 2 stars, early access | Local agent testing with MCP failures |
| [Shopify/toxiproxy](https://github.com/Shopify/toxiproxy) | 12,541 stars, MIT | Network-level latency, timeouts and resets in front of any endpoint |

**Peer-reviewed and preprint grounding.** ReliabilityBench ([arXiv:2601.06112](https://arxiv.org/abs/2601.06112)) injects timeouts, rate limits, partial responses and schema drift and reports that single-run pass rates overstate reliability by 20 to 40 percent. *Loud Failures, Quiet Failures* ([arXiv:2610.10062](https://arxiv.org/abs/2610.10062)) injects typed tool faults into multi-turn tool use. *When Tools Fail, Agents Fabricate* ([NeurIPS 2026](https://neurips.cc/virtual/2026/173038)) reports 27 to 54 percent correct recovery and 19 to 30 percent confident fabrication across 2,100 fault-fired runs. *The Scaling Paradox of Tool-Calling LLM Agents Under Realistic MCP Faults* ([NeurIPS 2026](https://neurips.cc/virtual/2026/poster/151564)) introduces MCP-Injector for permanent, transient and intermittent MCP faults.

**Verdict: SKIP; adopt.** Both layers already have tools, and the research shows how to use them. Use llmposter or MockServer for model-API faults and an MCP fault proxy for tool faults when testing agent tooling. One narrow gap remains in the search results (rewriting tool-call arguments in model responses), which ChaosLLM's malformed-JSON mode partly covers; it does not justify a new tool.

---

# Follow-up: GPU memory for `models`

Checked on 2026-11-20. The first round's verdict for model residency included reporting "server state". After `models` shipped with page-cache residency, the remaining question was whether glassbox should also report a loaded model's GPU memory.

| Source | What it exposes | Already shown by |
|---|---|---|
| Ollama `GET /api/ps` ([API docs](https://github.com/ollama/ollama/blob/main/docs/api.md)) | Each loaded model's `size` and `size_vram` | `ollama ps`, whose PROCESSOR column gives the GPU and CPU split |
| LM Studio `GET /api/v0/models` ([REST docs](https://lmstudio.ai/docs/developer/rest/endpoints)) | `state` per model; no size or memory field | `lms ps` |
| macOS `IOAccelerator` registry (`ioreg -r -c IOAccelerator`) | GPU memory for the whole system (`In use system memory`) | Activity Monitor; not attributable to a model |

On Apple silicon, the GPU memory used by one model in one process is not available without private interfaces or root. The per-model figures that do exist come from each server's own API, and each server's CLI already prints them.

**Verdict: SKIP.** Reporting GPU memory would re-display `ollama ps` and `lms ps`. `models` keeps measuring the page cache, which no server reports, and its documentation points to the server's own status for loaded models.

---

# Third round: the remaining agent-tooling concepts

The six Track B concepts from `docs/suite-study.md` not checked in earlier rounds, checked on 2026-11-20. Stars, licences and last push dates come from the GitHub API that day. Feature claims come from each project's README or documentation, linked in place.

| Concept | Inspired by | Verdict | One-line reason |
|---|---|---|---|
| Egress audit | TCPView | **EXTEND (narrow)** | Network monitors exist, but none found lists an agent's connections without root and keeps them with the rest of a capture. `watch` can record the process tree's open connections when it fires. |
| Provenance check | Sigcheck | **SKIP; adopt** | OpenSSF model signing covers weights. Agent Scan checks skill and MCP content. `persist` already detects skill and config changes between sessions. |
| Least-privilege identity | AccessChk, ShellRunas | **SKIP; adopt** | SandVault and Hazmat run agents as a dedicated macOS user. `reach` already measures whichever user runs it. |
| Staged actions | MoveFile, PendMoves | **SKIP** | Claude Code's default mode asks before each edit, and checkpoints undo edits. `persist --since` reviews what changed outside the project. |
| Empirical limits | Testlimit | **SKIP** | The Anthropic API returns the account's actual limits in every response. Probing would spend quota to learn the same numbers. |
| Action trace with a stable schema | Procmon, Sysmon | **SKIP; adopt** | OpenTelemetry's GenAI conventions define agent and tool spans, and Claude Code exports tool events over OpenTelemetry. |

## Egress audit: "where did the agent connect?"

| Project | Stars | Licence | Last push | Covers |
|---|---|---|---|---|
| [domcyrus/rustnet](https://github.com/domcyrus/rustnet) | 5,247 | Apache-2.0 | 2026-11-19 | Live TCP, UDP and QUIC connections with process attribution; headless JSON snapshots; PCAP export. Packet capture needs platform permissions (macOS PKTAP and BPF access, Linux capabilities) |
| [objective-see/LuLu](https://github.com/objective-see/LuLu) | 13,626 | GPL-3.0 | 2026-11-02 | macOS per-application firewall: alerts on and blocks outgoing connections |
| [anthropics/sandbox-runtime](https://github.com/anthropics/sandbox-runtime) | 7,194 | Apache-2.0 | 2026-11-13 | Domain allowlist enforced around an agent process |

**Gap.** Monitors and firewalls cover the machine or an application. None found records the connections of one agent session's process tree without elevated permissions and keeps them alongside the session's other evidence.

**Feasibility, checked on a macOS test machine on 2026-11-20.** As a normal user, `lsof -nP -a -i -p <pid>` listed a running Claude Code process's TCP connections (remote address, port and state). On Linux, `/proc/<pid>/fd` socket inodes joined with `/proc/net/tcp` and `/proc/net/tcp6` give the same for the user's own processes, with no extra tools.

**Limits.** A capture is a point-in-time list. Connections that opened and closed between captures are missed, so the result is evidence, not an audit trail. Use rustnet, LuLu or sandbox-runtime when every connection must be recorded or controlled.

**Verdict: EXTEND (narrow).** Add the open connections of the `--pid` process tree to `watch` captures, labelled as a point-in-time list. No new command.

## Provenance check: "is this the model or skill I approved?"

| Project | Stars | Licence | Last push | Covers |
|---|---|---|---|---|
| [sigstore/model-transparency](https://github.com/sigstore/model-transparency) | 249 | Apache-2.0 | 2026-11-09 | `model_signing sign` and `verify` for model directories: Sigstore keyless, PKI or bare keys (OpenSSF Model Signing) |
| [snyk/agent-scan](https://github.com/snyk/agent-scan) | 3,262 | Apache-2.0 | 2026-11-19 | Discovers installed agents, MCP servers and skills; scans them for prompt injection, data handling and malware patterns |
| [GautamTalksDev/mcp-pin](https://github.com/GautamTalksDev/mcp-pin) | 4 | MIT | 2026-11-19 | Pins MCP tool definitions by hash and blocks changes; very new |

There is no signing scheme for agent skills to verify against. The useful check, "did this change since I approved it", is what `persist --save` and `persist --since` already do for skills, agent instructions and MCP configuration.

**Verdict: SKIP; adopt.** Use `model_signing verify` for weights, Agent Scan for skill and MCP content, and `persist` for change detection.

## Least-privilege identity: "the agent runs as its own user"

| Project | Stars | Licence | Last push | Covers |
|---|---|---|---|---|
| [webcoyote/sandvault](https://github.com/webcoyote/sandvault) | 456 | Apache-2.0 | 2026-11-12 | Runs agents in a limited macOS user account with a shared workspace; includes Claude Code, Codex, Gemini and others |
| [dredozubov/hazmat](https://github.com/dredozubov/hazmat) | 180 | MIT | 2026-11-14 | Dedicated agent user, per-session Seatbelt policy, `pf` firewall rules and pre-session backups |
| [nolabs-ai/nono](https://github.com/nolabs-ai/nono) | 5,052 | Apache-2.0 | 2026-11-19 | Same-user sandbox with rollback (covered in round 1) |

`reach` probes as the user that runs it. Run it as the agent's user, for example `sudo -u <agent-user> glassbox reach --home <agent-home>`, to audit that account. This was not tested here because no dedicated agent user exists on the test machine.

**Verdict: SKIP; adopt.** Use SandVault or Hazmat for the identity, and `reach` to audit it.

## Staged actions: "queue changes, apply after approval"

Claude Code's [default permission mode](https://code.claude.com/docs/en/permission-modes) asks before most file edits, shell commands and network actions. [Checkpointing](https://code.claude.com/docs/en/agent-sdk/file-checkpointing) snapshots files before each edit, and `/rewind` restores them, though not for changes made by shell commands or to remote systems. Git worktrees isolate whole sessions. Several projects add review-before-write gates of their own, but none is maintained widely enough to cite as a standard.

**Verdict: SKIP.** Staging is a property of the agent's edit loop, and the agent already provides it. For changes outside the project, which checkpoints do not cover, `persist --since` lists what was added or changed.

## Empirical limits: "probe the real limits instead of trusting the documented ones"

| Source | What it gives |
|---|---|
| Anthropic API [rate limit headers](https://platform.claude.com/docs/en/api/rate-limits) | `anthropic-ratelimit-requests-*` and `anthropic-ratelimit-tokens-*` (`-limit`, `-remaining`, `-reset`) on every response, plus `retry-after` on a 429 |
| [ivanfioravanti/llm_context_benchmarks](https://github.com/ivanfioravanti/llm_context_benchmarks) (107 stars, Apache-2.0, 2026-10-15) | Throughput at increasing context sizes for local and OpenAI-compatible servers |

**Verdict: SKIP.** For hosted APIs, the response headers already report the account's actual limits, so probing would spend quota to rediscover them. For local servers, the context size is a load-time setting, and throughput at each size is already benchmarked.

## Action trace with a stable schema

| Source | What it gives |
|---|---|
| [open-telemetry/semantic-conventions-genai](https://github.com/open-telemetry/semantic-conventions-genai) (554 stars, Apache-2.0, 2026-11-20) | `invoke_agent` and `execute_tool` spans with `gen_ai.tool.name`, `gen_ai.tool.call.id` and agent attributes; MCP conventions. Earlier versions live in [open-telemetry/semantic-conventions](https://github.com/open-telemetry/semantic-conventions) (690 stars, Apache-2.0, 2026-11-20) |
| Claude Code [monitoring](https://code.claude.com/docs/en/monitoring-usage) | OpenTelemetry export, including a `claude_code.tool_result` event per tool call (tool name, tool use ID, success, duration). Arguments such as commands and paths only with `OTEL_LOG_TOOL_DETAILS` |

**Verdict: SKIP; adopt.** A new schema would compete with the one the ecosystem has settled on. Use OpenTelemetry export where an action trace is needed. `watch` stays a trigger on the transcript and does not become a tracer.

## Method and limits (third round)

- Prior-art search used web search and GitHub, then each verdict-deciding claim was checked against the project's own README or the vendor's documentation.
- Searches found no tool that probes provider limits by load, and no maintained, widely used review-before-write gate. Absence of evidence is limited to those searches.
- Only the egress feasibility check was run, and only on macOS. The Linux path is from the `proc(5)` interface, not yet exercised.

---

# Fourth round: inference placement and benchmark hygiene

The two Track C concepts from `docs/suite-study.md` not checked in earlier rounds, checked on 2026-11-20. Stars, licences and last push dates come from the GitHub API that day. llama.cpp claims come from its documentation and source at commit `374d90a9`.

| Concept | Inspired by | Verdict | One-line reason |
|---|---|---|---|
| Placement | Coreinfo | **SKIP** | macOS shows the core layout through `sysctl` but does not let a program pin threads to cores. Linux has `lscpu`, `taskset` and `numactl`, and llama.cpp takes a CPU mask and a NUMA mode itself. |
| Benchmark hygiene | CPUSTRES, Clockres | **SKIP; adopt** | `llama-bench` repeats each test and reports the mean and standard deviation. hyperfine flags outliers, and stress-ng supplies controlled background load. Timer resolution is far below token latency. |

## Placement: "run inference threads on the right cores"

| Platform | See the layout | Control placement |
|---|---|---|
| macOS | `sysctl hw.nperflevels hw.perflevel0.name hw.perflevel0.logicalcpu` (and `perflevel1`) lists performance and efficiency cores with their caches | No thread pinning. `taskpolicy -c` and `-b` only lower a process's scheduling class; the scheduler chooses the cores |
| Linux | `lscpu`, and `lstopo` from [open-mpi/hwloc](https://github.com/open-mpi/hwloc) (746 stars, BSD-3-Clause, 2026-11-20) | `taskset -c`, `numactl --cpunodebind --membind` |
| llama.cpp | `llama-bench` records the CPU in `cpu_info` | [`llama-server`](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md) and [`llama-bench`](https://github.com/ggml-org/llama.cpp/blob/master/tools/llama-bench/README.md) take `-C/--cpu-mask`, `--cpu-strict`, `--prio` and `--numa distribute\|isolate\|numactl` |

**Checked on a macOS test machine (Apple silicon, performance and efficiency cores).** `sysctl` reported both core types with their L2 cache sizes. `thread_policy_set` with `THREAD_AFFINITY_POLICY` returned `KERN_NOT_SUPPORTED` (46). The SDK header calls that policy experimental and a hint even where it works. llama.cpp's CPU backend agrees: its Apple implementation of `ggml_thread_apply_affinity` does nothing and is commented "Not supported on Apple platforms", so `--cpu-mask` has no effect on macOS.

**Checked on Linux (Debian 11 container).** `lscpu` printed the topology, and `taskset -c 0` restricted the process to CPU 0, as shown by `Cpus_allowed_list` in `/proc/self/status`. `numactl` and `hwloc` are separate packages.

**Verdict: SKIP.** On Linux, existing commands show the layout and control placement, and llama.cpp applies a mask itself. On macOS, a program cannot pin threads to cores, so there is nothing for a tool to control, and `sysctl` already shows the layout.

## Benchmark hygiene: "does the number hold when the machine is busy?"

| Project | Stars | Licence | Last push | Covers |
|---|---|---|---|---|
| [ggml-org/llama.cpp](https://github.com/ggml-org/llama.cpp) `llama-bench` | 135,842 | MIT | 2026-11-20 | `-r` repetitions (default 5), mean and standard deviation per test, each sample in JSON output, `--delay` between tests, `--prio` |
| [sharkdp/hyperfine](https://github.com/sharkdp/hyperfine) | 29,307 | Apache-2.0 | 2026-11-01 | Repeated runs with statistics, warmup and prepare commands, and outlier detection that warns of interference from other programs and caching |
| [ColinIanKing/stress-ng](https://github.com/ColinIanKing/stress-ng) | 2,827 | GPL-2.0 | 2026-11-20 | Controlled CPU, memory and I/O load; builds on Linux and macOS |
| [vllm-project/guidellm](https://github.com/vllm-project/guidellm) | 1,799 | Apache-2.0 | 2026-11-20 | Load testing of model servers (covered in round 1) |

To check that a result holds under load, run `llama-bench` or `guidellm` once on an idle machine and once beside `stress-ng --cpu N`, and compare the means against their standard deviations.

**Timer resolution, checked on the same macOS machine.** `mach_timebase_info` is 125/3, and the smallest observed step of `mach_absolute_time` was 41 ns. `clock_getres` reported 42 ns for `CLOCK_UPTIME_RAW` and 1 µs for `CLOCK_MONOTONIC`. Token latencies are measured in milliseconds, at least a thousand times coarser, so timer granularity does not limit an inference benchmark. Clockres's concern does not carry over.

**Verdict: SKIP; adopt.** Use `llama-bench` or `guidellm` for repeated measurements with spread, hyperfine for command-level timing, and stress-ng for background load.

## Method and limits (fourth round)

- macOS checks ran on one Apple silicon machine. Intel Macs were not tested.
- The Linux check ran in a container with one NUMA node, so `numactl` binding was not exercised on multi-node hardware.
- Placement's effect on inference speed was not measured, because neither verdict depends on it.
