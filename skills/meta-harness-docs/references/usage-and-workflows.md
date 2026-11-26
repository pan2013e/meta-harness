# Meta-Harness Usage & Multi-Agent Workflows

## 1. Workspaces: Git Projects, Shared Folders, & Quick Sessions

- **Git-Backed Projects (Recommended):** Open a local folder via **Open** in the sidebar (`FolderPickerModal` browses directories in-app across Windows/macOS/Linux; common dot folders like `.agents`, `.meta-harness`, `.claude`, `.codex`, and `.gemini` are browsable). Each session branches from current `HEAD` into `mh/<session_id>/integration` under `<repo>/.meta-harness/worktrees/<session_id>/_integration` (hidden via `.git/info/exclude`), and spins up executor worktrees (`_exec_<handle>`, `mh/<session_id>/exec-<handle>`) on demand.
- **Non-Git Folders:** Offer an **Initialize Git** button in the sidebar/Welcome screen or run in **Shared Folder** mode with turns serialized on one session lock.
- **Quick Sessions:** Leave **Quick Sessions** selected in the project dropdown to work outside any repository. Checkouts live under `~/.meta-harness/workspaces/<session_id>` (overridable via `MH_WORKSPACE_DIR`), automatically Git-initialized.
- **Archiving & Restoring:** Right-click (or `...`) a session to **Rename**, **Archive**, or **Delete**:
  - **Archive** checkpoints dirty edits, removes on-disk worktree directories to reclaim disk space, and retains chat history + `mh/<session_id>/*` branches under **Archived (N)**.
  - **Unarchive** recreates the integration worktree.
  - On Codex App Server and compatible Claude Code ACP adapters, archiving/restoring/deleting also synchronizes linked native CLI conversations.

---

## 2. Routing, Orchestration, & Inter-Harness Collaboration

### Mention & Orchestrator Routing

| Prompt Pattern | Routing Behavior |
| :--- | :--- |
| Unmentioned prompt *(e.g. `Refactor auth.ts`)* | Sent to the session **Orchestrator** (`orchestrator_member`) with `<orchestrator_telemetry>` containing live quota headroom, discovered models/efforts, FTS5 **Project Memory** matches, and `<available_skills>`. |
| `@codex <task>` | Routed directly to `@codex` in its isolated executor worktree. |
| `@codex @claude <task>` | Dispatched to both `@codex` and `@claude` in parallel across separate worktrees; clean edits auto-merge into integration. |
| `@codex say hello to @antigravity` | Leading mention targets `@codex`; body mentions remain part of the instruction for agent-to-agent handoffs. |
| `Ask @codex to greet @antigravity` | Separated inline mentions trigger a short hidden low-effort orchestrator disambiguation pass constrained to those candidate handles. |
| `@orchestrator ...` / `@all ...` | `@orchestrator` takes precedence; `@all` broadcasts to all available non-orchestrator-only members. |
| `@handle[model:effort]` | Inline model/reasoning override syntax (e.g. `@codex[gpt-5.4:high]`). |

### MCP Collaboration Tools (`meta-harness-switch`)

Injected into every `session/new` / `session/load` via `meta-harness mcp-bridge`:

- **All members:**
  - `chat_list_peers` — Member handles, roles, states, models, and worktree branches.
  - `chat_read_messages` — Paginated read of recent group-chat messages.
  - `chat_send_message` — Post a group-chat message and asynchronously wake `@to` recipients (consumes 1 hop).
  - `peer_consult` — Ask an idle peer a question synchronously mid-turn (`target`, `question`, optional `context_turn_id`). Protected by `WaitForGraph` deadlock detection; renders a collapsible **Sync Consult** thread in chat.
  - `trajectory_list` & `trajectory_read` — Inspect peer turn summaries, tool calls, diffs, or full paginated events (`16 KiB/page`). Emits an inline `ActivityPill` (`@a read @b's trajectory`).
  - `memory_recall` & `memory_record_fact` — Search SQLite FTS5 `episodic_jobs` and record durable `project_facts`.
  - `quota_get_status` — Inspect adapter 5h/weekly quota windows and cooldowns.
  - `skill_list` & `skill_read` — List enabled skills and read `SKILL.md` / reference files with zero permission prompts.
- **Orchestrator additional tools:**
  - `models_list_catalog` — Live discovered ACP `configOptions` + static adapter strengths/latency tiers.
  - `orchestrator_dispatch` — Dispatch 1+ parallel or sequential tasks with explicit `target`, `model`, `effort`, and `task_brief` (auto-approved during orchestrator turns; honors `confirm_before_dispatch` when enabled via API).
  - `memory_curate_job` — Persist curated high-signal summaries + optional `project_fact` entries.

---

## 3. Composer Controls, Approvals, Diffs, Terminal, & Memory

- **Desktop menus:** Right-click selected message text to copy it or append a quote in the composer. Quoted text stays out of recipient and skill routing. The composer and terminal offer **Copy** and **Paste**, and the Windows title bar follows the selected theme.
- **Atomic Pills in Composer:**
  - `@handle` pills delete atomically on Backspace.
  - **Built-in workflow pills (green):** `/plan`, `/review`, `/test`, `/fix` (injected as structured `<slash_commands>` directives).
  - **Built-in action pills (amber):** `/compact`, `/diff`, `/shell`, `/memory`, `/new`, `/archive`.
  - **Skill pills:** Typing `/` also lists enabled skills (e.g. `/meta-harness-docs`) in a distinct sky-blue pill that preloads `SKILL.md` for that turn and logs an inline activity pill.
- **Model, Effort, & Context Ring:** Bottom-left selector supports per-recipient model & reasoning effort overrides when multiple agents are addressed. Bottom-right rings track subscription headroom and per-member context window composition (live ACP `usage_update` when available, or calibrated estimate including `gpt-6*` 1M windows).
- **Approval Modes & Shortcuts:**
  - **Worktree** (`worktree_smart`, default): auto-allows project reads, skill reads, and in-worktree edits; prompts for shell commands (supports **"Allow Pattern for Session"** saved in `session_permission_rules`).
  - **Strict** (`strict`): prompts on edits and shell executions.
  - **Auto** (`autonomous_sandbox`): allows in-worktree edits and safe commands; prompts on risky patterns (`git push`, `rm -rf`, etc.) and external paths.
  - Pending approval card hotkeys: `Alt+A` (Allow once), `Alt+R` (Reject).
- **Diff Review & Promote:** Header badge shows integration diff stats (`+N -M Review & Promote`). Supports **Fast-Forward / Merge**, **Squash Merge**, or **Apply Uncommitted Patch** to base branch.
- **In-App File Previews:** Clicking workspace file links opens read-only Monaco code view, native image viewer, PDF.js viewer, or sandboxed HTML iframe via `/preview/{session_id}/{grant}/{*path}` (30-min scoped read-only grants, 24 MiB cap).
- **Multi-Tab Interactive PTY Terminal (`Ctrl+\``):** Launches PowerShell / `$SHELL` with UTF-8/CJK support inside `Integration` or any `@member` worktree (`+` tab).
- **Project Memory:** Shared across all sessions of a project (`episodic_jobs` + `project_facts` in FTS5).

---

## 4. Settings, Profiles, CLI Options, & Remote Access

Open **Settings** from the bottom-left sidebar:
- **Appearance:** `Obsidian Studio` (default), `Tokyo Night`, `Catppuccin Mocha`, `Paper Daylight`.
- **Harnesses & Models:** Probe CLIs, refresh live subscription quotas, set default model/effort, check/upgrade CLIs, and create/switch isolated **Separate Profiles** (including Codex device-code login).
- **Skills:** View enabled/disabled global & repo-local (`.agents/skills/`) skills, install from **Local Folder** or **Git / GitHub URL**, author custom skills inline, inspect/edit files, reset built-ins, or uninstall.
- **Global Personas:** Reusable specialist templates (e.g. `@codex-reviewer`) copied into new sessions or added via **Add to session**.
- **Session:** Reassign orchestrator, tune `max_hops` (`2–16`), edit member models/personas, adjust approval mode, prune session command allow-rules.
- **Storage & Data:** Itemized disk usage breakdown, **Clear Caches & Orphans**, **Delete Archived Sessions**, and versioned JSON Settings export/import.
- **About:** App version, six-character build commit, license and third-party notices, and release update checks.

### CLI & Environment Overrides
- Run as a standard user. Both app entry points refuse Unix root and elevated Windows launches.
- `meta-harness [serve]` flags include `--bind <ADDR:PORT>` (default `127.0.0.1:4410`), `--tls-cert <PATH>`, `--tls-key <PATH>`, `--auth-token <TOKEN>`, `--no-open`, `--no-tray`, `--config <PATH>`, and `--data-dir <PATH>`.
- Env overrides: `MH_CONFIG_DIR`, `MH_DATA_DIR`, `MH_WORKSPACE_DIR` (defaults Quick Sessions to `~/.meta-harness/workspaces`).
- Direct LAN binds require a trusted HTTPS certificate and key. Use `#token=<TOKEN>` pairing in browsers or `Authorization: Bearer <TOKEN>` for API clients. The browser receives a signed `HttpOnly; SameSite=Strict` session cookie that expires after 24 hours and is `Secure` over HTTPS. The chat and terminal sockets also close at expiry.
- To use an HTTPS reverse proxy, bind Meta-Harness to loopback, configure a strong `--auth-token`, and forward the original `Host` and WebSocket upgrades. Pairing grants workspace and terminal access.
- Standalone release binaries provide `install`, `update --check`, `update`, and `uninstall`. Desktop releases use the Windows installer or macOS disk image for updates. Uninstall keeps sessions and settings.
