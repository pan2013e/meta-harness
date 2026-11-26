# AGENTS.md — General Specifications & Engineering Contract

This document defines the mandatory architecture invariants, engineering standards, platform constraints, and verification gates for **any coding agent or developer** implementing **Meta-Harness**.

Read this file first. Consult [the technical specification](docs/SPECIFICATION.md) for worktree flows, protocol and storage references, and UI behavior. See [the README](README.md) for local development and release commands and [the changelog](CHANGELOG.md) for milestone history.

---

## 1. Mission & Aligned Product Decisions

Meta-Harness is a local-first workspace where a human developer and multiple AI agent harnesses (`@codex`, `@claude`, `@antigravity`, and custom personas such as `@codex-reviewer`) collaborate in project group chats and standalone Quick Sessions.

Every design decision must uphold these core specifications (finalized during design alignment):

1. **Per-Session Integration Branch + Per-Harness Git Worktrees**:
   - Git-backed sessions use a dedicated integration worktree and branch (`mh/<session_id>/integration`) and lazily created executor worktrees (`mh/<session_id>/exec-<handle>`). Keep the primary checkout untouched until explicit promotion with merge, squash, or patch. Non-Git folders offer Git initialization and use serialized shared-folder mode when isolation is unavailable. Quick Sessions initialize a managed repository when Git is available.
   - **Clean merges first.** The Git broker commits executor edits, integrates clean changes without an LLM call, and escalates conflicts to the orchestrator. Sync idle peer worktrees when configured.
2. **Adapter vs. Session Member Separation**:
   - Distinguish **`HarnessAdapter`** (CLI binary + ACP driver + **shared subscription quota bucket**, e.g., `codex`, `claude`, `antigravity`) from **`SessionMember`** (handle `@claude`, role `orchestrator` vs `executor`, optional persona prompt, dedicated worktree).
   - The harness brand chosen as **Orchestrator** (operating in the session integration worktree) can **also** spawn/dispatch coding tasks to an executor member of the same harness brand (in its own executor worktree), drawing from the same adapter quota pool, and users can add custom-named personas (`@codex-reviewer`).
   - Global personas are templates for session members. Changes to templates affect future copies. Harness profile selection controls CLI sign-in and resume scope per adapter.
3. **ACP Native + Parallel Network Switch**:
   - Harness turns use the **Agent Client Protocol (ACP)** over JSON-RPC 2.0 stdio with the ordinary CLI profile or the configured separate profile.
   - Harnesses run **in parallel** across their isolated worktrees and communicate via Meta-Harness's embedded single-binary MCP bridge (`meta-harness mcp-bridge` injected into `session/new` `mcpServers`):
     - **Async Group-Chat Messaging** (`@mention` / `chat_send_message`, queued if recipient is busy).
     - **Synchronous Mid-Turn Consults** (`peer_consult`), allowing an active harness to pause mid-turn and await an answer from an idle peer, protected by a **Wait-For Graph cycle detector** so circular consults never deadlock.
     - **Peer Trajectory Inspection** (`trajectory_list` / `trajectory_read`).
   - **UI Visibility**: Peer messages and consults appear in main Group Chat (with `peer_consult` styled as compact collapsible threads), while `trajectory_read` calls appear as subtle inline activity pills (`@claude read @codex's trajectory`).
4. **`@mention` Routing + Autonomous Quota-Aware Orchestrator**:
   - Explicit `@<handle>` messages (outside code blocks/spans) route directly to those members.
   - Unmentioned messages route to the available orchestrator, with a fallback to an available member. The orchestrator can answer directly or dispatch to available targets. Honor `confirm_before_dispatch` when set via the session API. The composer exposes `/plan` for planning requests and per-recipient model and effort choices.
   - Built-in adapters prefer live provider quota probes by default. Honor saved adapter preferences, shared quota pools, cooldowns, and manual calibration. Display unavailable quota windows when the provider supplies no usable data.
5. **Orchestrator-Curated High-Level Episodic Memory**:
   - Project-wide memory lives in **SQLite (`WAL`) + FTS5** (shared across sessions of the same project).
   - Curate high-level summaries for completed project work using the orchestrator when configured and available, with deterministic records as a fallback. Store job ownership, touched modules, outcomes, and harness/model notes.
6. **Worktree-Smart Permissions, Low Footprint & Optional Remote Access**:
   - Worktree is the default permission mode. Brokered reads from the project and writes inside the member worktree are allowed. Prompt for external file access and commands requiring approval, and support session-scoped allow patterns.
   - The standalone CLI serves `127.0.0.1:4410` by default, opens the browser on loopback, and provides tray controls for Windows and macOS releases. Electron runs the same backend on token-protected loopback. Non-loopback standalone binds require authentication.

---

## 2. Hard Architectural Invariants (Do Not Violate)

1. **No Raw Terminal Scraping for Agent Turns**: Speak structured JSON-RPC 2.0 via ACP (`initialize`, `session/new`, `session/load`, `session/set_config_option`, `session/prompt`, `session/cancel`, `session/update`, `session/request_permission`, `fs/*`, `terminal/*`).
2. **Loop, Hop & Deadlock Protection**:
   - Every user message initiates a `chain_id` with `max_agent_hops_per_user_turn` (default `6`). Every agent-triggered `@mention`, `chat_send_message`, or `peer_consult` decrements remaining hops; at `0`, pause routing and surface a UI confirmation banner.
   - Maintain an in-memory **Wait-For Graph** across active `peer_consult` calls: reject any consult edge `A -> B` if `B` transitively waits on `A` (returning a structured MCP tool error advising async `chat_send_message` instead).
   - Provide **Stop All** through `POST /api/sessions/{id}/cancel` to promptly signal cancellation to active ACP turns and pending `peer_consult` calls. Target under 250 ms for cancellation signaling.
3. **Mention Parsing Hygiene**:
   - Strip fenced code blocks (` ``` ... ``` `) and inline code spans (`` `...` ``) **before** extracting `@<handle>` tokens so `@decorator`, `@types/react`, or `name@domain.com` never trigger routing.
4. **Cross-Platform Process, Git & Path Discipline** (Windows, macOS & Linux):
   - Resolve executables cleanly via `which` across all OSes (handling `.cmd` / `.ps1` / `.exe` shims on Windows and executable binaries on macOS/Linux) before spawning subprocesses.
   - Isolate and clean up every spawned child process tree across all OSes: attach to a **Windows Job Object** (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) on Windows, spawn into a dedicated process group (`process_group(0)` + `killpg(SIGTERM -> SIGKILL)`) on macOS & Linux, and set `prctl(PR_SET_PDEATHSIG, SIGTERM)` on Linux so zero orphan agent/Node processes survive server exit.
   - Normalize paths with `dunce::canonicalize` (resolving macOS `/var -> /private/var` symlinks while avoiding raw `\\?\` verbatim prefixes on Windows that break `git` and Node CLIs).
   - Resolve per-OS configuration and state paths via `dirs`. Honor `MH_CONFIG_DIR`, `MH_DATA_DIR`, and `MH_WORKSPACE_DIR` for config, data, and Quick Session workspaces.
   - Prune session/member Git worktrees cleanly (`git worktree remove --force`) when a session is archived/deleted so `.git/worktrees` never accumulates stale locks.
5. **Single Binary MCP Bridge**:
   - Implement `meta-harness mcp-bridge` inside the main Rust binary (speaking stdio MCP JSON-RPC to the harness and forwarding over localhost HTTP with ephemeral `MH_BRIDGE_TOKEN` to `mh-server`). Zero Python or auxiliary Node scripts.
6. **Token-Conscious Delta Context**:
   - Track `last_seen_msg_seq` per `SessionMember`. Forward unseen group chat messages when reusing ACP state and a bounded recent transcript when starting fresh. Include compact summaries, persona instructions, and relevant telemetry.
7. **Session Files and Previews**:
   - Resolve files against the session's active workspace and enforce canonical path containment. HTML previews use a sandboxed frame and short-lived signed URLs scoped to read-only files for the session. Keep app authentication separate from preview grants.

---

## 3. Environment & Toolchain Bootstrap Notes (Windows, macOS & Linux)

Before starting Rust backend implementation on any host, verify Node.js 24, npm, Git, and a current stable Rust toolchain with `node -v`, `npm -v`, `git --version`, and `cargo --version`:

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
  - Install a C compiler (`sudo apt install build-essential` / `dnf groupinstall "Development Tools"`) and Rust (`rustup`). Preserve bundled SQLite through `rusqlite` and Rustls for HTTP TLS.

---

## 4. Code Organization & Module Boundaries

Keep crates decoupled with strict unidirectional dependencies:

```text
mh-core       (domain DTOs, mention parsing, wait-for graph, paths, configuration, token estimation)
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
│   ├── sidebar/  # Project picker, Quick Sessions, session list, and session actions
│   ├── chat/     # Virtualized message list, collapsible consult threads, trajectory-read pills, Dispatch & Permission cards, @mention composer
│   ├── drawer/   # Trajectory rendering and event grouping
│   ├── modals/   # Global settings, memory, diff review, folder picker, and file previews
│   └── common/   # Accessible UI primitives, keyboard shortcuts, status badges
└── types/        # Shared wire DTOs mirroring Rust serde types
```

The Electron launcher lives in `desktop/`. Build and version scripts live in `scripts/`.

---

## 5. Coding Standards & Quality Gates

### Rust (`crates/*`)
- Use Rust Edition **2024** and the current stable toolchain.
- `thiserror` in library crates; `anyhow` only at binary entrypoints (`mh-server`).
- Never `.unwrap()` / `.expect()` on runtime I/O, user input, Git CLI outputs, or third-party ACP JSON payloads.
- Coalesce ACP `session/update` streaming deltas (16–33ms batch window) before broadcasting over WebSocket and persisting to SQLite.
- **Pre-completion checks**:
  - `cargo fmt --all -- --check`
  - `cargo clippy --locked --workspace --all-targets -- -D warnings`
  - `cargo test --locked --workspace`

### Frontend (`web/*`)
- Strict TypeScript (`"strict": true`, zero `any` in domain models).
- Virtualize chat and trajectory lists (`@tanstack/react-virtual`) and memoize rows so streaming tokens across parallel agents sustain 60fps.
- **Pre-completion checks**:
  - `npm run typecheck` (`tsc --noEmit`)
  - `npm run test` (`vitest run`)
  - `npm run build`

### Offline Deterministic Integration Testing
- Use the deterministic mock in `crates/mh-acp/src/mock_agent.rs`. ACP tests launch the `mock-acp-agent` test binary. Server end-to-end tests use `meta-harness internal-mock-agent` and manual quotas to cover the switch without a provider subscription.

### Versions and releases

- Use `node scripts/version.mjs` to keep Rust, web, and Electron at the same semantic version. Run `node scripts/version.mjs --check` before committing a version change. Record significant changes in `CHANGELOG.md` and use `vMAJOR.MINOR.PATCH` tags for release commits.
- CI runs the verification gates on Windows, macOS, and Linux. The **Release build** workflow builds Windows and macOS packages through `node scripts/build.mjs`, including a CLI-only option.
