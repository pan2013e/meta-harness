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

## Separate harness profiles

In **Settings → Harnesses**, choose **Create profile & sign in** for Codex, Claude Code, or Antigravity. Follow the provider's sign-in steps shown in Settings. Meta-Harness activates the new directory after sign-in completes and restores it on restart. A preconfigured `profile_dir` takes precedence.

For profiles created in the web app, **Use default CLI profile** returns to the CLI's ordinary sign-in. **Use saved separate profile** switches back using the saved credentials. The selection persists across restarts.

Codex's primary sign-in button opens the provider's browser login on the machine running Meta-Harness. **Use device code** starts the alternative flow, where you enter the code on the provider's page. Separate Antigravity profiles can use **Check shared CLI usage** to view the shared account's quota for reference.

Use **Check CLI version** and **Upgrade CLI** in the same card for supported launchers. Upgrades run the vendor updater or refresh the compatible npm bundle for the default Codex ACP launcher.

Under **Settings → Session**, a persona can inherit its harness's defaults or choose its own model and reasoning effort. Removing a persona preserves its chat history and worktree.

Set `profile_dir` under an adapter in your Meta-Harness config, then restart Meta-Harness. Relative paths are rooted in the Meta-Harness data directory. Each adapter can use a different profile.

```toml
[adapters.codex]
profile_dir = "profiles/codex"

[adapters.claude]
profile_dir = "profiles/claude"

[adapters.antigravity]
profile_dir = "profiles/antigravity"
```

Add these keys to the existing adapter sections in `configs/harnesses.local.toml`, or start with `configs/harnesses.example.toml`. Meta-Harness creates the directories on first launch of each harness and starts a fresh ACP session when the profile changes. Existing chat stays in Meta-Harness, and existing native conversations stay in their original profiles.

For Codex or Claude Code, sign in to the separate directory using the harness CLI with `CODEX_HOME` or `CLAUDE_CONFIG_DIR` pointing to the resolved profile path shown in Settings. For Antigravity, the ACP launcher uses `--gemini-dir <profile> --app-data-dir antigravity-cli`. The OS keyring may still supply the same Google account. When needed, run `agy acp --login` with the same directory flags. Gemini CLI fallback uses `GEMINI_CLI_HOME`.

## Global personas and settings backups

Create reusable personas under **Settings → Global Personas**. New sessions include saved personas whose harnesses are available. Use **Add to session** for an existing session. Each session keeps its own copy of the persona's instructions and defaults.

Mention multiple agents in the composer to get a model and reasoning effort tab for each recipient. Selections in the composer apply to messages from that composer. Set a persona's persistent defaults in Settings.

Use **Settings → Storage & Data → Settings backup** to export or import versioned JSON containing the theme, harness model defaults, quota preferences, and global personas. Import merges records by harness ID or persona handle. Keep machine-specific TOML, sign-in profiles, and session data in their original locations.

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
## Build a desktop release

Install Node.js 24, Git, and the stable Rust toolchain for your OS. On Windows use Visual Studio C++ Build Tools for the MSVC toolchain or put the matching MinGW-w64 tools on PATH for the GNU toolchain. On macOS install the Xcode Command Line Tools.

```sh
node scripts/build.mjs
node scripts/build.mjs --cli-only
```

The full build puts the Electron installer and archive in `dist/desktop` and the standalone native binary in `dist/cli`. Use `--unpacked` to build an unpacked desktop app for local testing. GitHub Actions also offers a **Release build** workflow for Windows and macOS artifacts.

The native release starts in the Windows system tray or macOS menu bar. Choose **Open Meta-Harness** to open it in your browser. Use `--no-tray` for foreground server operation. The Electron app opens in its own window and keeps running in the tray when you close the window. **Quit Meta-Harness** stops the app and its backend. Local packages are unsigned unless signing is configured.
