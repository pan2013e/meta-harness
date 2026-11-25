# Meta-Harness

**Meta-Harness** is a lightweight, local-first web application that turns multiple AI coding harnesses (**Codex**, **Claude Code**, **Antigravity / Gemini**, and any ACP-compatible agent) into a collaborative **project-scoped group chat** connected through a programmable **ACP Network Switch**.

All harnesses run locally over the **Agent Client Protocol (ACP)** using your **existing AI subscriptions**—no separate API keys or third-party proxy billing required.

---

## Core Capabilities

1. **Project-Scoped Group Sessions**: Open sessions rooted in any project folder (`cwd`), with the user and multiple agent harnesses (`@codex`, `@claude`, `@antigravity`) sharing a real-time group chat.
2. **ACP Network Switch & Trajectory Sharing**: Acts as an L2/L3 switch over ACP stdio processes. Injects an embedded MCP bridge (`meta-harness-switch`) into every harness via `session/new` so harnesses can message peers (`@handle`) and inspect each other's structured trajectories (plans, tool calls, diffs, thoughts) on demand.
3. **`@mention` Routing & Intelligent Orchestrator**: Address any harness directly with `@codex` / `@claude`, or configure an **Orchestrator Harness** to handle unmentioned prompts—analyzing task complexity, model strengths, episodic memory, and live **subscription quota headrooms** before selecting an executor, model, and reasoning effort via ACP `session/set_config_option`.
4. **High-Level Episodic Memory**: Remembers *which harness did what job*, architectural decisions, and module ownership in SQLite + FTS5 without indexing noisy low-level details.
5. **Lightweight & Responsive**: Rust backend (`axum` + `tokio` + `rusqlite` + `agent-client-protocol`) with lazy subprocess spawning, Win32 Job Object lifecycle safety, and a virtualized TypeScript + Vite + React UI.

---

## Repository Map

```text
meta-harness/
├── AGENTS.md                     # Canonical engineering contract & general specs for coding agents
├── CLAUDE.md                     # Entrypoint pointer for Claude Code -> AGENTS.md
├── configs/
│   └── harnesses.example.toml    # Example ACP harness registry (Codex, Claude Code, Antigravity)
├── docs/
│   ├── SPECIFICATION.md          # Detailed system, protocol, database, and UI/UX specification
│   └── IMPLEMENTATION_PLAN.md    # Phased roadmap, crate/module breakdown, and verification gates
├── crates/                       # (Planned) Rust workspace crates
│   ├── mh-core/                  # Domain types, routing grammar, complexity/quota scoring
│   ├── mh-store/                 # SQLite + FTS5 persistence & episodic memory engine
│   ├── mh-acp/                   # ACP process supervisor, stdio client, workspace broker, quota probes
│   ├── mh-switch/                # Session bus, hop/loop guard, orchestrator pipeline, MCP bridge
│   └── mh-server/                # Axum HTTP/WS server, static asset host, `mcp-bridge` CLI mode
└── web/                          # (Planned) Vite + React + TypeScript + Tailwind v4 frontend
```

---

## Documentation for Developers & Agents

- **[AGENTS.md](AGENTS.md)** — General specifications, architectural invariants, coding standards, and Windows toolchain instructions for any agent implementing features in this repository.
- **[docs/SPECIFICATION.md](docs/SPECIFICATION.md)** — End-to-end architecture, ACP + MCP switch wire contracts, orchestrator policy, SQLite DDL, and UI layout.
- **[docs/IMPLEMENTATION_PLAN.md](docs/IMPLEMENTATION_PLAN.md)** — Step-by-step milestone plan (Phase 0 -> Phase 6) with acceptance criteria.
