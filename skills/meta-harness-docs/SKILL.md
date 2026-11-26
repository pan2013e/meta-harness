---
name: meta-harness-docs
title: Meta-Harness Docs
description: Comprehensive Meta-Harness usage guide, multi-agent group-chat and MCP collaboration workflows, crate/frontend architecture, ACP/worktree/permission invariants, and contributor self-bootstrapping & verification gates. Activate when using, configuring, debugging, extending, or developing Meta-Harness itself.
---

# Meta-Harness Docs

**Meta-Harness** is a local-first workspace where a developer collaborates side-by-side with **Codex**, **Claude Code**, **Antigravity / Gemini**, and custom **Agent Client Protocol (ACP)** harnesses inside a unified group chat backed by isolated Git worktrees.

Use this skill whenever the user asks about Meta-Harness workflows, configuration, multi-agent coordination, internal architecture, bug fixes, or adding new features inside the Meta-Harness repository itself.

---

## Core Capabilities at a Glance

- **Per-Session Integration & Executor Worktrees**: Git-backed sessions create an integration branch (`mh/<session_id>/integration` under `<repo>/.meta-harness/worktrees/<session_id>/_integration`) and lazily spin up per-executor checkouts (`mh/<session_id>/exec-<handle>` under `_exec_<handle>`). Your main checkout stays clean until you click **Review & Promote** (`merge`, `squash`, or `patch`). Non-Git folders offer **Initialize Git** or serialized **Shared Folder** mode; **Quick Sessions** allocate Git-initialized scratch repos under `~/.meta-harness/workspaces/<session_id>`.
- **`HarnessAdapter` vs. `SessionMember`**: Each adapter (`codex`, `claude`, `antigravity`, or custom in `harnesses.toml`) tracks a CLI driver, discovered `configOptions` (models + reasoning efforts), sign-in profile (`default` or isolated `profile_dir`), and shared subscription quota pool. Session members (`@handle`) bind a role (`orchestrator`, `executor`, `both`), optional persona prompt, model/effort override, worktree, and `last_seen_msg_seq`.
- **Parallel ACP + Single-Binary MCP Switch**: Harnesses communicate over ACP stdio JSON-RPC 2.0 and connect to the embedded MCP server (`meta-harness mcp-bridge`) for peer coordination (`chat_send_message`, cycle-checked synchronous `peer_consult`, `trajectory_read`, `memory_recall`, `orchestrator_dispatch`, `skill_list`, `skill_read`).
- **Agent Skills**: Standard `SKILL.md` bundles stored in `<MH_DATA_DIR>/skills/` or repo-local `.agents/skills/` are advertised to all harnesses (`<available_skills>`), invocable manually via `/skill-name` composer pills, or autonomously via `skill_list`/`skill_read` and read-only brokered file reads.
- **Built-in Developer Tools**: Interactive PTY **WebShell** (`Ctrl+\``), sandboxed in-app **File Preview** (HTML, Monaco source code, images, PDF), **Project Memory** (SQLite WAL + FTS5), cumulative **Diff Review & Promote**, storage cleanup, and Web/Electron/Tray distributions.

---

## Progressive Disclosure Reference Map

Read the targeted reference guide under this skill's directory (via `skill_read` MCP tool with `skill: "meta-harness-docs"` and `path`, or direct read of the skill path):

1. **[references/usage-and-workflows.md](references/usage-and-workflows.md)** — Day-to-day workflows & UI guide:
   - Workspaces, Quick Sessions, Archive/Restore/Delete + Codex/Claude conversation sync
   - `@mention` & Orchestrator routing, `orchestrator_dispatch`, `peer_consult`, `trajectory_read`, Activity Timelines
   - Composer pills (`@handle`, `/plan` `/review` `/test` `/fix`, `/compact` `/diff` `/shell` `/memory` `/new` `/archive`, `/skill-name`), model/effort overrides, approval modes (`Worktree`, `Strict`, `Auto`), and shortcuts (`Alt+A`, `Alt+R`, `Ctrl+B`, `Ctrl+\``)
   - Settings panels, desktop clipboard and quote menus, release updates, and HTTPS LAN pairing

2. **[references/architecture-and-crates.md](references/architecture-and-crates.md)** — Repository layout, crate dependency DAG, and symbol map:
   - Strictly unidirectional Rust crates: `mh-core` -> `mh-store` -> `mh-acp` -> `mh-switch` -> `mh-server`
   - SQLite schema & migrations (`v1`–`v6`), REST routes & `/api/ws` event types
   - React 19 + TypeScript + Tailwind v4 + Zustand (`web/src/*`) and Electron launcher (`desktop/`)

3. **[references/protocols-and-invariants.md](references/protocols-and-invariants.md)** — Mandatory architectural contracts (never violate):
   - ACP stdio JSON-RPC lifecycle, 25 ms stream coalescing, child-session isolation
   - Wait-For Graph deadlock detection, hop guard (`max_hops`), `<250 ms` `Stop All` cancellation
   - Mention code-block masking, cross-platform process tree cleanup (Win32 Job Objects / POSIX process groups / Linux `PR_SET_PDEATHSIG`), `dunce` path normalization, `worktree_smart` & read-only skills containment, and signed iframe preview grants

4. **[references/dev-testing-and-release.md](references/dev-testing-and-release.md)** — Bootstrapping Meta-Harness development inside Meta-Harness:
   - Windows (MSVC/WinLibs), macOS, and Linux toolchain setup
   - Safe self-hosting rules when developing Meta-Harness in its own worktrees (avoid killing your own running binary/port!)
   - Offline deterministic testing with `mock-acp-agent` and `mh-test-fixture` (`[MOCK:*]` tags)
   - Required Rust (`fmt`, `clippy`, `test`) & Web (`typecheck`, `test`, `build`) quality gates, `node scripts/version.mjs`, `CHANGELOG.md`, and `node scripts/build.mjs`

---

## Self-Bootstrapping Quick Checklist (Developing Meta-Harness with Meta-Harness)

1. Verify that stable Rust, Node.js 24, npm, and Git are installed. Follow the `AGENTS.md` at the root of the Meta-Harness checkout.
2. When building/testing the web UI + backend together, build `web/` first (`npm --prefix web run build`) before `cargo test` / `cargo build` so `RustEmbed` in `crates/mh-server` picks up updated static assets.
3. Preserve unidirectional crate boundaries, handle runtime I/O and external JSON errors, and test backend logic offline with the mock ACP subprocess fixtures.
4. Before committing, bump versions together with `node scripts/version.mjs <patch|minor|major>` and verify them with `node scripts/version.mjs --check`. Update `CHANGELOG.md` and run the Rust and frontend checks.
