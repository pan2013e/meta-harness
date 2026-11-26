import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MarkdownBlock } from "./components/common/MarkdownBlock";
import { formatCreditsDisplay, QuotaMeter } from "./components/common/Badges";
import { DiffViewer } from "./components/common/DiffViewer";
import { ChatPane } from "./components/chat/ChatPane";
import { Composer } from "./components/chat/Composer";
import { MentionEditor, serializeMentionDom } from "./components/chat/MentionEditor";
import { MessageRow } from "./components/chat/MessageRow";
import { DiffReviewModal } from "./components/modals/DiffReviewModal";
import { SettingsDialog } from "./components/modals/SettingsDialog";
import { Sidebar } from "./components/sidebar/Sidebar";
import { cleanWorktreePath, groupTrajectoryEvents } from "./components/drawer/trajectory";
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
    const first = new Promise<void>((resolve) => {
      releaseFirst = resolve;
    });
    vi.spyOn(api, "getSession").mockImplementation(async (id) => {
      if (id === "first") await first;
      return {
        session: {
          id,
          project_id: "project",
          title: id,
          isolation_mode: "shared_cwd",
          base_branch: null,
          base_commit_sha: null,
          integration_branch: null,
          integration_wt_path: null,
          orchestrator_member: null,
          confirm_before_dispatch: false,
          max_hops: 6,
          permission_policy: "worktree_smart",
          working_summary: "",
          created_at: 0,
          updated_at: 0,
        },
        members: [],
        statuses: {},
        permission_rules: [],
        pending_permissions: [],
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
    vi.stubGlobal(
      "WebSocket",
      Object.assign(function () {
        return socket;
      }, { OPEN: 1 })
    );
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

  it("replaces streaming content when a new step starts after tool calls", () => {
    useAppStore.setState({
      activeSession: {
        id: "s_1",
        project_id: "p_1",
        title: "Test",
        base_branch: null,
        base_commit_sha: null,
        integration_branch: null,
        integration_wt_path: null,
        isolation_mode: "shared_cwd",
        orchestrator_member: null,
        confirm_before_dispatch: false,
        max_hops: 6,
        permission_policy: "worktree_smart",
        working_summary: "",
        created_at: 0,
        updated_at: 0,
      },
      messages: [
        {
          id: "m_1",
          seq: 1,
          session_id: "s_1",
          chain_id: "c_1",
          parent_msg_id: null,
          sender_kind: "member",
          sender_id: "codex",
          recipients: [],
          kind: "text",
          content: "Checking workspace...",
          turn_id: "t_1",
          metadata: { streaming: true },
          created_at: 0,
        },
      ],
    });

    useAppStore.getState().handleWsEvent({
      type: "chat.message_delta",
      session_id: "s_1",
      message_id: "m_1",
      turn_id: "t_1",
      member_handle: "codex",
      delta: "Final summary ready.",
      replace: true,
    });

    expect(useAppStore.getState().messages[0].content).toBe("Final summary ready.");
  });
});

describe("UI primitives", () => {
  it("renders markdown code fences without highlighting internal @mentions, plus <details> & preview links", () => {
    const md =
      "## Summary\n\nPing @codex outside.\n[Open the page](C:/Users/demo/.meta-harness/worktrees/s_1/_integration/index.html)\n<details><summary>Prompt</summary>Body text</details>\n```python\n@decorator\ndef foo(): pass\n```";
    render(<MarkdownBlock content={md} sessionId="s_1" />);
    expect(screen.getByText("Summary")).toBeTruthy();
    expect(screen.getByText("@codex")).toBeTruthy();
    expect(screen.getByText(/@decorator/)).toBeTruthy();
    expect(screen.getByText("Prompt")).toBeTruthy();
    const link = screen.getByText("Open the page").closest("a");
    expect(link?.getAttribute("href")).toBe("/api/sessions/s_1/files/index.html");
  });

  it("renders quota meter percentage, exhausted, n/a, and 429 cooldown states", () => {
    const { rerender } = render(<QuotaMeter label="5h" pct={72} />);
    expect(screen.getByText("72%")).toBeTruthy();

    rerender(<QuotaMeter label="Weekly" pct={null} />);
    expect(screen.getByText("n/a")).toBeTruthy();

    rerender(<QuotaMeter label="5h" pct={0} rateLimited />);
    expect(screen.getByText("429 COOLDOWN")).toBeTruthy();
  });

  it("renders unified diffs grouped by file and binary notices", () => {
    const diff = [
      "diff --git a/src/main.rs b/src/main.rs",
      "--- a/src/main.rs",
      "+++ b/src/main.rs",
      "@@ -1,2 +1,2 @@",
      "-let x = 1;",
      "+let x = 2;",
      "diff --git a/poster.webp b/poster.webp",
      "Binary files a/poster.webp and b/poster.webp differ",
    ].join("\n");
    render(<DiffViewer diffText={diff} />);
    expect(screen.getByText("src/main.rs")).toBeTruthy();
    expect(screen.getByText("poster.webp")).toBeTruthy();
    expect(screen.getByText("binary")).toBeTruthy();
  });

  it("shows hunk lines that start like file headers", () => {
    render(
      <DiffViewer
        diffText={[
          "diff --git a/notes.txt b/notes.txt",
          "--- a/notes.txt",
          "+++ b/notes.txt",
          "@@ -1 +1 @@",
          "--- deleted text",
          "+++ b/added text",
        ].join("\n")}
      />
    );
    expect(screen.getByText("notes.txt")).toBeTruthy();
    expect(screen.getByText("--- deleted text")).toBeTruthy();
    expect(screen.getByText("+++ b/added text")).toBeTruthy();
    expect(screen.getByText("+1")).toBeTruthy();
    expect(screen.getByText("-1")).toBeTruthy();
  });

  it("presents the user's message on the right and collapsible worked steps for multi-step agent turns", () => {
    render(
      <MessageRow
        consultReplies={[]}
        message={{
          id: "message",
          seq: 1,
          session_id: "session",
          chain_id: "chain",
          parent_msg_id: null,
          sender_kind: "user",
          sender_id: "user",
          recipients: ["codex"],
          kind: "text",
          content: "Check the workspace",
          turn_id: null,
          metadata: {},
          created_at: 0,
        }}
      />
    );
    expect(
      screen.getByRole("article", { name: "Your message" }).classList.contains("justify-end")
    ).toBe(true);

    render(
      <MessageRow
        consultReplies={[]}
        message={{
          id: "agent_msg",
          seq: 2,
          session_id: "session",
          chain_id: "chain",
          parent_msg_id: null,
          sender_kind: "orchestrator",
          sender_id: "codex",
          recipients: [],
          kind: "text",
          content: "Final polished answer",
          turn_id: "t_99",
          metadata: {
            streaming: false,
            intermediate_notes: ["Checking repo structure..."],
          },
          created_at: 0,
        }}
      />
    );
    expect(screen.getByText("Final polished answer")).toBeTruthy();
    expect(screen.getByText(/Worked · 1 step/)).toBeTruthy();
  });

  it("groups stored Codex terminal chunks and strips internal worktree path prefixes", () => {
    expect(
      cleanWorktreePath(
        "C:\\Users\\panzh\\test\\.meta-harness\\worktrees\\s_1\\_integration\\index.html"
      )
    ).toBe("index.html");

    const entries = groupTrajectoryEvents([
      {
        id: 1,
        turn_id: "turn",
        created_at: 0,
        event_kind: "tool_call",
        payload: { toolCallId: "exec", kind: "execute", title: "git status" },
      },
      {
        id: 2,
        turn_id: "turn",
        created_at: 1,
        event_kind: "tool_call",
        payload: {
          toolCallId: "exec",
          _meta: { terminal_output_delta: { terminal_id: "exec", data: "On branch " } },
        },
      },
      {
        id: 3,
        turn_id: "turn",
        created_at: 2,
        event_kind: "terminal",
        payload: { tool_call_id: "exec", terminal_id: "exec", output_delta: "main" },
      },
      {
        id: 4,
        turn_id: "turn",
        created_at: 3,
        event_kind: "tool_call",
        payload: {
          toolCallId: "exec",
          status: "completed",
          _meta: { terminal_exit: { exit_code: 0 } },
        },
      },
    ]);
    expect(entries).toHaveLength(1);
    render(<TrajectoryRow entry={entries[0]} />);
    expect(screen.getByText("git status")).toBeTruthy();
    expect(screen.getByText("Exit 0")).toBeTruthy();
    expect(screen.getByText("On branch main")).toBeTruthy();
  });

  it("rounds quota credit decimals and hides plan dropdown / live checkbox in 2-col Settings", () => {
    expect(formatCreditsDisplay("Credits: 60002.0775370000")).toBe("Credits: 60002");

    if (!HTMLDialogElement.prototype.showModal) {
      HTMLDialogElement.prototype.showModal = function () {
        this.open = true;
      };
    }

    useAppStore.setState({
      probes: [
        {
          adapter_id: "codex",
          display_name: "Codex",
          accent_color: "#10a37f",
          installed: true,
          resolved_command: "/usr/bin/codex",
          auth_detected: true,
          auth_hint: "Ready",
          strengths: [],
        },
        {
          adapter_id: "claude",
          display_name: "Claude Code",
          accent_color: "#d97757",
          installed: false,
          resolved_command: null,
          auth_detected: false,
          auth_hint: "missing",
          strengths: [],
        },
      ],
      quotas: {
        codex: {
          adapter_id: "codex",
          plan_label: "ProLite",
          window_5h_remaining_pct: null,
          window_5h_resets_at: null,
          weekly_remaining_pct: 92,
          weekly_resets_at: null,
          local_5h_tokens_est: 0,
          rate_limited_until: null,
          exact_probe_opt_in: true,
          source: "live_probe",
          credits_label: "Credits: 60002.0775370000",
          updated_at: 1,
        },
      },
      adapterCatalogs: {
        codex: {
          adapter_id: "codex",
          models: [{ value: "gpt-5.4", label: "GPT-5.4" }],
          efforts: [{ value: "high", label: "High" }],
          default_context_window: 200000,
        },
      },
    });

    render(<SettingsDialog onClose={() => {}} />);
    expect(screen.queryByText(/Live provider quota/)).toBeNull();
    expect(screen.queryByLabelText("Codex plan")).toBeNull();
    expect(screen.getByText("Credits: 60002")).toBeTruthy();
    expect(screen.getByLabelText("Codex default model")).toBeTruthy();
    expect(screen.getByLabelText("Codex default effort")).toBeTruthy();
    expect(screen.getByText("Not installed (1)")).toBeTruthy();
    expect(screen.queryByText("Claude Code")).toBeNull();
    fireEvent.click(screen.getByText("Not installed (1)"));
    expect(screen.getByText("Claude Code")).toBeTruthy();
  });

  it("serializes atomic @mention pills and deletes an entire pill cleanly", () => {
    const onChange = vi.fn();
    render(
      <MentionEditor
        value="@codex check build"
        validHandles={new Set(["codex"])}
        onChange={onChange}
        onMentionQuery={() => {}}
      />
    );
    const textbox = screen.getByRole("textbox", { name: "Message group chat" });
    expect(serializeMentionDom(textbox)).toBe("@codex check build");
    const removeBtn = screen.getByRole("button", { name: "Remove @codex" });
    fireEvent.click(removeBtn);
    expect(onChange).toHaveBeenLastCalledWith(" check build");
  });

  it("supports session context menu rename and renders only one settings button when sidebar is open", async () => {
    vi.spyOn(api, "patchSession").mockResolvedValue({
      session: {
        id: "s_1",
        project_id: "p_1",
        title: "Renamed Chat",
        base_branch: null,
        base_commit_sha: null,
        integration_branch: null,
        integration_wt_path: null,
        isolation_mode: "shared_cwd",
        orchestrator_member: "codex",
        confirm_before_dispatch: false,
        max_hops: 6,
        permission_policy: "worktree_smart",
        working_summary: "",
        created_at: 0,
        updated_at: 0,
      },
    });

    useAppStore.setState({
      sidebarCollapsed: false,
      projects: [
        {
          id: "p_1",
          name: "demo",
          root_path: "/tmp/demo",
          is_git_repo: true,
          created_at: 0,
          last_opened_at: 0,
        },
      ],
      activeProjectId: "p_1",
      sessions: [
        {
          id: "s_1",
          project_id: "p_1",
          title: "New session",
          base_branch: null,
          base_commit_sha: null,
          integration_branch: null,
          integration_wt_path: null,
          isolation_mode: "shared_cwd",
          orchestrator_member: "codex",
          confirm_before_dispatch: false,
          max_hops: 6,
          permission_policy: "worktree_smart",
          working_summary: "",
          created_at: 0,
          updated_at: 0,
        },
      ],
      activeSession: null,
      probes: [
        {
          adapter_id: "codex",
          display_name: "Codex",
          accent_color: "#10a37f",
          installed: true,
          resolved_command: "/usr/bin/codex",
          auth_detected: true,
          auth_hint: "Ready",
          strengths: [],
        },
      ],
    });

    render(
      <div className="flex">
        <Sidebar />
        <ChatPane />
      </div>
    );

    // Only one Settings button exists in Sidebar footer while sidebar is visible
    expect(screen.getAllByRole("button", { name: /Settings/i })).toHaveLength(1);
    expect(screen.getByText("What would you like to build?")).toBeTruthy();

    fireEvent.contextMenu(screen.getByText("New session"));
    fireEvent.click(screen.getByRole("menuitem", { name: "Rename" }));
    const titleInput = screen.getByLabelText("Session title");
    fireEvent.change(titleInput, { target: { value: "Renamed Chat" } });
    fireEvent.keyDown(titleInput, { key: "Enter" });
    await waitFor(() => {
      expect(api.patchSession).toHaveBeenCalledWith("s_1", { title: "Renamed Chat" });
      expect(screen.getByText("Renamed Chat")).toBeTruthy();
    });
  });

  it("serializes atomic colored /slash pills alongside @mention pills and deletes /slash cleanly", () => {
    const onChange = vi.fn();
    render(
      <MentionEditor
        value="/plan @codex /diff"
        validHandles={new Set(["codex"])}
        onChange={onChange}
        onMentionQuery={() => {}}
        onSlashQuery={() => {}}
      />
    );
    const textbox = screen.getByRole("textbox", { name: "Message group chat" });
    expect(serializeMentionDom(textbox)).toBe("/plan @codex /diff");
    const planPill = textbox.querySelector('[data-slash="plan"]') as HTMLElement | null;
    const diffPill = textbox.querySelector('[data-slash="diff"]') as HTMLElement | null;
    expect(planPill?.dataset.slashKind).toBe("workflow");
    expect(diffPill?.dataset.slashKind).toBe("action");

    fireEvent.click(screen.getByRole("button", { name: "Remove /plan" }));
    expect(onChange).toHaveBeenLastCalledWith(" @codex /diff");
  });

  it("uses custom Model & Effort popovers with model_details and resets unsupported effort", () => {
    vi.spyOn(api, "getSessionDiff").mockResolvedValue({ diff: null });
    useAppStore.setState({
      activeProjectId: null,
      activeSession: {
        id: "s_standalone",
        project_id: null,
        title: "Standalone Task",
        base_branch: null,
        base_commit_sha: null,
        integration_branch: "mh/s_standalone/integration",
        integration_wt_path: "/home/user/.meta-harness/workspaces/s_standalone/.meta-harness/worktrees/s_standalone/_integration",
        isolation_mode: "worktree_per_member",
        orchestrator_member: "codex",
        confirm_before_dispatch: false,
        max_hops: 6,
        permission_policy: "worktree_smart",
        working_summary: "",
        archived_at: null,
        created_at: 0,
        updated_at: 0,
      },
      probes: [
        {
          adapter_id: "codex",
          display_name: "Codex",
          accent_color: "#10a37f",
          installed: true,
          resolved_command: "/usr/bin/codex",
          auth_detected: true,
          auth_hint: "Ready",
          strengths: [],
        },
      ],
      members: [
        {
          session_id: "s_standalone",
          handle: "codex",
          adapter_id: "codex",
          display_name: "Codex",
          role: "both",
          persona_prompt: "",
          worktree_branch: null,
          worktree_path: null,
          acp_session_id: null,
          active_model: null,
          active_effort: null,
          last_seen_msg_seq: 0,
        },
      ],
      adapterCatalogs: {
        codex: {
          adapter_id: "codex",
          models: [
            { value: "gpt-6-astra", label: "gpt-6-astra" },
            { value: "gpt-5.6-luna", label: "gpt-5.6-luna" },
          ],
          model_details: [
            {
              value: "gpt-6-astra",
              label: "gpt-6-astra",
              description: "Flagship reasoning",
              is_default: true,
              default_effort: "medium",
              supported_efforts: [
                { value: "low", label: "Low" },
                { value: "xhigh", label: "Extra High" },
                { value: "ultra", label: "Ultra" },
              ],
            },
            {
              value: "gpt-5.6-luna",
              label: "gpt-5.6-luna",
              description: "Fast lightweight model",
              is_default: false,
              default_effort: null,
              supported_efforts: [{ value: "low", label: "Low" }],
            },
          ],
          efforts: [{ value: "medium", label: "Medium" }],
          default_context_window: 200000,
        },
      },
    });

    const { container } = render(<Composer />);
    expect(container.querySelectorAll("select")).toHaveLength(0);

    fireEvent.click(screen.getByRole("button", { name: "Target model" }));
    fireEvent.click(screen.getByText("gpt-6-astra"));
    expect(useAppStore.getState().autoModelOverride).toBe("gpt-6-astra");

    fireEvent.click(screen.getByRole("button", { name: "Reasoning effort" }));
    expect(screen.getByText("Ultra")).toBeTruthy();
    fireEvent.click(screen.getByText("Ultra"));
    expect(useAppStore.getState().autoEffortOverride).toBe("ultra");

    fireEvent.click(screen.getByRole("button", { name: "Target model" }));
    fireEvent.click(screen.getByText("gpt-5.6-luna"));
    expect(useAppStore.getState().autoModelOverride).toBe("gpt-5.6-luna");
    expect(useAppStore.getState().autoEffortOverride).toBeNull();

    // Test action slash command /diff opens Diff modal without sending a message
    const editor = screen.getByRole("textbox", { name: "Message group chat" });
    editor.textContent = "/diff";
    fireEvent.input(editor);
    fireEvent.click(screen.getByRole("button", { name: /Send/i }));
    expect(useAppStore.getState().diffModalOpen).toBe(true);
  });

  it("removes clutter texts, unifies Open/New buttons, supports Archived section, and hides Promote in standalone mode", async () => {
    vi.spyOn(api, "patchSession").mockResolvedValue({
      session: {
        id: "s_active",
        project_id: null,
        title: "Quick Task",
        base_branch: null,
        base_commit_sha: null,
        integration_branch: "mh/s_active/integration",
        integration_wt_path: null,
        isolation_mode: "worktree_per_member",
        orchestrator_member: "codex",
        confirm_before_dispatch: false,
        max_hops: 6,
        permission_policy: "worktree_smart",
        working_summary: "",
        archived_at: 1700000,
        created_at: 0,
        updated_at: 0,
      },
    });
    vi.spyOn(api, "listStandaloneSessions").mockResolvedValue({
      sessions: [
        {
          id: "s_active",
          project_id: null,
          title: "Quick Task",
          base_branch: null,
          base_commit_sha: null,
          integration_branch: null,
          integration_wt_path: null,
          isolation_mode: "worktree_per_member",
          orchestrator_member: "codex",
          confirm_before_dispatch: false,
          max_hops: 6,
          permission_policy: "worktree_smart",
          working_summary: "",
          archived_at: 1700000,
          created_at: 0,
          updated_at: 0,
        },
      ],
    });

    useAppStore.setState({
      sidebarCollapsed: false,
      activeProjectId: null,
      projects: [],
      sessions: [
        {
          id: "s_active",
          project_id: null,
          title: "Quick Task",
          base_branch: null,
          base_commit_sha: null,
          integration_branch: "mh/s_active/integration",
          integration_wt_path: null,
          isolation_mode: "worktree_per_member",
          orchestrator_member: "codex",
          confirm_before_dispatch: false,
          max_hops: 6,
          permission_policy: "worktree_smart",
          working_summary: "",
          archived_at: null,
          created_at: 0,
          updated_at: 0,
        },
        {
          id: "s_old",
          project_id: null,
          title: "Old Archived Task",
          base_branch: null,
          base_commit_sha: null,
          integration_branch: null,
          integration_wt_path: null,
          isolation_mode: "shared_cwd",
          orchestrator_member: null,
          confirm_before_dispatch: false,
          max_hops: 6,
          permission_policy: "worktree_smart",
          working_summary: "",
          archived_at: 123456,
          created_at: 0,
          updated_at: 0,
        },
      ],
      activeSession: null,
      probes: [
        {
          adapter_id: "codex",
          display_name: "Codex",
          accent_color: "#10a37f",
          installed: true,
          resolved_command: "/usr/bin/codex",
          auth_detected: true,
          auth_hint: "Ready",
          strengths: [],
        },
      ],
    });

    render(
      <div className="flex">
        <Sidebar />
        <ChatPane />
      </div>
    );

    expect(screen.queryByText(/Obsidian/)).toBeNull();
    expect(screen.queryByText(/isolated worktrees · Auto-named/i)).toBeNull();
    const openBtn = screen.getByRole("button", { name: /^Open$/i });
    const newBtn = screen.getByRole("button", { name: /^New$/i });
    expect(openBtn.className).toBe(newBtn.className);

    fireEvent.click(screen.getByText("Archived (1)"));
    expect(screen.getByText("Old Archived Task")).toBeTruthy();

    fireEvent.contextMenu(screen.getByText("Quick Task"));
    fireEvent.click(screen.getByRole("menuitem", { name: "Archive" }));
    await waitFor(() => {
      expect(api.patchSession).toHaveBeenCalledWith("s_active", { archived: true });
    });

    cleanup();
    useAppStore.setState({
      activeSession: {
        id: "s_standalone",
        project_id: null,
        title: "Standalone",
        base_branch: "main",
        base_commit_sha: null,
        integration_branch: "mh/s_standalone/integration",
        integration_wt_path: null,
        isolation_mode: "worktree_per_member",
        orchestrator_member: "codex",
        confirm_before_dispatch: false,
        max_hops: 6,
        permission_policy: "worktree_smart",
        working_summary: "",
        created_at: 0,
        updated_at: 0,
      },
      sessionDiff: {
        integration_branch: "mh/s_standalone/integration",
        base_branch: "main",
        ahead_commits: 1,
        diff_stat: { files_changed: 1, insertions: 4, deletions: 1 },
        unified_diff: "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-old\n+new\n",
        truncated: false,
      },
    });
    render(<DiffReviewModal onClose={() => {}} />);
    expect(screen.queryByRole("button", { name: /Promote to/i })).toBeNull();
  });

  it("renders Storage & Data tab in Settings and runs cleanup actions", async () => {
    if (!HTMLDialogElement.prototype.showModal) {
      HTMLDialogElement.prototype.showModal = function () {
        this.open = true;
      };
    }

    vi.spyOn(api, "getStorage").mockResolvedValue({
      storage: {
        total_bytes: 5 * 1024 * 1024,
        database_bytes: 512 * 1024,
        active_worktrees_bytes: 2 * 1024 * 1024,
        active_worktrees_count: 1,
        standalone_workspaces_bytes: 1024 * 1024,
        standalone_count: 1,
        archived_sessions_count: 2,
        archived_bytes: 1024 * 1024,
        cache_and_orphans_bytes: 512 * 1024,
        orphan_dirs_count: 1,
        data_dir: "/home/user/.local/share/meta-harness",
        workspaces_dir: "/home/user/.meta-harness/workspaces",
      },
    });
    vi.spyOn(api, "cleanupStorage").mockResolvedValue({
      result: {
        action: "delete_archived",
        removed_sessions: 2,
        removed_dirs: 2,
        freed_bytes: 1024 * 1024,
      },
      storage: {
        total_bytes: 4 * 1024 * 1024,
        database_bytes: 512 * 1024,
        active_worktrees_bytes: 2 * 1024 * 1024,
        active_worktrees_count: 1,
        standalone_workspaces_bytes: 1024 * 1024,
        standalone_count: 1,
        archived_sessions_count: 0,
        archived_bytes: 0,
        cache_and_orphans_bytes: 512 * 1024,
        orphan_dirs_count: 1,
        data_dir: "/home/user/.local/share/meta-harness",
        workspaces_dir: "/home/user/.meta-harness/workspaces",
      },
    });
    vi.spyOn(api, "listStandaloneSessions").mockResolvedValue({ sessions: [] });

    render(<SettingsDialog onClose={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Storage & Data" }));

    await waitFor(() => {
      expect(screen.getByText("5.00 MB")).toBeTruthy();
      expect(screen.getByText("Archived Sessions (2)")).toBeTruthy();
    });

    fireEvent.click(screen.getByRole("button", { name: /Delete Archived Sessions/i }));
    await waitFor(() => {
      expect(api.cleanupStorage).toHaveBeenCalledWith("delete_archived");
      expect(screen.getByText(/Deleted 2 archived sessions/)).toBeTruthy();
    });
  });
});
