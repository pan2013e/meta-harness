# Meta-Harness Architecture & Module Map

Meta-Harness separates concerns across five strictly layered Rust crates (`Edition 2024`), a React 19 SPA (`web/`), an Electron shell (`desktop/`), and synchronization scripts (`scripts/`).

---

## 1. Rust Crate Dependency DAG

```text
mh-core       (domain DTOs, mention parser, skill metadata, WaitForGraph, AppPaths, config, token estimator)
  ▲
mh-store      (SQLite WAL + FTS5 schema migrations v1..v6, projects, sessions, messages, trajectories, memory, skill_states)
  ▲
mh-acp        (ACP JSON-RPC client & stdio supervisor, Git worktree broker, WorkspaceBroker permissions, quota probes, profiles)
  ▲
mh-switch     (SwitchEngine router, hop guard, peer_consult RPC, orchestrator telemetry/curation, built-in skills sync, MCP tools)
  ▲
mh-server     (Axum REST + /api/ws hub, bearer auth, PTY WebShell, sandboxed web_preview, CLI & `mcp-bridge`, RustEmbed SPA, tray)
```

Never introduce upward or cyclic crate dependencies.

---

## 2. Key Rust Modules & Responsibilities

### `crates/mh-core/src/`
- `domain.rs` — Wire DTOs shared with `web/src/types/api.ts`: `Project`, `Session`, `SessionMember`, `GlobalPersona`, `PermissionRule`, `ChatMessage` (`MessageKind`), `TurnTrajectory`, `TrajectoryEvent`, `QuotaSnapshot`, `AdapterModelCatalog`, `StorageSummary`, `SessionContextReport`, `PendingPermissionView`, `WsEvent`.
- `skills.rs` — `SkillSource`, `SkillSummary`, `SkillDetail`, `parse_skill_markdown`, `sanitize_skill_slug`, `RESERVED_SLASH_COMMANDS`, `scan_skill_dir`, `compute_skill_tree_hash`.
- `mention.rs` — `mask_code_regions`, `parse_mentions`, `parse_user_mentions` (skips leading workflow/skill `/tokens` before resolving leading `@recipients` + `@all`/`@orchestrator` and `[model:effort]` qualifiers).
- `dag.rs` — `WaitForGraph` tracking active `caller -> target` consult edges to detect and reject deadlocks.
- `paths.rs` — `AppPaths::resolve()` (`MH_CONFIG_DIR`, `MH_DATA_DIR`, `MH_WORKSPACE_DIR`, `skills_dir()`), `expand_user_path`, `dunce::canonicalize`-based `normalize_path` / `normalize_for_create` / `is_path_within`.
- `config.rs` & `tokens.rs` — `harnesses.toml` loader + `gpt-6*` 1M-window & token estimator.

### `crates/mh-store/src/`
- `migrations.rs` — SQLite `WAL` + `NORMAL` sync + foreign keys. Schema versions:
  - **v1:** `projects`, `sessions`, `session_members`, `session_permission_rules`, `messages`, `turn_trajectories`, `trajectory_events`, `episodic_jobs`, `episodic_jobs_fts` (FTS5), `project_facts`, `adapter_quota_prefs`.
  - **v2:** `adapter_model_prefs`.
  - **v3:** nullable `sessions.project_id` (Quick Sessions) + `archived_at`.
  - **v4:** `global_personas`.
  - **v5:** `session_harness_conversations` + trigger tracking native harness conversation IDs across resets/archives.
  - **v6:** `skill_states` (`state_key`, `source`, `origin`, `enabled`, `builtin_hash`, `updated_at`).
- `sessions.rs`, `messages.rs`, `trajectories.rs`, `memory.rs`, `settings.rs`, `skills.rs`, `projects.rs` — CRUD & sanitized FTS5 queries.

### `crates/mh-acp/src/`
- `protocol.rs` & `client.rs` — ACP stdio JSON-RPC 2.0 client (`initialize`, `session/new`, `session/load`, `session/set_config_option`, `session/prompt`, `session/cancel`, `usage_update`, 25 ms delta coalescing, child-session isolation).
- `supervisor.rs` — Lazy spawning keyed by `(session_id, member_handle)`, idle hibernation reaper (`idle_process_ttl_secs`), catalog cache, `run_ephemeral_prompt` (for auto-titling & intent classification), and native conversation archive/restore/delete hooks.
- `broker.rs` — `WorkspaceBroker` handling `fs/read_text_file`, `fs/write_text_file`, `terminal/*`, `session/request_permission` (auto-allowing member worktree edits, project reads, read-only `skill_read_roots`, saved session patterns, and orchestrator/member auto-approved MCP switch tools).
- `worktree.rs` — Integration & lazy executor worktree lifecycle, auto-commit, fast-forward/merge/conflict detection, peer sync, `merge`/`squash`/`patch` promotion, checkpointing, and pruning.
- `quota.rs`, `profile.rs`, `maintenance.rs`, `platform.rs`, `mock_agent.rs` — Quota probes (`codex app-server` / OAuth, Claude OAuth / Keychain, Antigravity CLI), CLI version/upgrade/sign-in jobs, isolated `profile_dir`, Win32 Job Objects / POSIX `killpg`, deterministic `internal-mock-agent`.

### `crates/mh-switch/src/`
- `router.rs` — `SwitchEngine`, `SessionRuntime`, parallel turn queueing, delta context formatting (`<group_chat_updates>`, `<available_skills>`, `<activated_skills>`, `<orchestrator_telemetry>`, `<slash_commands>`), post-turn Git integration, conflict escalation, memory curation, 429 failover, context usage reporting, `/compact`, auto-title, `<250 ms` `cancel_session`.
- `skills.rs` — Embedded `meta-harness-docs` hash-checked synchronization (`ensure_builtin_skills`), reset helper, global + `.agents/skills/` discovery, skill read & deduplicated `ActivityPill` emission.
- `mcp.rs` & `orchestrator.rs` — MCP tool schemas (`list_switch_tools`) & dispatch (`execute_mcp_tool`), telemetry formatting, and curation prompts.

### `crates/mh-server/src/`
- `lib.rs` — `WebAssets` (`RustEmbed` of `../../web/dist`) and Axum router mounting `/api/*`, `/preview/{session_id}/{grant}/{*path}`, `/internal/mcp`, and SPA fallback.
- `routes.rs`, `skills.rs`, `settings.rs`, `harness_jobs.rs` — REST handlers, skill Git/local/custom installers, storage breakdown & cleanup.
- `mcp_bridge.rs`, `pty.rs`, `web_preview.rs`, `ws.rs`, `auth.rs`, `tray.rs`, `main.rs` — Stdio `mcp-bridge`, portable PTY WebSocket, signed iframe previews with strict CSP, WebSocket hub, LAN cookie/bearer auth, OS tray, CLI entrypoint.

---

## 3. Frontend (`web/src/`) & Electron (`desktop/`)

- `api/client.ts` & `types/api.ts` — Typed `api.*` wrapper, bearer token attachment, `WsHubClient` auto-reconnect.
- `store/useAppStore.ts` — Normalized Zustand state (theme, sidebar, projects, sessions, members, deltas, trajectories, quotas, memory, skills, storage, WS dispatch).
- `components/sidebar/Sidebar.tsx` — Project switcher (plus Quick Sessions), active/archived sessions, right-click Rename/Archive/Delete, Initialize Git.
- `components/chat/`:
  - `Composer.tsx` & `MentionEditor.tsx` — ContentEditable atomic pills for `@handles`, workflow/action `/commands`, and `/skills`; model/effort popover; Worktree/Strict/Auto policy; quota & context rings.
  - `MessageRow.tsx` & `TurnActivityTimeline.tsx` — Virtualized messages, Dispatch/Permission cards, collapsible `Sync Consult` threads, `Worked · N steps` timelines, skill & trajectory `ActivityPill`s.
  - `WebShellDrawer.tsx` — Multi-tab xterm.js PTY drawer.
- `components/modals/` — `SettingsDialog.tsx` (`Appearance`, `Harnesses & Models`, `Skills` -> `SkillsSettings.tsx`, `Global Personas` -> `GlobalPersonas.tsx`, `Session`, `Storage & Data` -> `SettingsBackup.tsx`, `About`), `DiffReviewModal.tsx`, `FilePreviewModal.tsx`, `FolderPickerModal.tsx`, `MemoryModal.tsx`.
- `desktop/` — Electron main/preload scripts spawning `meta-harness --desktop-child` with ephemeral `MH_DESKTOP_TOKEN` passed securely via environment (stripped before spawning child agents/shells).
