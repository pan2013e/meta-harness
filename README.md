# Meta-Harness

**Meta-Harness** is a lightweight, local-first web application that turns multiple AI coding harnesses (**Codex**, **Claude Code**, **Antigravity / Gemini**, and any ACP-compatible agent) into a collaborative **project-scoped group chat** connected through a programmable **ACP Network Switch** and **per-harness Git worktree isolation**.

All harnesses run locally over the **Agent Client Protocol (ACP)** using your **existing AI subscriptions**—no separate API keys or third-party proxy billing required.

---

## Core Capabilities

1. **Project-Scoped Sessions with Git Worktree Isolation**:
   - Every session creates a clean **Session Integration Branch/Worktree** (`mh/s-<id>`) plus **isolated Git worktrees per executor member** (`@codex`, `@claude`, `@antigravity`, or custom personas like `@codex-reviewer`), leaving your main checkout untouched while multiple harnesses work in parallel.
   - Clean turns auto-merge into the session branch in Rust at zero token cost; merge conflicts or multi-agent waves automatically wake the **Orchestrator** to integrate and resolve conflicts. Preview/test integrated changes and promote back to your main checkout via **Fast-Forward/Merge**, **Squash Merge**, or **Apply Patch**.
2. **ACP Network Switch, Sync Consults & Trajectory Sharing**:
   - Injects a single-binary MCP bridge (`meta-harness mcp-bridge`) into every harness via ACP `session/new`.
   - Harnesses can run in parallel, post async `@mention` handoffs in Group Chat, invoke deadlock-checked synchronous mid-turn consults (`peer_consult`), and inspect peer trajectories (`trajectory_read`).
3. **`@mention` Routing & Quota-Aware Orchestrator**:
   - Direct `@handle` messages route straight to those members; unmentioned prompts route to your configured **Orchestrator Harness** (which can also dispatch executor work to its own harness brand via a separate worktree while sharing the adapter's quota pool).
   - Evaluates live subscription headroom (local CLI logs + ACP status + 429 circuit breakers + opt-in exact probes), task complexity, model strengths, and project memory to select executor(s), `model`, and `reasoning_effort` via `session/set_config_option`.
4. **Orchestrator-Curated Episodic Memory**:
   - After each completed job, the Orchestrator runs a lightweight, low-effort curation pass (folded into merge/synthesis turns when active) to record *who did what job*, module ownership, and harness performance notes in project-wide **SQLite + FTS5** memory.
5. **Cross-Platform & Lightweight (+ Optional LAN/Tailscale Mode)**:
   - Cross-platform single Rust binary for **Windows, macOS, and Linux** (`axum` + `tokio` + `rusqlite`) serving a virtualized `Vite + React 19 + TypeScript + Tailwind v4` UI on `127.0.0.1:4410` by default, with optional `--bind` + bearer-token auth for mobile/laptop access over LAN or Tailscale.

---

## Repository Map

```text
meta-harness/
├── AGENTS.md                     # Canonical engineering contract & general specs for coding agents
├── CLAUDE.md                     # Entrypoint pointer for Claude Code -> AGENTS.md
├── configs/
│   └── harnesses.example.toml    # Example ACP adapter & switch configuration
├── docs/
│   ├── SPECIFICATION.md          # Detailed system, worktree, switch/MCP, DB, and UI specification
│   └── IMPLEMENTATION_PLAN.md    # Phased roadmap (Phase 0 -> Phase 6) & verification gates
├── crates/                       # Rust workspace crates
│   ├── mh-core/                  # Domain types, @mention parser, wait-for DAG, quota/complexity models
│   ├── mh-store/                 # SQLite + FTS5 persistence & project episodic memory
│   ├── mh-acp/                   # ACP stdio supervisor, worktree & git broker, permissions, quota probes
│   ├── mh-switch/                # Parallel router, hop & deadlock guard, orchestrator, MCP bridge handlers
│   └── mh-server/                # Axum HTTP/WS server, optional LAN auth, CLI & `mcp-bridge` subcommand
└── web/                          # Vite + React 19 + TypeScript + Tailwind v4 frontend
```

---

## Documentation for Developers & Agents

- **[AGENTS.md](AGENTS.md)** — Mandatory architectural invariants, coding standards, and Windows toolchain instructions for any agent implementing features in this repository.
- **[docs/SPECIFICATION.md](docs/SPECIFICATION.md)** — End-to-end architecture, Git worktree lifecycle, ACP + MCP switch wire contracts, orchestrator policy, SQLite DDL, and UI layout.
- **[docs/IMPLEMENTATION_PLAN.md](docs/IMPLEMENTATION_PLAN.md)** — Step-by-step milestone plan (Phase 0 -> Phase 6) with acceptance criteria.
