# AGENTS.md — General Specifications & Engineering Contract

This document defines the mandatory architecture invariants, engineering standards, platform constraints, and verification gates for **any coding agent or developer** implementing **Meta-Harness**.

Read this file first, then consult:
- `docs/SPECIFICATION.md` for exact wire protocols, Git worktree flows, data schemas, MCP tool definitions, and UI behavior.
- `docs/IMPLEMENTATION_PLAN.md` for milestone ordering, crate boundaries, and acceptance tests.

---

## 1. Mission & Aligned Product Decisions

Meta-Harness is a local-first web application where a human developer and multiple AI agent harnesses (`@codex`, `@claude`, `@antigravity`, and custom personas like `@codex-reviewer`) collaborate inside **project-scoped group chat sessions**.

Every design decision must uphold these core specifications (finalized during design alignment):

1. **Per-Session Integration Branch + Per-Harness Git Worktrees**:
   - Every session creates a dedicated **Session Integration Worktree & Branch** (`mh/s-<short_id>`) plus **isolated Git worktrees per executor member** (`mh/s-<short_id>/<handle>`), keeping the user's primary checked-out working tree untouched until explicit promotion (**Fast-Forward/Merge**, **Squash Merge**, or **Apply Patch**). Non-Git folders offer 1-click `git init` (with fallback to serialized shared-folder mode).
   - **Zero-Token Clean Merge First**: When an executor finishes a turn with edits, the Rust Git Broker auto-commits uncommitted worktree changes and cleanly merges into the session integration branch (`mh/s-<short_id>`) in Rust without spending LLM tokens—waking the **Orchestrator Harness** only when merge conflicts occur or multi-executor synthesis is needed. Idle peer worktrees fast-forward to stay synced.
2. **Adapter vs. Session Member Separation**:
   - Distinguish **`HarnessAdapter`** (CLI binary + ACP driver + **shared subscription quota bucket**, e.g., `codex`, `claude`, `antigravity`) from **`SessionMember`** (handle `@claude`, role `orchestrator` vs `executor`, optional persona prompt, dedicated worktree).
   - The harness brand chosen as **Orchestrator** (operating in the session integration worktree) can **also** spawn/dispatch coding tasks to an executor member of the same harness brand (in its own executor worktree), drawing from the same adapter quota pool, and users can add custom-named personas (`@codex-reviewer`).
3. **ACP Native + Parallel Network Switch**:
   - All harness communication uses the **Agent Client Protocol (ACP)** over JSON-RPC 2.0 stdio, inheriting the user's existing CLI subscriptions (`~/.codex`, `~/.claude`, `~/.gemini`).
   - Harnesses run **in parallel** across their isolated worktrees and communicate via Meta-Harness's embedded single-binary MCP bridge (`meta-harness mcp-bridge` injected into `session/new` `mcpServers`):
     - **Async Group-Chat Messaging** (`@mention` / `chat_send_message`, queued if recipient is busy).
     - **Synchronous Mid-Turn Consults** (`peer_consult`), allowing an active harness to pause mid-turn and await an answer from an idle peer, protected by a **Wait-For Graph cycle detector** so circular consults never deadlock.
     - **Peer Trajectory Inspection** (`trajectory_list` / `trajectory_read`).
   - **UI Visibility**: Peer messages and consults appear in main Group Chat (with `peer_consult` styled as compact collapsible threads), while `trajectory_read` calls appear as subtle inline activity pills (`@claude read @codex's trajectory`).
4. **`@mention` Routing + Autonomous Quota-Aware Orchestrator**:
   - Explicit `@<handle>` messages (outside code blocks/spans) route directly to those members.
   - Unmentioned messages route to the session's **Orchestrator Harness**, which auto-dispatches coding tasks by default (rendering a live Dispatch Card with cancel/reroute controls), answers trivial questions or handles integration merges directly without wasting an extra hop, and supports an optional `"Confirm Plan First"` toggle.
   - Subscription usage lookup uses **safe local telemetry by default** (local CLI session logs in `~/.codex`, `~/.claude`, `~/.gemini` + ACP status metadata + Meta-Harness ledger + reactive `429` cooldowns calibrated to user plan tier), with an **opt-in toggle** for live endpoint / localhost rate-limit header-tap exact percentages.
5. **Orchestrator-Curated High-Level Episodic Memory**:
   - Project-wide memory lives in **SQLite (`WAL`) + FTS5** (shared across sessions of the same project).
   - After every completed job, the **Orchestrator** runs a lightweight, low-effort LLM summarization pass (folded into its merge/synthesis turn whenever one is already active!) to curate *who did what job, touched modules, outcomes, and harness/model notes*—never raw code chunks or vector-DB embeddings.
6. **Worktree-Smart Permissions, Low Footprint & Optional Remote Access**:
   - **Permissions**: By default, auto-allow file reads/writes strictly *inside* the harness's assigned Git worktree; prompt in Group Chat for shell commands (`terminal/*`) and outside-worktree paths, with 1-click **"Allow command pattern for session"** rules (`cargo *`, `npm test`).
   - **Hosting**: Single Rust binary serving `127.0.0.1:4410` in the user's browser by default (<30 MB idle server RSS), plus optional `--bind` + bearer-token auth mode for LAN / Tailscale access from laptop or mobile.

---

## 2. Hard Architectural Invariants (Do Not Violate)

1. **No Raw Terminal Scraping for Agent Turns**: Speak structured JSON-RPC 2.0 via ACP (`initialize`, `session/new`, `session/load`, `session/set_config_option`, `session/prompt`, `session/cancel`, `session/update`, `session/request_permission`, `fs/*`, `terminal/*`).
2. **Loop, Hop & Deadlock Protection**:
   - Every user message initiates a `chain_id` with `max_agent_hops_per_user_turn` (default `6`). Every agent-triggered `@mention`, `chat_send_message`, or `peer_consult` decrements remaining hops; at `0`, pause routing and surface a UI confirmation banner.
   - Maintain an in-memory **Wait-For Graph** across active `peer_consult` calls: reject any consult edge `A -> B` if `B` transitively waits on `A` (returning a structured MCP tool error advising async `chat_send_message` instead).
   - Provide an immediate **Stop All (`session/cancel`)** endpoint/button that aborts all active ACP turns and pending `peer_consult` futures within <250ms.
3. **Mention Parsing Hygiene**:
   - Strip fenced code blocks (` ``` ... ``` `) and inline code spans (`` `...` ``) **before** extracting `@<handle>` tokens so `@decorator`, `@types/react`, or `name@domain.com` never trigger routing.
4. **Cross-Platform Process, Git & Path Discipline** (Windows, macOS & Linux):
   - Resolve executables cleanly via `which` across all OSes (handling `.cmd` / `.ps1` / `.exe` shims on Windows and executable binaries on macOS/Linux) before spawning subprocesses.
   - Isolate and clean up every spawned child process tree across all OSes: attach to a **Windows Job Object** (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) on Windows, spawn into a dedicated process group (`process_group(0)` + `killpg(SIGTERM -> SIGKILL)`) on macOS & Linux, and set `prctl(PR_SET_PDEATHSIG, SIGTERM)` on Linux so zero orphan agent/Node processes survive server exit.
   - Normalize paths with `dunce::canonicalize` (resolving macOS `/var -> /private/var` symlinks while avoiding raw `\\?\` verbatim prefixes on Windows that break `git` and Node CLIs).
   - Resolve per-OS configuration and state paths cleanly via `dirs` (`%APPDATA%` / `%LOCALAPPDATA%` on Windows, `~/Library/Application Support` on macOS, `$XDG_CONFIG_HOME` / `$XDG_DATA_HOME` on Linux), with `MH_CONFIG_DIR` and `MH_DATA_DIR` overrides.
   - Prune session/member Git worktrees cleanly (`git worktree remove --force`) when a session is archived/deleted so `.git/worktrees` never accumulates stale locks.
5. **Single Binary MCP Bridge**:
   - Implement `meta-harness mcp-bridge` inside the main Rust binary (speaking stdio MCP JSON-RPC to the harness and forwarding over localhost HTTP with ephemeral `MH_BRIDGE_TOKEN` to `mh-server`). Zero Python or auxiliary Node scripts.
6. **Token-Conscious Delta Context**:
   - Track `last_seen_msg_seq` per `SessionMember` and forward **only unseen group-chat messages** plus compact memory/telemetry headers on each `session/prompt`.

---

## 3. Environment & Toolchain Bootstrap Notes (Windows, macOS & Linux)

Before starting Rust backend implementation on any host, verify `node -v`, `npm -v`, `git --version`, and `cargo --version`:

- **Windows** (when `cargo` / Visual Studio C++ Build Tools `cl.exe` are absent):
  - Install Rustup **plus** a user-space GCC MinGW-w64 toolchain (`WinLibs`, providing `gcc.exe`, `libgcc`, and `libgcc_eh` for `rusqlite`'s `bundled` SQLite compilation):
    ```powershell
    winget install --id Rustlang.Rustup -e --accept-package-agreements --accept-source-agreements
    winget install --id BrechtSanders.WinLibs.POSIX.MSVCRT -e --accept-package-agreements --accept-source-agreements
    rustup toolchain install stable-x86_64-pc-windows-gnu
    rustup default stable-x86_64-pc-windows-gnu
    ```
    *(If Visual Studio C++ Build Tools are already installed, `stable-x86_64-pc-windows-msvc` works equally well.)*
- **macOS**:
  - Install Xcode Command Line Tools (`xcode-select --install`) and Rust (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`).
- **Linux**:
  - Install a C compiler (`sudo apt install build-essential` / `dnf groupinstall "Development Tools"`) and Rust (`rustup`). Always enable `features = ["bundled"]` on `rusqlite` and `rustls` (never `openssl-sys` / `native-tls`) so builds stay self-contained across all OSes.

---

## 4. Code Organization & Module Boundaries

Keep crates decoupled with strict unidirectional dependencies:

```text
mh-core       (domain DTOs, @mention parser, Wait-For DAG cycle detector, quota/complexity models; zero I/O)
  ▲
mh-store      (SQLite + FTS5 schema, migrations, sessions, members, messages, trajectories, episodic memory)
  ▲
mh-acp        (ACP stdio supervisor, Git worktree broker, worktree-smart permissions, multi-tier quota probes)
  ▲
mh-switch     (parallel session router, hop guard, peer_consult RPC, orchestrator + memory curator, MCP handlers)
  ▲
mh-server     (Axum REST + WS hub, optional LAN bearer auth, static asset serving, CLI & `mcp-bridge` subcommand)
```

Frontend (`web/`):
```text
web/src/
├── api/          # Typed REST client, auth token handler & multiplexed WS client with auto-reconnect
├── store/        # Normalized Zustand stores (projects, activeSession, chatStream, worktreeDiff, trajectories, quotas, memory)
├── components/
│   ├── sidebar/  # Project picker, session list, member/persona roster +Orchestrator badge + live quota meters
│   ├── chat/     # Virtualized message list, collapsible consult threads, trajectory-read pills, Dispatch & Permission cards, @mention composer
│   ├── gitbar/   # "Session Changes" status bar, cumulative diff modal, integration terminal/preview launcher, 3-mode Promote menu
│   ├── drawer/   # Trajectory inspector (thoughts, tool calls, diffs), Curated Memory ledger, Switch & Quota settings
│   └── common/   # Accessible UI primitives, keyboard shortcuts, status badges
└── types/        # Shared wire DTOs mirroring Rust serde types
```

---

## 5. Coding Standards & Quality Gates

### Rust (`crates/*`)
- Rust Edition **2024** (or **2021** if toolchain <1.85).
- `thiserror` in library crates; `anyhow` only at binary entrypoints (`mh-server`).
- Never `.unwrap()` / `.expect()` on runtime I/O, user input, Git CLI outputs, or third-party ACP JSON payloads.
- Coalesce ACP `session/update` streaming deltas (16–33ms batch window) before broadcasting over WebSocket and persisting to SQLite.
- **Pre-completion checks**:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`

### Frontend (`web/*`)
- Strict TypeScript (`"strict": true`, zero `any` in domain models).
- Virtualize chat and trajectory lists (`@tanstack/react-virtual`) and memoize rows so streaming tokens across parallel agents sustain 60fps.
- **Pre-completion checks**:
  - `npm run typecheck` (`tsc --noEmit`)
  - `npm run test` (`vitest run`)
  - `npm run build`

### Offline Deterministic Integration Testing
- Maintain a deterministic Rust **Mock ACP Agent** (`crates/mh-acp/tests/fixtures/mock_acp_agent.rs`) so CI and local tests verify parallel worktree execution, clean Rust auto-merges, conflict escalation to Orchestrator, `peer_consult` deadlock prevention, `trajectory_read`, hop limits, Orchestrator `session/set_config_option` calls, and memory curation **without consuming real AI subscriptions**.
