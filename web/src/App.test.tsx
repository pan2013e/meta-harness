import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { MarkdownBlock } from "./components/common/MarkdownBlock";
import { QuotaMeter } from "./components/common/Badges";
import { DiffViewer } from "./components/common/DiffViewer";
import { MessageRow } from "./components/chat/MessageRow";
import { groupTrajectoryEvents } from "./components/drawer/trajectory";
import { TrajectoryRow } from "./components/drawer/TrajectoryRow";
import { api, WsHubClient } from "./api/client";
import { useAppStore } from "./store/useAppStore";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  useAppStore.setState(useAppStore.getInitialState(), true);
});

describe("Session navigation", () => {
  it("keeps the most recently selected session when older requests finish last", async () => {
    let releaseFirst!: () => void;
    const first = new Promise<void>((resolve) => { releaseFirst = resolve; });
    vi.spyOn(api, "getSession").mockImplementation(async (id) => {
      if (id === "first") await first;
      return {
        session: {
          id, project_id: "project", title: id, isolation_mode: "shared_cwd",
          base_branch: null, base_commit_sha: null, integration_branch: null,
          integration_wt_path: null, orchestrator_member: null,
          confirm_before_dispatch: false, max_hops: 6, permission_policy: "worktree_smart",
          working_summary: "", created_at: 0, updated_at: 0,
        },
        members: [], statuses: {}, permission_rules: [], pending_permissions: [],
      };
    });
    vi.spyOn(api, "listMessages").mockResolvedValue({ messages: [] });
    vi.spyOn(api, "listTrajectories").mockResolvedValue({ trajectories: [] });
    vi.spyOn(api, "getSessionDiff").mockResolvedValue({ diff: null });

    const selectSession = useAppStore.getState().selectSession;
    const oldSelection = selectSession("first");
    await selectSession("second");
    releaseFirst();
    await oldSelection;

    expect(useAppStore.getState().activeSession?.id).toBe("second");
    expect(api.getSessionDiff).toHaveBeenCalledExactlyOnceWith("second");
  });

  it("applies session and trajectory subscriptions queued during the socket handshake", () => {
    const send = vi.fn();
    const close = vi.fn();
    const socket = { readyState: 0, send, close, onopen: () => {} };
    vi.stubGlobal("WebSocket", Object.assign(function () { return socket; }, { OPEN: 1 }));
    const client = new WsHubClient(vi.fn());
    client.connect(null);
    client.setSession("session");
    client.subscribeTrajectory("turn");
    socket.readyState = 1;
    socket.onopen();

    expect(send.mock.calls.map(([frame]) => JSON.parse(frame))).toEqual([
      { type: "set_session", session_id: "session" },
      { type: "subscribe_trajectory", turn_id: "turn" },
    ]);
    client.disconnect();
    expect(close).toHaveBeenCalledOnce();
  });
});

describe("UI primitives", () => {
  it("renders markdown code fences without highlighting internal @mentions as mentions", () => {
    const md =
      "## Summary\n\nPing @codex outside.\n- Item one\n```python\n@decorator\ndef foo(): pass\n```";
    render(<MarkdownBlock content={md} />);
    expect(screen.getByText("Summary")).toBeTruthy();
    expect(screen.getByText("@codex")).toBeTruthy();
    expect(screen.getByText(/@decorator/)).toBeTruthy();
  });

  it("renders quota meter percentage, exhausted, n/a, and 429 cooldown states", () => {
    const { rerender } = render(<QuotaMeter label="5h" pct={72} />);
    expect(screen.getByText("72%")).toBeTruthy();

    rerender(<QuotaMeter label="Weekly" pct={null} />);
    expect(screen.getByText("n/a")).toBeTruthy();

    rerender(<QuotaMeter label="5h" pct={0} rateLimited />);
    expect(screen.getByText("429 COOLDOWN")).toBeTruthy();
  });

  it("renders unified diffs grouped by file", () => {
    const diff =
      "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,2 +1,2 @@\n-let x = 1;\n+let x = 2;";
    render(<DiffViewer diffText={diff} />);
    expect(screen.getByText("src/main.rs")).toBeTruthy();
    expect(screen.getByText("+1")).toBeTruthy();
    expect(screen.getByText("-1")).toBeTruthy();
  });

  it("shows hunk lines that start like file headers", () => {
    render(<DiffViewer diffText={[
      "diff --git a/notes.txt b/notes.txt", "--- a/notes.txt", "+++ b/notes.txt",
      "@@ -1 +1 @@", "--- deleted text", "+++ b/added text",
    ].join("\n")} />);
    expect(screen.getByText("notes.txt")).toBeTruthy();
    expect(screen.getByText("--- deleted text")).toBeTruthy();
    expect(screen.getByText("+++ b/added text")).toBeTruthy();
    expect(screen.getByText("+1")).toBeTruthy();
    expect(screen.getByText("-1")).toBeTruthy();
  });

  it("presents the user's message on the right side of the conversation", () => {
    render(<MessageRow consultReplies={[]} message={{
      id: "message", seq: 1, session_id: "session", chain_id: "chain", parent_msg_id: null,
      sender_kind: "user", sender_id: "user", recipients: ["codex"], kind: "text",
      content: "Check the workspace", turn_id: null, metadata: {}, created_at: 0,
    }} />);
    expect(screen.getByRole("article", { name: "Your message" }).classList.contains("justify-end")).toBe(true);
    expect(screen.getByText("Check the workspace")).toBeTruthy();
  });

  it("groups stored Codex terminal chunks and ACP status into one readable tool row", () => {
    const entries = groupTrajectoryEvents([
      { id: 1, turn_id: "turn", created_at: 0, event_kind: "tool_call", payload: { toolCallId: "exec", kind: "execute", title: "git status" } },
      { id: 2, turn_id: "turn", created_at: 1, event_kind: "tool_call", payload: { toolCallId: "exec", _meta: { terminal_output_delta: { terminal_id: "exec", data: "On branch " } } } },
      { id: 3, turn_id: "turn", created_at: 2, event_kind: "terminal", payload: { tool_call_id: "exec", terminal_id: "exec", output_delta: "main" } },
      { id: 4, turn_id: "turn", created_at: 3, event_kind: "tool_call", payload: { toolCallId: "exec", status: "completed", _meta: { terminal_exit: { exit_code: 0 } } } },
    ]);
    expect(entries).toHaveLength(1);
    render(<TrajectoryRow entry={entries[0]} />);
    expect(screen.getByText("git status")).toBeTruthy();
    expect(screen.getByText("Exit 0")).toBeTruthy();
    expect(screen.getByText("On branch main")).toBeTruthy();
  });
});
