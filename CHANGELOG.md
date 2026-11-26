# Changelog

## Unreleased

## 0.6.0

- Check for GitHub release updates in Settings → About. Verify desktop downloads and hand installation to the Windows installer or macOS disk image.
- Add `meta-harness update`, `install`, and `uninstall` for standalone release binaries. Stage and verify updates before replacing the installed binary.
- Build installer and standalone update assets for Windows x64 and macOS Apple Silicon and Intel. Publish the GitHub release after all platform builds succeed.

- Keep the composer and Skills settings in sync when switching projects or opening files quickly.
- Protect installed skills from duplicate imports and validate a bundle before installing its contents.
- Keep skill discovery within its designated directories and check every file in a brokered tool approval.
- Finish pending ACP requests when a harness shuts down. Clean up timed-out peer consultations before accepting another turn.
- Allow a brokered terminal to stop while another request is waiting for its exit. Keep the MCP bridge reachable on loopback when the web server binds to a LAN address.
- Preserve staged files when creating the first session worktree. Keep cache cleanup away from unrelated folders and workspaces being created.
- Restore partially streamed replies after reconnecting or returning to a session, without duplicating text.
- Cancel active turns before resetting harnesses during context compaction, and apply session metadata updates atomically.

## 0.5.0

- Add Agent Skills support across group chats (`SKILL.md` bundles in `<MH_DATA_DIR>/skills/` plus repo-local `.agents/skills/` discovery, `/skill-name` composer pills, auto-approved `skill_list` / `skill_read` MCP tools & read-only brokered file access, and inline skill activity pills).
- Add **Settings → Skills** to install skills from local folders, Git / GitHub URLs, or inline authoring, plus enable/disable toggles, file inspection/editing, built-in reset, and uninstall.
- Bundle and auto-synchronize the preinstalled **`Meta-Harness Docs`** (`/meta-harness-docs`) skill with full usage, crate architecture, ACP/worktree invariant, and contributor self-bootstrapping guides.

## 0.4.0

- Archive, restore, and delete linked Codex and compatible Claude Code conversations in their original profiles. Preserve conversation references across context resets and profile changes, and report when a harness keeps its own history.
- Distinguish harness-reported context usage from estimates and use the reported window for the active model.
- Open Project Memory with `/memory` even when a Quick Session or no harness is active.
- Summarize internal switch calls in the activity timeline and keep native child-session updates separate from the parent turn.
- Add an illustrated usage guide and a README walkthrough of multi-agent collaboration.

## 0.3.0

- Route leading recipient lists directly and resolve ambiguous inline references with a hidden orchestrator pass.
- Show shared Antigravity CLI usage by default with a note when a private profile is active.
- Give Worktree, Strict, and Auto their own approval-mode icons.
- Show one compact waiting indicator and display approval cards as soon as a request arrives.
- Automatically grant one-time approval to registered switch MCP tools during orchestrator turns. Session dispatch confirmation continues to apply when enabled.

## 0.2.0

- Render HTML files inside the file preview, including relative styles and scripts.
- Open the browser when the standalone CLI starts on loopback.
- Show connection, workspace, model catalog, and subscription loading status during startup.
- Show the GitHub repository and six-character build commit in About.
- Refresh the workspace documentation and retire the completed implementation plan.

## 0.1.0

- Added collaborative ACP sessions with worktree isolation, mention routing, trajectories, and project memory.
- Added provider quotas, separate harness profiles, global personas, and portable settings backups.
- Added Electron and standalone tray releases, in-page file previews, a persistent terminal drawer, and cross-platform CI.
