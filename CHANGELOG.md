# Changelog

## Unreleased

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
