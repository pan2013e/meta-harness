# Meta-Harness

**Meta-Harness** is a local-first workspace for collaborating with **Codex**, **Claude Code**, **Antigravity / Gemini**, and other **Agent Client Protocol (ACP)** harnesses. It provides project group chats with isolated Git worktrees and Quick Sessions for work without an existing repository.

Meta-Harness launches the harness CLIs locally over ACP. The adapters use your existing CLI sign-ins and subscription plans.

## Quick start

1. Get the Electron package or standalone executable for your platform from [Release build](https://github.com/pan2013e/meta-harness/actions/workflows/release-build.yml), or [build from source](#build-a-desktop-release).
2. Start Meta-Harness. The standalone executable opens the loopback web UI at `127.0.0.1:4410` by default. Electron opens the app in its own window.
3. Use **Settings → Harnesses & Models** to check which CLIs and sign-ins are available. Open a project folder or start a **Quick Session**, then send a message to the orchestrator or address a member with `@handle`.

---

## Core Capabilities

- **Worktree isolation.** Git-backed sessions use an integration branch at `mh/<session_id>/integration` and lazily create per-executor branches at `mh/<session_id>/exec-<handle>`. The Git broker merges clean edits automatically and escalates conflicts to the orchestrator. Review changes and promote them to the project checkout using merge, squash, or patch. Non-Git folders use serialized shared-folder sessions.
- **Harness collaboration.** Direct `@mentions` route to members. Unmentioned messages go to the orchestrator, which can use model choices, quota data, and project memory to dispatch work. The embedded MCP bridge supports asynchronous handoffs, cycle-checked synchronous consults in isolated sessions, and peer trajectory inspection.
- **Live subscription usage.** Built-in adapters use provider probes by default where available. Usage is tracked per adapter and shared by its personas. The composer shows subscription usage alongside context usage, and unavailable quota windows are labeled accordingly.
- **Review and preview.** The session header opens the diff review and terminal. Hiding the terminal preserves its interactive shell. File previews render HTML with its relative assets, images, PDFs, and read-only source in Monaco within the app.
- **Project memory.** SQLite and FTS5 store job summaries and project facts. The orchestrator can curate entries for completed project work, with deterministic records as a fallback.
- **Local and remote access.** The standalone Rust binary serves the same React UI used by Electron. A non-loopback `--bind` enables token authentication for access over a LAN or Tailscale.

---

## Separate harness profiles

In **Settings → Harnesses & Models**, choose **Create profile & sign in** for Codex, Claude Code, or Antigravity. Follow the provider's sign-in steps shown in Settings. Meta-Harness activates the new directory after sign-in completes and restores it on restart. A preconfigured `profile_dir` takes precedence.

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
│   └── SPECIFICATION.md          # System, protocol, storage, and UI specification
├── desktop/                      # Electron launcher and packaging
├── scripts/                      # Release builds and synchronized version bumps
├── crates/                       # Rust workspace crates
│   ├── mh-core/                  # Domain types, @mention parser, wait-for DAG, quota/complexity models
│   ├── mh-store/                 # SQLite + FTS5 persistence & project episodic memory
│   ├── mh-acp/                   # ACP stdio supervisor, worktree & git broker, permissions, quota probes
│   ├── mh-switch/                # Parallel router, hop & deadlock guard, orchestrator, MCP bridge handlers
│   └── mh-server/                # Axum HTTP/WS server, optional LAN auth, CLI & `mcp-bridge` subcommand
├── web/                          # Vite + React 19 + TypeScript + Tailwind v4 frontend
└── CHANGELOG.md                  # Notable and milestone changes
```

---

## Documentation for Developers & Agents

- **[AGENTS.md](AGENTS.md)** — Mandatory architectural invariants, coding standards, and Windows toolchain instructions for any agent implementing features in this repository.
- **[docs/SPECIFICATION.md](docs/SPECIFICATION.md)** — Architecture, Git worktree lifecycle, ACP and MCP contracts, storage, server APIs, and UI behavior.

## Build a desktop release

Install Node.js 24, Git, and the stable Rust toolchain for your OS. On Windows use Visual Studio C++ Build Tools for the MSVC toolchain or put the matching MinGW-w64 tools on PATH for the GNU toolchain. On macOS install the Xcode Command Line Tools.

```sh
node scripts/build.mjs
node scripts/build.mjs --cli-only
```

The full build puts the Electron installer and archive in `dist/desktop` and the standalone native binary in `dist/cli`. Use `--unpacked` to build an unpacked desktop app for local testing. GitHub Actions also offers a **Release build** workflow for Windows and macOS artifacts.

The native release starts in the Windows system tray or macOS menu bar and opens your browser when the server is bound to loopback. Use `--no-open` to skip opening the browser and `--no-tray` for foreground operation. The Electron app opens in its own window and keeps running in the tray when you close the window. **Quit Meta-Harness** stops the app and its backend. Local packages are unsigned unless signing is configured.

For a development build, run `npm --prefix web ci` and `npm --prefix web run build` before starting the server with `cargo run --locked --bin meta-harness`. With the backend running on port 4410, `npm --prefix web run dev` serves the frontend at port 5173 and proxies API and WebSocket requests to the server.

## Versioning

Meta-Harness uses semantic versions across the Rust CLI, web UI, and Electron package. The initial release is `v0.1.0`.

Use `node scripts/version.mjs patch` for fixes, `minor` for compatible features, or `major` for a breaking stable release. The script updates all three manifests and their lockfiles together. Include the bump with the next change commit. CI and the release build check that the versions agree.

Record noteworthy changes in [CHANGELOG.md](CHANGELOG.md). When publishing a milestone, add a versioned heading for its notes and tag the release commit as `vMAJOR.MINOR.PATCH`.

The About page shows the server version, a six-character build commit when available, the repository link, and license information.
