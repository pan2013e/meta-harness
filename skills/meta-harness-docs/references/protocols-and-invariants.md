# Protocols & Hard Architectural Invariants

Any code change to Meta-Harness **must preserve every invariant** defined in `AGENTS.md` and `docs/SPECIFICATION.md`.

---

## 1. Agent Client Protocol (ACP) Discipline

- **Structured JSON-RPC 2.0 Only:** Never scrape raw PTY terminal text for agent turns. Communicate exclusively via ACP methods: `initialize`, `session/new`, `session/load`, `session/set_config_option`, `session/prompt`, `session/cancel`, `session/update`, `session/request_permission`, `fs/*`, and `terminal/*`.
- **Stream Coalescing & Child-Session Isolation:** Batch rapid `session/update` text deltas (~25 ms window) before broadcasting WebSocket `chat.message_delta` events and persisting to SQLite. Subagent/child `sessionId` notifications (`params.sessionId != root_acp_session_id`) must NOT overwrite the parent member's `last_usage` or transcript.
- **Resumption Safety:** Only reuse `acp_session_id` via `session/load` when both the resolved working directory (`worktree_path`) and normalized `profile_dir` match (`resume_profile_session`), avoiding stale cross-profile/cross-worktree session leakage.

---

## 2. Single-Binary MCP Switch & Loop / Deadlock Guards

- **Zero External Script Runtimes:** `meta-harness mcp-bridge` runs inside the Rust binary and forwards stdio JSON-RPC to a separate loopback listener (`POST /internal/mcp`). Its `MH_BRIDGE_TOKEN` is scoped to the active session member. Keep the bridge off the public listener.
- **Hop Guard (`max_hops`):** Every user message creates a `chain_id` with `max_hops` (default `6`, tunable `2–16`). Agent-initiated `@mentions`, `chat_send_message`, and `peer_consult` increment `hops_used`; reaching `max_hops` pauses further routing and emits `switch.hop_limit_paused` so the UI offers **Continue**.
- **Wait-For Graph Deadlock Prevention:** Before blocking on `peer_consult(caller -> target)`, check `WaitForGraph::would_create_cycle(caller, target)`. Reject cycles immediately with structured MCP guidance advising asynchronous `chat_send_message` instead.
- **Prompt Cancellation (`< 250 ms`):** `POST /api/sessions/{id}/cancel` bumps `cancel_epoch`, cancels active chains and pending permission/consult waiters, clears `WaitForGraph`, and issues ACP `session/cancel` across session clients.

---

## 3. Mention Parsing & Leading Slash Commands

- **Markdown Masking:** `mh-core::mention::mask_code_regions` masks blockquotes, fenced code blocks, and inline code spans before extracting mentions or skill directives. Email addresses and scoped package names stay out of routing.
- **Recipient Separation:** Leading slash commands/skills (e.g., `/plan /meta-harness-docs @codex ...`) strip trailing punctuation before parsing leading `@recipients`; body references (`@codex say hello to @antigravity`) stay in the prompt rather than spawning extra targets simultaneously.

---

## 4. Git Worktrees, Permissions, & Skill Sandboxing

- **Main Checkout Untouched:** Keep the user's primary repo checkout untouched until explicit promotion (`merge`, `squash`, `patch` — each holding `SessionRuntime::git_merge_lock`). Clean merges integrate without an LLM call; merge conflicts escalate to the session Orchestrator in `_integration` with peer trajectories and conflict markers.
- **Managed Git:** Use the worktree broker for Git operations. Its commands suppress hooks and filesystem monitor callbacks. Diffs disable external diff tools and text conversion.
- **Canonical Path Containment:** Always resolve filesystem targets via `expand_user_path`, `dunce::canonicalize` / `normalize_for_create`, and `is_path_within` (preventing `..` traversal, symlink escape, and Windows verbatim `\\?\` prefixes that break Git and Node).
- **Read-Only Skills Sandbox:**
  - `skill_read_roots` (`<MH_DATA_DIR>/skills`, `<worktree>/.agents/skills`, `<project_root>/.agents/skills`) permit **reads only** (`fs/read_text_file`, `kind == "read"` permission fast-path, `skill_list` / `skill_read` MCP tools) without user prompts.
  - **Never** grant unprompted **writes** outside `member_worktree`! `handle_write_text_file` and edit permissions outside `member_worktree` remain gated.
- **Preview Security Boundary:** `/preview/{session_id}/{grant}/{*path}` validates a 30-minute signed grant scoped to the session and opened HTML site. Canonical paths must remain inside the workspace. Shared previews allow the site and static assets, and enforce a 24 MiB limit and sandbox CSP.

---

## 5. Cross-Platform Process & Error Hygiene

- **Privilege Boundary:** Check the current process credentials before starting the backend, opening data, or launching agents. Reject Unix root and elevated Windows tokens.
- **LAN Authentication:** Require HTTPS on direct LAN listeners. Browser sessions use expiring signed cookies scoped to the request authority, and upgraded chat and terminal connections observe the same deadline.
- **Process Tree Isolation:** Every spawned CLI harness, MCP command, terminal command, and background `git` operation must go through platform isolation (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` on Windows, `process_group(0)` + `killpg` on Unix, `PR_SET_PDEATHSIG` on Linux) and resolve executables via `which` (`mh-acp::platform`) so zero orphan processes remain on exit or cancel.
- **Directory Conventions:** Resolve paths via `AppPaths::resolve()` (`dirs` + `MH_CONFIG_DIR` / `MH_DATA_DIR` / `MH_WORKSPACE_DIR`). Clean worktrees cleanly (`git worktree remove --force`) on session archive/delete.
- **No Panics on Runtime Data:** Use `thiserror` in library crates (`mh-core`, `mh-store`, `mh-acp`, `mh-switch`) and `anyhow` in `mh-server`. Never `.unwrap()` or `.expect()` on I/O, Git output, SQLite input, or third-party ACP JSON in non-test production code.
