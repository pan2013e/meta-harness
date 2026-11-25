# Meta-Harness Technical Specification

## 1. System Overview

**Meta-Harness** is a local-first web application that coordinates multiple AI coding harnesses (**Codex**, **Claude Code**, **Antigravity / Gemini CLI**, and custom ACP-compliant agents) inside **project-scoped group chat sessions**.

Instead of re-implementing an LLM agent loop with raw API keys, Meta-Harness acts as:
1. **An ACP Client Hub** speaking JSON-RPC 2.0 over stdio to locally installed agent harnesses (reusing the user's existing AI subscriptions).
2. **An L2/L3 Agent Network Switch** routing messages (`@mentions`), exposing cross-harness **Trajectory Inspection** and **Peer Messaging** via an injected local MCP bridge (`meta-harness mcp-bridge`), and enforcing hop/loop safety.
3. **An Intelligent Orchestration Plane** where a user-selected **Orchestrator Harness** inspects live subscription usage/quotas, model characteristics, task complexity, and high-level episodic memory before dispatching work to an **Executor Harness** with tailored `model` and `reasoning_effort` settings.

```mermaid
flowchart TB
    subgraph Browser["Web Frontend (React + Vite + Zustand)"]
        ChatUI["Group Chat & @Mention Composer"]
        Drawer["Trajectory Inspector & Memory Ledger"]
        Roster["Harness Roster & Live Quota Meters"]
    end

    subgraph Backend["Rust Backend (`meta-harness` binary)"]
        WS["Axum REST + Multiplexed WebSocket Hub"]
        Switch["Network Switch Router (Mention Parser, Hop Guard, Delta Sync)"]
        Orch["Orchestrator Pipeline & Quota/Complexity Telemetry"]
        Mem["Episodic Memory Engine (SQLite + FTS5)"]
        Broker["ACP Supervisor & Workspace File/Terminal Broker"]
        MCPHub["Internal Switch MCP Endpoint (`/internal/mcp`)"]
    end

    subgraph Harnesses["Local Agent Harnesses (User Subscriptions)"]
        Codex["@codex (`codex-acp` stdio)"]
        Claude["@claude (`claude-agent-acp` stdio)"]
        AGY["@antigravity (`agy --acp` stdio)"]
        Bridge["`meta-harness mcp-bridge` (stdio <-> HTTP)"]
    end

    Browser <-->|"HTTP + WS JSON"| WS
    WS <--> Switch
    Switch <--> Orch
    Switch <--> Mem
    Switch <--> Broker
    Broker <-->|"ACP JSON-RPC (stdio)"| Codex
    Broker <-->|"ACP JSON-RPC (stdio)"| Claude
    Broker <-->|"ACP JSON-RPC (stdio)"| AGY
    Codex & Claude & AGY <-->|"MCP (stdio)"| Bridge
    Bridge <-->|"Localhost HTTP + Bearer Token"| MCPHub
    MCPHub <--> Switch
    MCPHub <--> Mem
    MCPHub <--> Orch
```

---

## 2. Core Domain Concepts

### 2.1 Project
- Represents a validated directory on the local filesystem (`root_path`, normalized via `dunce::canonicalize`).
- Tracks basic repository context (`is_git_repo`, current branch, dirty status) and owns **Project Episodic Memory** shared across sessions in that folder.

### 2.2 Session (Group Chat Room)
- Created under a specific **Project** (`cwd = project.root_path`, or an optional dedicated subdirectory/git worktree).
- Contains:
  - **Participants**: The `user` plus an active subset of registered **Harness Profiles** (e.g., `["codex", "claude", "antigravity"]`).
  - **Orchestrator Config**: Optional `orchestrator_harness_id` (e.g., `"antigravity"` or `"claude"`), optional fixed orchestrator `model` / `reasoning_effort` (typically fast/low-cost for triage), and routing policy (`auto_dispatch`, `confirm_before_dispatch`).
  - **Switch Policy**: `max_agent_hops_per_user_turn` (default `6`), permission policy (`prompt`, `allow_edits_prompt_shell`, `yolo_sandboxed`).

### 2.3 Message vs. Trajectory
- **Chat Message**: High-signal item in the group chat stream:
  - `user_text`: Human prompt (with optional `@mentions` and file attachments/paths).
  - `orchestrator_dispatch`: Structured routing card showing chosen executor, model, reasoning effort, quota snapshot, and rationale.
  - `agent_reply`: Top-level conversational response emitted by a harness, linked `1:1` to a `turn_id` Trajectory.
  - `system_notice` / `permission_request`: Switch lifecycle alerts, hop-limit pauses, or interactive tool-approval cards.
- **Trajectory (`TurnTrajectory`)**: All granular ACP `session/update` events recorded during a single `session/prompt` execution (`turn_id`):
  - `thought_chunks` (coalesced reasoning blocks)
  - `plan_updates` (agent step checklists)
  - `tool_calls` (name, input args, status, condensed output, duration)
  - `file_edits` (path, unified diff, before/after sha256)
  - `terminal_runs` (command, cwd, exit code, tail output)

---

## 3. ACP Client Lifecycle & Harness Supervision (`mh-acp`)

### 3.1 Lazy Spawning & Subscription Inheritance
1. Harnesses are **not** spawned when `meta-harness` starts or when a session opens. A subprocess spawns lazily on its first assigned turn in a session.
2. Child processes inherit `USERPROFILE` / `HOME`, `APPDATA`, `LOCALAPPDATA`, and `PATH` so existing logins (`~/.codex/auth.json`, `~/.claude/`, `~/.gemini/`) work seamlessly with zero credential copying.
3. On Windows, every child process is immediately attached to a Win32 Job Object configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
4. An idle reaper checks harnesses every 60s; processes idle longer than `idle_process_ttl_secs` (default `900s`) are gracefully terminated and transparently respawned (`session/load` if supported by the agent's `agentCapabilities.loadSession`, otherwise `session/new` seeded with the session's compact working summary) on next message.

### 3.2 ACP Handshake & Configuration Flow
For each spawned harness in a session:
1. **`initialize`**:
   - Advertises client capabilities: `fs: { readTextFile: true, writeTextFile: true }`, `terminal: true`.
   - Captures `agentCapabilities` and supported features.
2. **`session/new`** (or **`session/load`**):
   - Passes `cwd = session.project_root`.
   - Injects the **Switch MCP Bridge** into `mcpServers`:
     ```json
     {
       "cwd": "C:/Users/panzh/my-project",
       "mcpServers": [
         {
           "name": "meta-harness-switch",
           "command": "C:/path/to/meta-harness.exe",
           "args": [
             "mcp-bridge",
             "--url", "http://127.0.0.1:4410/internal/mcp",
             "--session", "<session_id>",
             "--caller", "<harness_handle>"
           ],
           "env": [
             { "name": "MH_BRIDGE_TOKEN", "value": "<ephemeral_hmac_token>" }
           ]
         }
       ]
     }
     ```
   - Inspects returned `configOptions` array and indexes option IDs by semantic category (`model`, `thought_level` / `reasoning_effort`, `mode`) alongside their allowed `options` values.
3. **`session/set_config_option`** (Dynamic Model & Effort Selection):
   - Before invoking `session/prompt`, if the Orchestrator or user requested a target `model` or `reasoning_effort` that differs from the harness's active value, `mh-acp` fuzzy-matches or maps the requested value against the harness's advertised `configOptions` and sends `session/set_config_option`.
   - If a harness does not advertise a `reasoning_effort` selector for a specific non-reasoning model, `mh-acp` logs a benign notice and skips setting effort rather than failing the turn.
4. **`session/prompt` & Delta Context Envelope**:
   - Maintains `last_seen_message_seq` per `(session_id, harness_handle)`.
   - Prepends unseen group-chat messages since `last_seen_message_seq` in a compact XML envelope so the harness sees what user and peer agents said since its last turn without duplicating prior context:
     ```xml
     <group_chat_updates session="proj-alpha" your_handle="@codex">
       <msg id="41" from="user" to="@claude">Design the REST endpoints for tasks.</msg>
       <msg id="42" from="@claude" turn_id="t_89">Here is the OpenAPI contract in docs/api.md. @codex please implement the Axum handlers.</msg>
     </group_chat_updates>
     ```
5. **Client Callback Broker (`fs/*`, `terminal/*`, `session/request_permission`)**:
   - **Path Guard**: Rejects `fs/read_text_file` or `fs/write_text_file` outside `session.project_root` unless approved via an explicit user permission prompt.
   - **Diff Capture & Conflict Lock**: Before executing `fs/write_text_file`, snapshots prior file content to compute a unified diff stored in `TurnTrajectory`, and checks if another harness has an uncommitted write lock on that path in the same active chain.
   - **Permission Forwarding**: Emits `permission.requested` over WebSocket to the UI and awaits a `oneshot` channel resolution (`allow_once`, `allow_always_session`, `reject`)—or auto-resolves if matched by session permission rules.

---

## 4. Network Switch & Inter-Harness MCP Bridge (`mh-switch`)

### 4.1 Mention Grammar & Routing Table
1. Pre-process message markdown to mask fenced code blocks and inline backticks.
2. Match `(?:^|\s)@([a-zA-Z0-9_-]+)(?:\[([a-zA-Z0-9._-]+)(?::([a-zA-Z0-9_-]+))?\])?`:
   - `@codex` -> routes to `codex` using current/default model & effort.
   - `@codex[gpt-5.4:high]` -> optional power-user inline override setting `model = gpt-5.4`, `reasoning_effort = high` before prompting.
   - `@all` -> broadcasts sequentially or concurrently (read-only advisory mode unless confirmed) to all active session harnesses.
   - `@orchestrator` -> explicitly invokes the Orchestrator Harness.
3. **Unmentioned User Message**:
   - If `session.orchestrator_harness` is set -> routes to the Orchestrator Pipeline (Section 5).
   - Otherwise -> routes to `session.default_harness` (or prompts user in UI if multiple harnesses exist and none is default).
4. **Agent Output Mention Routing**:
   - When an agent finishes a turn (or calls `chat_send_message`), if its message addresses `@<peer_handle>` and `chain.hops_remaining > 0`, the Switch decrements `hops_remaining` and enqueues a turn for `<peer_handle>`.

### 4.2 Switch MCP Tools Exposed to Harnesses
Every harness receives these tools automatically via `meta-harness-switch`:

| Tool Name | Who Can Call | Parameters | Behavior |
| :--- | :--- | :--- | :--- |
| `chat_list_peers` | All harnesses | `{}` | Returns session members, handles, live status (`idle`/`working`), current model, and role (`orchestrator`/`executor`). |
| `chat_read_messages` | All harnesses | `{ limit?: u32, before_seq?: i64 }` | Fetches recent group chat messages for situational context. |
| `chat_send_message` | All harnesses | `{ to?: string[], content: string }` | Posts an intermediate or handoff message into the group chat and triggers `@to` recipients (consumes 1 hop). |
| `trajectory_list` | All harnesses | `{ harness?: string, limit?: u32 }` | Lists recent turns (`turn_id`, `harness`, `summary`, `touched_files`, `status`, `created_at`). |
| `trajectory_read` | All harnesses | `{ turn_id: string, detail: "summary" \| "tool_calls" \| "diffs" \| "full", page?: u32 }` | Reads a peer harness's trajectory at bounded token budgets (max 16 KB per page) so agents can inspect diffs, errors, or reasoning without context overflow. |
| `memory_recall` | All harnesses | `{ query?: string, path_prefix?: string, limit?: u32 }` | Queries high-level project episodic memory (`episodic_jobs` + `project_facts`) via SQLite FTS5. |
| `memory_record_fact` | All harnesses | `{ category: string, summary: string }` | Saves a concise architectural decision or ownership note to project memory. |
| `quota_get_status` | Orchestrator (or all) | `{}` | Returns normalized subscription usage & headroom across all harnesses. |
| `models_list_catalog` | Orchestrator | `{}` | Returns live ACP `configOptions` (`models`, `efforts`) merged with model strength profiles. |
| `orchestrator_dispatch` | Orchestrator | `{ dispatches: [{ target: string, model?: string, effort?: string, task_brief: string, mode?: "sequential" \| "parallel_readonly" }], rationale: string }` | Configures executor harness(es) via `session/set_config_option` and dispatches the structured task brief(s). |

---

## 5. Orchestrator & Subscription Quota Intelligence

### 5.1 Multi-Tier Subscription Usage Tracker
Because user-authenticated CLI harnesses expose usage differently, `mh-acp::quota` combines three non-intrusive signal sources into a unified `QuotaSnapshot` per harness:

```rust
pub struct QuotaSnapshot {
    pub harness_id: String,
    pub plan_label: Option<String>,          // e.g., "ChatGPT Plus", "Claude Max 5x", "Google AI Pro"
    pub window_5h_remaining_pct: Option<f32>,// 0.0..=100.0
    pub window_5h_resets_at: Option<i64>,
    pub weekly_remaining_pct: Option<f32>,   // 0.0..=100.0
    pub weekly_resets_at: Option<i64>,
    pub local_5h_tokens_est: u64,            // Measured from Meta-Harness turns + local CLI session logs
    pub rate_limited_until: Option<i64>,     // Set immediately on 429 / quota error
    pub source: QuotaSource,                 // LiveProbe | LocalLogEstimate | UserCalibrated | Cooldown429
}
```

1. **Local Log & Turn Accounting (Zero Overhead, Always Available)**:
   - Reads local session metadata where present (`~/.codex/sessions`, `~/.claude/projects`, `~/.gemini/`) and sums rolling 5-hour / 7-day turns & token estimates observed by Meta-Harness, normalized against user-configurable plan baselines.
2. **Cached Status Probe (Optional, TTL = 10 min)**:
   - Parses non-LLM status endpoints/commands when available without consuming prompt completions.
3. **Reactive 429 / Exhaustion Circuit Breaker**:
   - If an ACP `session/prompt` fails with a quota/rate-limit error, `QuotaTracker` immediately marks `rate_limited_until` and triggers automatic Orchestrator re-routing if mid-dispatch.

### 5.2 Complexity, Model & Effort Selection Matrix
To minimize latency and token overhead on unmentioned user turns, `mh-switch` pre-injects a compact `<orchestrator_telemetry>` block (~150–250 tokens) into the Orchestrator's prompt containing:
1. Live `QuotaSnapshot` summary for all enabled session harnesses.
2. Available `(model, reasoning_effort)` pairs per harness from ACP `configOptions` + capability tags (`strengths`, `relative_quota_cost`).
3. Top 5 relevant `episodic_jobs` from FTS5 matching keywords/paths in the user's message.

The Orchestrator applies four decision rules (codified in its role prompt and validated by `orchestrator_dispatch`):
- **Trivial / Quick Lookup / Small Edit**: Pick fast model + `low` reasoning effort on a harness with ample 5h headroom.
- **Standard Feature / UI Component / Test Suite**: Match domain affinity (e.g., frontend/UX -> `@claude`, systems/Rust/CLI -> `@codex`, broad cross-repo research -> `@antigravity`) + `medium` effort.
- **Hard Architecture / Concurrency Bug / Tricky Refactor**: Select top-tier model + `high` (or `xhigh`) reasoning effort, provided `window_5h_remaining_pct > 25%`; if primary harness is below `15%` quota, automatically shift to the highest-headroom capable peer and explain the substitution in the Dispatch Card.
- **Continuity Bias**: If `episodic_jobs` shows `@codex` just implemented module `X` in the previous job and has healthy quota, prefer `@codex` for follow-up fixes in `X` (leveraging warm ACP session context!).

---

## 6. High-Level Episodic Memory Engine (`mh-store`)

### 6.1 Design Scope
Memory strictly captures **who did what, where, and with what outcome**—never raw file contents or verbose tool outputs.

### 6.2 Automatic + Explicit Capture
When an executor turn completes:
1. **Deterministic Extraction (Zero LLM calls)**:
   - `executor_harness`, `model`, `reasoning_effort`
   - `task_brief` (from Orchestrator dispatch or user `@mention` prompt, truncated to 240 chars)
   - `outcome_summary` (first/final summary paragraph of agent reply, truncated to 400 chars)
   - `touched_paths` (deduplicated relative paths modified during the turn via `fs/write_text_file` or `git status --porcelain` delta)
   - `status` (`completed` | `cancelled` | `failed`)
2. **FTS5 Indexing**: Indexed over `(executor_harness, task_brief, outcome_summary, touched_paths_text)` for sub-millisecond recall.

### 6.3 SQLite Schema (`rusqlite` with `WAL` mode)

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

CREATE TABLE projects (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    root_path       TEXT NOT NULL UNIQUE,
    created_at      INTEGER NOT NULL,
    last_opened_at  INTEGER NOT NULL
);

CREATE TABLE sessions (
    id                      TEXT PRIMARY KEY,
    project_id              TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    title                   TEXT NOT NULL,
    orchestrator_harness    TEXT,
    orchestrator_model      TEXT,
    orchestrator_effort     TEXT,
    max_hops                INTEGER NOT NULL DEFAULT 6,
    permission_policy       TEXT NOT NULL DEFAULT 'prompt',
    working_summary         TEXT NOT NULL DEFAULT '',
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL
);

CREATE TABLE session_members (
    session_id              TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    harness_id              TEXT NOT NULL,
    acp_session_id          TEXT,
    active_model            TEXT,
    active_effort           TEXT,
    last_seen_msg_seq       INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, harness_id)
);

CREATE TABLE messages (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    id              TEXT NOT NULL UNIQUE,
    session_id      TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    chain_id        TEXT NOT NULL,
    sender_kind     TEXT NOT NULL, -- 'user' | 'harness' | 'orchestrator' | 'system'
    sender_id       TEXT NOT NULL,
    recipients_json TEXT NOT NULL DEFAULT '[]',
    kind            TEXT NOT NULL, -- 'text' | 'dispatch_card' | 'permission_card' | 'notice'
    content         TEXT NOT NULL,
    turn_id         TEXT,
    metadata_json   TEXT NOT NULL DEFAULT '{}',
    created_at      INTEGER NOT NULL
);
CREATE INDEX idx_messages_session_seq ON messages(session_id, seq);

CREATE TABLE turn_trajectories (
    turn_id         TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    harness_id      TEXT NOT NULL,
    model           TEXT,
    effort          TEXT,
    status          TEXT NOT NULL, -- 'running' | 'completed' | 'cancelled' | 'failed'
    summary         TEXT NOT NULL DEFAULT '',
    touched_files   TEXT NOT NULL DEFAULT '[]',
    started_at      INTEGER NOT NULL,
    finished_at     INTEGER
);

CREATE TABLE trajectory_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    turn_id         TEXT NOT NULL REFERENCES turn_trajectories(turn_id) ON DELETE CASCADE,
    event_kind      TEXT NOT NULL, -- 'thought' | 'plan' | 'tool_call' | 'diff' | 'terminal' | 'error'
    payload_json    TEXT NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE INDEX idx_traj_events_turn ON trajectory_events(turn_id, id);

CREATE TABLE episodic_jobs (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    session_id      TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    turn_id         TEXT,
    harness_id      TEXT NOT NULL,
    model           TEXT,
    effort          TEXT,
    task_brief      TEXT NOT NULL,
    outcome_summary TEXT NOT NULL,
    touched_paths   TEXT NOT NULL DEFAULT '[]',
    status          TEXT NOT NULL,
    pinned          INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL
);

CREATE VIRTUAL TABLE episodic_jobs_fts USING fts5(
    job_id UNINDEXED,
    project_id UNINDEXED,
    harness_id,
    task_brief,
    outcome_summary,
    touched_paths
);

CREATE TABLE project_facts (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    category        TEXT NOT NULL, -- 'architecture' | 'ownership' | 'convention' | 'constraint'
    summary         TEXT NOT NULL,
    author_id       TEXT NOT NULL,
    created_at      INTEGER NOT NULL
);
```

---

## 7. Backend-to-Frontend API & WebSocket Protocol (`mh-server`)

### 7.1 REST Endpoints
- `GET /api/health` — Server status, version, active subprocess count, RSS memory usage.
- `GET /api/projects` | `POST /api/projects` | `DELETE /api/projects/:id` — Manage project folders (includes local directory browser helper `GET /api/fs/list?path=...` restricted to local user drives).
- `GET /api/projects/:id/sessions` | `POST /api/projects/:id/sessions` — List/create sessions.
- `GET /api/sessions/:id` | `PATCH /api/sessions/:id` | `DELETE /api/sessions/:id` — Read/update session members, orchestrator harness, hop budget, permission policy.
- `GET /api/sessions/:id/messages?before_seq=&limit=100` — Paginated message history.
- `POST /api/sessions/:id/messages` — Post user message (initiates routing chain).
- `POST /api/sessions/:id/cancel` — Immediate Stop All (`session/cancel` across active harnesses).
- `POST /api/sessions/:id/permissions/:req_id` — Resolve a pending `session/request_permission` (`allow_once` | `allow_session` | `reject`).
- `GET /api/trajectories/:turn_id` — Fetch structured trajectory events & diffs for drawer inspection.
- `GET /api/projects/:id/memory` | `POST /api/projects/:id/memory/facts` | `PATCH /api/memory/jobs/:id` | `DELETE /api/memory/jobs/:id` — View/pin/edit/prune high-level episodic memory.
- `GET /api/harnesses` | `POST /api/harnesses/probe` | `GET /api/quotas` — Harness availability, discovered ACP `configOptions`, and subscription quota snapshots.

### 7.2 Multiplexed WebSocket (`/api/ws?session_id=...`)
Server-to-Client tagged JSON events (coalesced at 25ms intervals during streaming):
- `chat.message_created` / `chat.message_delta` / `chat.message_completed`
- `switch.dispatch_decided` (Orchestrator -> Executor card update)
- `switch.hop_limit_paused` (Prompt user to extend hop budget or stop)
- `harness.status_changed` (`spawning` | `idle` | `thinking` | `running_tool` | `waiting_permission` | `hibernated` | `error`)
- `trajectory.event` (Streamed only when client subscribes to `turn_id` or as lightweight summary counters `tool_count`, `edit_count` on main chat!)
- `permission.requested` / `permission.resolved`
- `quota.updated`
- `memory.job_recorded`

*(Optimization: Full `trajectory.event` payloads only stream over WS for turns the user currently has open in the Trajectory Drawer; closed turns only receive lightweight badge count updates, keeping WebSocket bandwidth and browser memory tiny!)*

---

## 8. Frontend UX & Performance Specification (`web/`)

### 8.1 Three-Column Adaptive Layout
1. **Left Rail (260px, Collapsible)**:
   - **Project Selector**: Dropdown + "Open Project Folder" modal showing git branch pill.
   - **Sessions List**: Group chat rooms under the current project with unread/running indicators.
   - **Harness Roster**: Cards for `@codex`, `@claude`, `@antigravity`, etc., showing:
     - Live status dot (`Idle`, `Working`, `Hibernated` to save RAM)
     - Crown toggle to assign **Orchestrator** role
     - Compact dual progress bar for **5h Quota** & **Weekly Quota**
2. **Center Canvas (Fluid)**:
   - **Session Header**: Session title, active Orchestrator badge (`Orchestrator: @antigravity`), hop counter when agents are conversing (`Chain Hop 2/6`), and prominent red **Stop All** button when any turn is active.
   - **Virtualized Group Chat Stream**:
     - Distinct color-coded harness badges & markdown rendering with syntax-highlighted code blocks.
     - **Orchestrator Dispatch Card**: Compact collapsible banner explaining why a specific executor + model + effort was selected.
     - **Trajectory Summary Pill** on every agent reply: e.g., `[14s · gpt-5.4 (high) · 4 tool calls · +48/-12 in 2 files -> Inspect]`.
     - **Inline Permission Card**: Clear diff/command preview with one-click `Allow Once`, `Allow for Session`, `Reject`.
   - **Smart Composer**:
     - Rich textarea with instant `@` autocomplete popup listing harnesses (plus inline model/effort picker chip if user wants manual override) and `#` project file path autocomplete.
     - Dynamic routing indicator above Send button:
       - If `@codex` typed: *"Direct to **@codex** (`gpt-5.4 · medium`)"*
       - If no `@mention`: *"Routed via Orchestrator (**@antigravity**) -> auto-selects executor, model & effort"*
3. **Right Inspector Drawer (420px, On-Demand)**:
   - Tab 1 — **Trajectory**: Live step-by-step timeline of thoughts, tool calls, terminal runs, and side-by-side/unified diffs for the selected agent turn, plus a "Peer Harnesses Reading This" indicator when another agent queries `trajectory_read`.
   - Tab 2 — **Memory Ledger**: Searchable high-level list of jobs ("`@codex` implemented auth JWT middleware · 3 files") and pinned architectural facts, with 1-click delete/pin/edit.
   - Tab 3 — **Switch & Quotas**: Detailed subscription usage calibration, model/effort overrides, hop limit slider, and custom ACP command editor.
