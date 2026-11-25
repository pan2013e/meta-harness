# AGENTS.md — General Specifications & Engineering Contract

This document defines the mandatory architecture invariants, engineering standards, platform constraints, and verification gates for **any coding agent or developer** implementing **Meta-Harness**.

Read this file first, then consult:
- `docs/SPECIFICATION.md` for exact wire protocols, data schemas, MCP tool definitions, and UI behavior.
- `docs/IMPLEMENTATION_PLAN.md` for milestone ordering, crate boundaries, and acceptance tests.

---

## 1. Mission & Product Pillars

Meta-Harness is a local-first web application where a human developer and multiple AI agent harnesses (`@codex`, `@claude`, `@antigravity`, etc.) collaborate inside **project-scoped group chat sessions**.

Every design decision must uphold these six pillars:

1. **Project-Bound Sessions**: Every session belongs to a concrete local project directory (`cwd`). File operations and terminal commands initiated by harnesses are sandboxed to that project root unless explicitly granted by the user.
2. **Group-Chat UX with Trajectory Clarity**: Main chat stays clean and conversational (user turns, `@mention` messages, orchestrator dispatch cards, permission cards, final/milestone agent replies). Low-level reasoning, tool calls, terminal streams, and diffs live in structured **Trajectories** viewable in a side drawer or queryable by peer harnesses.
3. **ACP Native + Network Switch**: All harness communication uses the **Agent Client Protocol (ACP)** over JSON-RPC 2.0 stdio, inheriting the user's existing CLI subscriptions (`~/.codex`, `~/.claude`, `~/.gemini`). Never require separate paid API keys for core operation. East–West communication (harness-to-harness messaging and trajectory inspection) is enabled by injecting Meta-Harness's embedded MCP bridge (`meta-harness mcp-bridge`) into each harness's `session/new` `mcpServers` list.
4. **Deterministic `@mention` Routing + Smart Orchestrator**:
   - Messages containing explicit `@<handle>` mentions (outside code blocks/spans) route directly to those harnesses.
   - Messages with **no** `@<handle>` route to the session's configured **Orchestrator Harness**, which inspects subscription quota status, task complexity, model characteristics, and episodic memory to pick an executor harness, `model`, and `reasoning_effort` (applied via ACP `session/set_config_option`).
5. **High-Level Episodic Memory Only**: Persist *who did what job, which modules/files were touched, outcomes, and key architectural decisions* in SQLite + FTS5. **Do not** build heavy vector-DB pipelines, embedding models, or raw line-by-line code memorization.
6. **Frugal Resource Footprint & Snappy Responsiveness**:
   - **Backend**: Single Rust binary (`tokio` + `axum` + `rusqlite` bundled + `agent-client-protocol`). Target <30 MB idle RSS for Meta-Harness itself. Spawn harness subprocesses lazily on demand and reap idle ones cleanly.
   - **Frontend**: Lightweight SPA (`Vite` + `React 19` + `TypeScript` + `Zustand` + `@tanstack/react-virtual` + `Tailwind CSS v4`), virtualizing message and trajectory lists so 60fps responsiveness holds under heavy token streaming.

---

## 2. Hard Architectural Invariants (Do Not Violate)

1. **No Raw Terminal Scraping for Agent Turns**: Always speak structured JSON-RPC 2.0 via ACP (`initialize`, `session/new`, `session/set_config_option`, `session/prompt`, `session/cancel`, `session/update`, `session/request_permission`, `fs/*`, `terminal/*`).
2. **Mandatory Loop & Runaway Protection**:
   - Every user message initiates a routing `chain_id` with a configurable hop budget (`max_agent_hops_per_user_turn`, default `6`).
   - Every agent-triggered message (`@mention` or `chat_send_message`) decrements the remaining hop budget. When `hops_remaining == 0`, pause dispatch and surface a user confirmation action in the UI.
   - Expose an immediate **Stop All (`session/cancel`)** endpoint/button that halts all in-flight turns in the session within <250ms.
3. **Mention Parsing Hygiene**:
   - Strip fenced code blocks (` ``` ... ``` `) and inline code spans (`` `...` ``) **before** extracting `@<handle>` tokens so code snippets like `@decorator`, `@types/node`, or email addresses never accidentally summon a harness.
   - Only match registered session member handles (plus reserved `@orchestrator` and `@all`).
4. **Windows-First Process & Path Discipline** (Primary Host OS: Windows):
   - Resolve `.cmd` / `.ps1` / `.exe` shims cleanly on Windows (e.g., `npx.cmd`, `codex.exe`, `agy.cmd`) using `which` crate before `tokio::process::Command::new`.
   - Attach every spawned harness child process tree to a **Windows Job Object** with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (and process groups on Unix) so zero orphan `node.exe` or agent subprocesses survive server shutdown or crashes.
   - Normalize filesystem paths consistently (`dunce::canonicalize` rather than raw `std::fs::canonicalize`, avoiding `\\?\` extended-length prefix bugs that break Node/CLI tools on Windows!).
5. **Single Binary MCP Bridge**:
   - Do **not** require Python or a separate Node daemon for the inter-harness MCP server. The main Rust binary must support a subcommand (`meta-harness mcp-bridge --server-url ... --session-id ... --caller ... --token ...`) that speaks standard MCP JSON-RPC over stdio to the harness and proxies calls over localhost HTTP/IPC to the running `mh-server`.
6. **File Write Conflict Visibility**:
   - All ACP `fs/write_text_file` requests pass through `mh-acp`'s Workspace Broker. Track active per-file writers per turn; if two harnesses attempt concurrent edits to the same file in the same turn chain, serialize or trigger a permission/conflict alert rather than silently clobbering changes.
7. **Token-Conscious Context Forwarding**:
   - Because each harness maintains its own stateful ACP `sessionId`, **only send incremental unseen group-chat deltas** (messages since that harness's `last_seen_msg_id`) plus a compact (<250 token) episodic memory header—never re-send the entire chat transcript on every turn!

---

## 3. Environment & Toolchain Bootstrap Notes (Windows)

Before starting Rust backend implementation on a fresh Windows machine, verify toolchains:

- **Node.js & Git**: Check `node -v`, `npm -v`, `git --version`.
- **Rust Toolchain (`rustup` / `cargo`)**:
  - Check `cargo --version`. If missing on Windows **and** Visual Studio C++ Build Tools (`cl.exe` / `link.exe`) are not installed:
    - **Recommended lightweight setup (zero admin rights, ~350 MB vs ~6 GB MSVC)**: Install Rust with the self-contained GNU ABI toolchain:
      ```powershell
      winget install --id Rustlang.Rustup -e --accept-package-agreements --accept-source-agreements
      rustup toolchain install stable-x86_64-pc-windows-gnu
      rustup default stable-x86_64-pc-windows-gnu
      ```
      *(Note: If Visual Studio C++ Build Tools ARE already installed, `stable-x86_64-pc-windows-msvc` works equally well. Ensure `rusqlite` uses `features = ["bundled"]` so SQLite compiles cleanly from C source under both GNU and MSVC toolchains.)*

---

## 4. Code Organization & Module Boundaries

Keep crates decoupled with strict unidirectional dependencies:

```text
mh-core       (pure domain models, @mention parser, complexity/quota scorer; zero I/O)
  ▲
mh-store      (SQLite + FTS5 schema, migrations, sessions, messages, trajectories, episodic memory)
  ▲
mh-acp        (ACP stdio process manager, configOptions mapper, workspace fs/terminal broker, quota probes)
  ▲
mh-switch     (group chat router, hop guard, orchestrator dispatch pipeline, MCP tool handlers)
  ▲
mh-server     (Axum REST + WebSocket hub, static frontend serving, CLI entrypoint & `mcp-bridge` mode)
```

Frontend (`web/`):
```text
web/src/
├── api/          # Typed REST client & multiplexed WebSocket client with auto-reconnect
├── store/        # Normalized Zustand stores (projects, activeSession, chatStream, trajectories, quotas, memory)
├── components/
│   ├── sidebar/  # Project picker, session list, harness roster + live quota meters
│   ├── chat/     # Virtualized message list, @mention composer, Orchestrator dispatch card, Permission card
│   ├── drawer/   # Trajectory inspector (thoughts, tool calls, diffs), Memory ledger, Switch settings
│   └── common/   # Accessible UI primitives, keyboard shortcuts, status badges
└── types/        # Shared wire DTOs mirroring Rust serde types
```

---

## 5. Coding Standards & Quality Gates

### Rust (`crates/*`)
- Rust Edition **2024** (or **2021** if toolchain <1.85).
- Use `thiserror` in library crates (`mh-core`, `mh-store`, `mh-acp`, `mh-switch`) and `anyhow` only at binary boundaries (`mh-server`).
- Never `.unwrap()` or `.expect()` on runtime I/O, user input, or external subprocess JSON payloads—malformed output from a third-party harness must degrade gracefully into a logged system event, never crash the switch.
- Buffer and coalesce high-frequency ACP `session/update` token chunks (e.g., 16–33ms batch window) before broadcasting over WebSocket and flushing to SQLite so streaming stays silky smooth with minimal SQLite write amplification.
- **Pre-completion checks**:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`

### Frontend (`web/*`)
- Strict TypeScript (`"strict": true`, zero `any` in domain models).
- Virtualize long lists (`@tanstack/react-virtual`) and memoize message rows so streaming tokens in the active turn only re-render the tail message bubble.
- Keyboard-first UX: `@` opens mention menu, `Up/Down/Enter/Tab` selects harness, `Cmd/Ctrl+Enter` sends, `Esc` closes drawers or cancels compose, `Alt+A` / `Alt+R` resolves pending permission prompts.
- **Pre-completion checks**:
  - `npm run typecheck` (`tsc --noEmit`)
  - `npm run test` (`vitest run`)
  - `npm run build`

### Testing Expectations
- **Mock ACP Harness Binary/Script**: Build a deterministic mock ACP agent in `crates/mh-acp/tests/fixtures/` so integration tests can verify `@mention` routing, `mcp-bridge` inter-harness communication, trajectory reading, permission gating, hop limits, and orchestrator `session/set_config_option` calls **offline and in CI without burning real user AI subscriptions**.
