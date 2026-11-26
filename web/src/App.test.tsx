import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App } from "./App";
import { MarkdownBlock } from "./components/common/MarkdownBlock";
import { formatCreditsDisplay, QuotaMeter, QuotaSnapshotSummary } from "./components/common/Badges";
import { DiffViewer } from "./components/common/DiffViewer";
import { ChatPane } from "./components/chat/ChatPane";
import { Composer } from "./components/chat/Composer";
import { MentionEditor, serializeMentionDom } from "./components/chat/MentionEditor";
import { MessageRow } from "./components/chat/MessageRow";
import { DiffReviewModal } from "./components/modals/DiffReviewModal";
import { FilePreviewModal } from "./components/modals/FilePreviewModal";
import { MemoryModal } from "./components/modals/MemoryModal";
import { SettingsDialog } from "./components/modals/SettingsDialog";
import { PersonaModelFields } from "./components/modals/PersonaModelFields";
import { Sidebar } from "./components/sidebar/Sidebar";
import { cleanWorktreePath, groupTrajectoryEvents } from "./components/drawer/trajectory";
import { TrajectoryRow } from "./components/drawer/TrajectoryRow";
import { terminalDirectory } from "./components/chat/terminalPaths";
import { api, WsHubClient } from "./api/client";
import { useAppStore } from "./store/useAppStore";
import type { ChatMessage, TrajectoryEvent, TurnTrajectory } from "./types/api";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  useAppStore.setState(useAppStore.getInitialState(), true);
});

describe("Startup and file previews", () => {
  it("keeps the composer available while model and quota lookups finish", async () => {
    let connect!: () => void;
    let loadModels!: () => void;
    let loadQuota!: () => void;
    const connection = new Promise<void>((resolve) => { connect = resolve; });
    const models = new Promise<void>((resolve) => { loadModels = resolve; });
    const quota = new Promise<void>((resolve) => { loadQuota = resolve; });
    vi.spyOn(api, "authStatus").mockImplementation(async () => {
      await connection;
      return { required: false, authenticated: true };
    });
    vi.spyOn(api, "listAdapters").mockImplementation(async (_refresh, summary) => {
      if (!summary) await models;
      return { adapters: {} };
    });
    vi.spyOn(api, "listQuotas").mockImplementation(async () => {
      await quota;
      return { quotas: [] };
    });
    vi.spyOn(api, "listProjects").mockResolvedValue({ projects: [] });
    vi.spyOn(api, "listStandaloneSessions").mockResolvedValue({ sessions: [] });
    vi.spyOn(api, "probeAdapters").mockResolvedValue({
      probes: [{
        adapter_id: "codex", display_name: "Codex", accent_color: "#10a37f",
        installed: true, resolved_command: "/usr/bin/codex",
        auth_detected: true, auth_hint: "Ready", strengths: [],
      }],
    });
    vi.spyOn(WsHubClient.prototype, "connect").mockImplementation(() => undefined);
    render(<App />);
    expect(screen.getByRole("status").textContent).toContain("Connecting to the server");

    connect();
    await screen.findByRole("textbox", { name: "Message group chat" });
    expect(screen.getByRole("status").textContent).toMatch(/Loading model choices.*Checking subscription usage/);

    loadModels();
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe("Checking subscription usage"));
    loadQuota();
    await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
  });

  it("renders HTML at the session file URL in an isolated frame", async () => {
    if (!HTMLDialogElement.prototype.showModal) {
      HTMLDialogElement.prototype.showModal = function () { this.open = true; };
    }
    const fetchFile = vi.fn().mockResolvedValue(new Response(null, {
      headers: { "content-type": "text/html; charset=utf-8", "x-meta-harness-preview": "fixture-grant" },
    }));
    vi.stubGlobal("fetch", fetchFile);
    render(<FilePreviewModal sessionId="fixture" path="site/index.html" onClose={vi.fn()} />);

    const frame = await screen.findByTitle("index.html webpage");
    const url = "/api/sessions/fixture/files/site/index.html?preview=true";
    expect(fetchFile).toHaveBeenCalledWith(url, expect.objectContaining({ method: "HEAD" }));
    expect(frame.getAttribute("src")).toBe("/preview/fixture/fixture-grant/site/index.html");
    expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
  });
});

describe("Session navigation", () => {
  it("discards skill responses for projects that are no longer selected", async () => {
    type SkillListing = Awaited<ReturnType<typeof api.listSkills>>;
    let finishFirst: (result: SkillListing) => void = () => {};
    const first = new Promise<SkillListing>((resolve) => { finishFirst = resolve; });
    const firstSkill = {
      id: "first", name: "First", description: "First project skill", source: "project" as const,
      origin: null, enabled: true, builtin_modified: false, skill_dir: "/first/skills/first",
      skill_md_path: "/first/skills/first/SKILL.md", files: ["SKILL.md"], updated_at: 0,
    };
    vi.spyOn(api, "listSkills").mockImplementation((id) => id === "first"
      ? first
      : Promise.resolve({ skills: [{ ...firstSkill, id: "second" }], skills_dir: "/data/skills" }));
    vi.spyOn(api, "listSessions").mockResolvedValue({ sessions: [] });
    vi.spyOn(api, "getMemory").mockResolvedValue({ jobs: [], facts: [] });

    const store = useAppStore.getState();
    await store.selectProject("first");
    await store.selectProject("second");
    expect(useAppStore.getState().skills.map((skill) => skill.id)).toEqual(["second"]);
    finishFirst({ skills: [firstSkill], skills_dir: "/data/skills" });
    await first;
    expect(useAppStore.getState().skills.map((skill) => skill.id)).toEqual(["second"]);
    expect(useAppStore.getState().skillsLoading).toBe(false);
  });
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
    const connected = vi.fn();
    const client = new WsHubClient(vi.fn(), connected);
    client.connect(null);
    client.setSession("session");
    client.subscribeTrajectory("turn");
    socket.readyState = 1;
    socket.onopen();

    expect(send.mock.calls.map(([frame]) => JSON.parse(frame))).toEqual([
      { type: "set_session", session_id: "session" },
      { type: "subscribe_trajectory", turn_id: "turn" },
    ]);
    expect(connected).toHaveBeenCalledOnce();
    client.disconnect();
    expect(close).toHaveBeenCalledOnce();
  });

  it("backfills a running turn on return and merges its streamed steps", async () => {
    const turn: TurnTrajectory = {
      turn_id: "running", session_id: "first", member_handle: "codex", adapter_id: "codex",
      model: null, effort: null, worktree_branch: null, head_commit_sha: null, status: "running",
      summary: "", touched_files: [], diff_stat: { files_changed: 0, insertions: 0, deletions: 0 },
      started_at: 1, finished_at: null,
    };
    const firstStep: TrajectoryEvent = { id: 1, turn_id: turn.turn_id, event_kind: "thought", payload: { text: "Before switching" }, created_at: 1 };
    const liveStep: TrajectoryEvent = { ...firstStep, id: 2, payload: { text: "While loading history" } };
    vi.spyOn(api, "getSession").mockImplementation(async (id) => ({
      session: {
        id, project_id: null, title: id, isolation_mode: "shared_cwd", base_branch: null,
        base_commit_sha: null, integration_branch: null, integration_wt_path: null,
        orchestrator_member: null, confirm_before_dispatch: false, max_hops: 6,
        permission_policy: "worktree_smart", working_summary: "", created_at: 0, updated_at: 0,
      },
      members: [], statuses: {}, permission_rules: [], pending_permissions: [],
    }));
    vi.spyOn(api, "listMessages").mockResolvedValue({ messages: [] });
    vi.spyOn(api, "listTrajectories").mockImplementation(async (id) => ({ trajectories: id === "first" ? [turn] : [] }));
    vi.spyOn(api, "getSessionDiff").mockResolvedValue({ diff: null });
    let deliverSnapshot!: () => void;
    vi.spyOn(api, "getTrajectory").mockImplementation(() => new Promise((resolve) => {
      deliverSnapshot = () => resolve({ trajectory: turn, events: [firstStep] });
    }));

    const store = useAppStore.getState();
    await store.selectSession("other");
    await store.selectSession("first");
    store.handleWsEvent({ type: "trajectory.event", session_id: "first", turn_id: "running", event: liveStep });
    deliverSnapshot();
    await waitFor(() => expect(useAppStore.getState().turnEvents.running).toEqual([firstStep, liveStep]));
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

    const update = {
      type: "chat.message_delta" as const,
      session_id: "s_1",
      message_id: "m_1",
      turn_id: "t_1",
      member_handle: "codex",
      delta: "Final summary ready.",
      replace: true,
      revision: 3,
    };
    const store = useAppStore.getState();
    store.handleWsEvent(update);
    store.handleWsEvent({ ...update, delta: "Old snapshot", revision: 2 });
    store.handleWsEvent({ ...update, delta: "Duplicate fragment", replace: false });

    expect(useAppStore.getState().messages[0].content).toBe("Final summary ready.");
  });
});

describe("UI primitives", () => {
  it("shows a shared usage note with private-profile quota windows in compact views", () => {
    render(<QuotaSnapshotSummary compact quota={{
      adapter_id: "antigravity", plan_label: "Google Account",
      window_5h_remaining_pct: 83, window_5h_resets_at: null,
      weekly_remaining_pct: null, weekly_resets_at: null,
      local_5h_tokens_est: 0, rate_limited_until: null,
      exact_probe_opt_in: true, source: "live_probe",
      shared_profile_usage: true, updated_at: 0,
    }} />);
    expect(screen.getByText("Shared CLI usage may differ from this private profile.")).toBeTruthy();
    expect(screen.getByText("83%")).toBeTruthy();
  });
  it("clears an incompatible persona effort when that persona changes model", () => {
    const save = vi.fn();
    const low = { value: "low", label: "Low" };
    const high = { value: "high", label: "High" };
    render(<PersonaModelFields label="@reviewer" model="first" effort="high" disabled={false} onChange={save}
      catalog={{ adapter_id: "codex", default_context_window: 200000, efforts: [low, high],
        models: [{ value: "first", label: "First" }, { value: "second", label: "Second" }],
        model_details: [
          { value: "first", label: "First", description: "", is_default: false, default_effort: null, supported_efforts: [high] },
          { value: "second", label: "Second", description: "", is_default: false, default_effort: null, supported_efforts: [low] },
        ],
      }} />);
    fireEvent.change(screen.getByRole("combobox", { name: "@reviewer model" }), { target: { value: "second" } });
    expect(save).toHaveBeenCalledWith({ active_model: "second", active_effort: null });
  });
  it("renders Markdown in trajectory narrative blocks", () => {
    render(<TrajectoryRow entry={{ type: "event", key: "1", event: {
      id: 1, turn_id: "turn", event_kind: "thought", created_at: 0,
      payload: { text: "\n\n**Testing key-event text entry**\n\nNext, run the check." },
    } }} />);
    expect(screen.getByText("Testing key-event text entry").tagName).toBe("STRONG");
    expect(screen.getByText("Next, run the check.")).toBeTruthy();
  });

  it.each([
    "Run mcp_meta-harness-switch_chat_read_messages",
    "mcp.meta-harness-switch.chat_read_messages",
  ])("summarizes internal switch activity from %s", (title) => {
    const [entry] = groupTrajectoryEvents([{
      id: 1, turn_id: "turn", event_kind: "tool_call", created_at: 0,
      payload: { title, status: "completed", rawInput: { limit: 40 } },
    }]);
    render(<TrajectoryRow entry={entry} actor="Orchestrator" />);
    expect(screen.getByText("Orchestrator read group chat messages")).toBeTruthy();
    expect(screen.queryByText(title)).toBeNull();
    expect(screen.queryByText("Input")).toBeNull();
  });

  it("decodes per-terminal directory reports without interpreting other OSC notifications", () => {
    expect(terminalDirectory("9;C:\\Users\\Demo\\My Files", 9)).toBe("C:\\Users\\Demo\\My Files");
    expect(terminalDirectory("file:///C:/Users/Demo/My%20Files", 7)).toBe("C:\\Users\\Demo\\My Files");
    expect(terminalDirectory("file://localhost/tmp/My%20Files", 7)).toBe("/tmp/My Files");
    expect(terminalDirectory("This is a notification", 9)).toBeNull();
  });
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

    const complete: ChatMessage = {
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
    };
    const { rerender } = render(
      <MessageRow consultReplies={[]} message={{ ...complete, content: "", metadata: { streaming: true } }} />
    );
    expect(screen.getByRole("status").textContent?.trim()).toBe("Working…");
    expect(screen.queryByText(/Thinking/)).toBeNull();
    expect(screen.queryByRole("button", { name: /Steps/ })).toBeNull();
    rerender(<MessageRow consultReplies={[]} message={complete} />);
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

  it("opens project memory from a Quick Session without a harness or network request", () => {
    const getMemory = vi.spyOn(api, "getMemory");
    function QuickSessionComposer() {
      const open = useAppStore((state) => state.memoryModalOpen);
      return <><Composer />{open && <MemoryModal onClose={() => {}} />}</>;
    }
    render(<QuickSessionComposer />);
    const textbox = screen.getByRole("textbox", { name: "Message group chat" });
    textbox.textContent = "/memory";
    fireEvent.input(textbox);
    fireEvent.keyDown(textbox, { key: "Enter" });
    expect(screen.getByRole("dialog", { name: "Project Memory" })).toBeTruthy();
    expect(screen.getByText("Open a project to view its saved jobs and pinned notes.")).toBeTruthy();
    expect(serializeMentionDom(textbox)).toBe("");
    expect(getMemory).not.toHaveBeenCalled();
  });

  it("displays the harness-reported context with its own usage and limit", () => {
    useAppStore.setState({ sessionContext: {
      session_id: "s_quick",
      members: [{
        handle: "codex", adapter_id: "codex", model: "gpt-6.2-sol",
        used_tokens: 17_000, context_window: 475_000, is_exact: true,
        system_and_persona_tokens: 280, memory_and_telemetry_tokens: 80,
        chat_history_tokens: 223, agent_and_tool_tokens: 10,
      }],
    } });
    render(<Composer />);
    fireEvent.click(screen.getByRole("button", { name: "Context usage" }));
    expect(screen.getByText("17k / 475k")).toBeTruthy();
    expect(screen.getByText("Harness context")).toBeTruthy();
    expect(screen.getByText("Window reported by the harness")).toBeTruthy();
    expect(screen.queryByText("System & Persona")).toBeNull();
  });

  it("follows mentions in the model menu and keeps effort overrides scoped to the recipient", async () => {
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
                { value: "medium", label: "Medium" },
                { value: "xhigh", label: "Extra High" },
                { value: "ultra", label: "Ultra" },
              ],
            },
            {
              value: "gpt-5.6-luna",
              label: "gpt-5.6-luna",
              description: "Fast lightweight model",
              is_default: false,
              default_effort: "low",
              supported_efforts: [{ value: "low", label: "Low" }],
            },
          ],
          efforts: [{ value: "medium", label: "Medium" }],
          default_context_window: 200000,
        },
      },
    });

    const sendMessage = vi.fn().mockResolvedValue(undefined);
    const { probes, members, adapterCatalogs } = useAppStore.getState();
    const claude = { ...members[0], handle: "reviewer", adapter_id: "claude", display_name: "Reviewer" };
    useAppStore.setState({
      sendMessage,
      probes: [...probes, { ...probes[0], adapter_id: "claude", display_name: "Claude" }],
      members: [...members, claude],
      adapterCatalogs: {
        ...adapterCatalogs,
        claude: {
          adapter_id: "claude",
          models: [{ value: "sonnet", label: "Sonnet" }],
          efforts: [{ value: "low", label: "Low" }, { value: "medium", label: "Medium" }],
          default_context_window: 200000,
        },
      },
    });
    render(<Composer />);

    const trigger = screen.getByRole("button", { name: "Model and reasoning effort" });
    const permission = screen.getByRole("button", { name: "Permission policy" });
    fireEvent.click(permission);
    expect(screen.getByRole("button", { name: /^Worktree Allow/ }).querySelector(".lucide-git-branch")).toBeTruthy();
    expect(screen.getByRole("button", { name: /^Strict Prompt/ }).querySelector(".lucide-lock-keyhole")).toBeTruthy();
    expect(screen.getByRole("button", { name: /^Auto Allow/ }).querySelector(".lucide-zap")).toBeTruthy();
    fireEvent.click(permission);
    const editor = screen.getByRole("textbox", { name: "Message group chat" });
    const draft = (text: string) => {
      editor.textContent = text;
      fireEvent.input(editor);
    };
    const openMenu = (handle: string) => {
      fireEvent.click(trigger);
      return within(screen.getByRole("group", { name: `Model and effort for @${handle}` }));
    };
    const send = screen.getByRole("button", { name: /Send/i });

    draft("Ask @codex to greet @reviewer");
    expect((trigger as HTMLButtonElement).disabled).toBe(true);
    expect(trigger.textContent).toContain("Auto route");

    draft("@codex say hello to @reviewer");
    const codexMenu = openMenu("codex");
    fireEvent.click(codexMenu.getByRole("button", { name: "gpt-6-astra" }));
    const slider = codexMenu.getByRole("slider", { name: "Reasoning effort" });
    expect(slider.getAttribute("aria-valuetext")).toBe("Default · Medium");
    expect((slider as HTMLInputElement).value).toBe("1");
    fireEvent.change(slider, { target: { value: "3" } });
    expect(slider.getAttribute("aria-valuetext")).toBe("Ultra");
    fireEvent.click(codexMenu.getByRole("button", { name: "Done" }));

    draft("Please ask (@Reviewer) and @codex for help");
    const multiMenu = openMenu("codex");
    expect(multiMenu.getByRole("tab", { name: "@codex" }).getAttribute("aria-selected")).toBe("true");
    fireEvent.click(multiMenu.getByRole("tab", { name: "@reviewer" }));
    const claudeMenu = within(screen.getByRole("tabpanel", { name: "@reviewer" }));
    expect(claudeMenu.queryByRole("button", { name: "gpt-6-astra" })).toBeNull();
    fireEvent.click(claudeMenu.getByRole("button", { name: "Sonnet" }));
    fireEvent.change(claudeMenu.getByRole("slider"), { target: { value: "1" } });
    fireEvent.keyDown(multiMenu.getByRole("tab", { name: "@reviewer" }), { key: "ArrowLeft" });
    expect(multiMenu.getByRole("tab", { name: "@codex" }).getAttribute("aria-selected")).toBe("true");
    expect(multiMenu.getByRole("slider").getAttribute("aria-valuetext")).toBe("Ultra");
    fireEvent.click(multiMenu.getByRole("button", { name: "Done" }));

    fireEvent.click(send);
    expect(sendMessage).toHaveBeenLastCalledWith("Please ask (@Reviewer) and @codex for help", {
      recipient_overrides: {
        codex: { model: "gpt-6-astra", effort: "ultra" },
        reviewer: { model: "sonnet", effort: "medium" },
      },
    });

    await waitFor(() => expect((send as HTMLButtonElement).disabled).toBe(true));
    draft("@orchestrator ask @reviewer to check this");
    const resetMenu = openMenu("codex");
    expect(resetMenu.getByRole("slider").getAttribute("aria-valuetext")).toBe("Ultra");
    fireEvent.click(resetMenu.getByRole("button", { name: "gpt-5.6-luna" }));
    expect(resetMenu.getByRole("slider").getAttribute("aria-valuetext")).toBe("Default · Low");
    fireEvent.click(resetMenu.getByRole("button", { name: "Done" }));
    fireEvent.click(send);
    expect(sendMessage).toHaveBeenLastCalledWith("@orchestrator ask @reviewer to check this", {
      recipient_overrides: { codex: { model: "gpt-5.6-luna", effort: null } },
    });

    await waitFor(() => expect((send as HTMLButtonElement).disabled).toBe(true));
    draft("/diff");
    fireEvent.click(send);
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
    expect(api.cleanupStorage).not.toHaveBeenCalledWith("delete_archived");
    fireEvent.click(screen.getByRole("button", { name: "Delete sessions" }));
    await waitFor(() => {
      expect(api.cleanupStorage).toHaveBeenCalledWith("delete_archived");
      expect(screen.getByText(/Deleted 2 archived sessions/)).toBeTruthy();
    });
  });

  it("exposes enabled skills in Composer / menu and manages skills in Settings", async () => {
    if (!HTMLDialogElement.prototype.showModal) {
      HTMLDialogElement.prototype.showModal = function () {
        this.open = true;
      };
    }

    const builtinSkill = {
      id: "meta-harness-docs",
      name: "Meta-Harness Docs",
      description: "Comprehensive Meta-Harness usage and development guide.",
      source: "builtin" as const,
      origin: "embedded",
      enabled: true,
      builtin_modified: false,
      skill_dir: "/data/skills/meta-harness-docs",
      skill_md_path: "/data/skills/meta-harness-docs/SKILL.md",
      files: ["SKILL.md", "references/usage-and-workflows.md"],
      updated_at: 100,
    };

    useAppStore.setState({
      skills: [builtinSkill],
      skillsDir: "/data/skills",
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

    type SkillFile = Awaited<ReturnType<typeof api.getSkillDetail>>;
    let finishSkillFile: (file: SkillFile) => void = () => {};
    const slowFile = new Promise<SkillFile>((resolve) => { finishSkillFile = resolve; });
    vi.spyOn(api, "getSkillDetail").mockReturnValueOnce(slowFile).mockResolvedValue({
      detail: {
        skill: builtinSkill,
        file_path: "references/usage-and-workflows.md",
        content: "# Usage and workflows",
      },
    });
    vi.spyOn(api, "patchSkill").mockResolvedValue({
      skill: { ...builtinSkill, enabled: false },
    });
    vi.spyOn(api, "listSkills").mockResolvedValue({
      skills: [{ ...builtinSkill, enabled: false }], skills_dir: "/data/skills",
    });

    render(<SettingsDialog onClose={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Skills" }));

    expect(screen.getByText("Meta-Harness Docs")).toBeTruthy();
    expect(screen.getByText("/meta-harness-docs")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /Inspect \/ Edit Files/i }));
    fireEvent.click(screen.getByRole("button", { name: "references/usage-and-workflows.md" }));
    await screen.findByDisplayValue("# Usage and workflows");
    await act(async () => {
      finishSkillFile({ detail: { skill: builtinSkill, file_path: "SKILL.md", content: "# Meta-Harness Docs" } });
      await slowFile;
    });
    expect(screen.getByDisplayValue("# Usage and workflows")).toBeTruthy();

    fireEvent.click(screen.getByRole("switch", { name: "Enable skill Meta-Harness Docs" }));
    await waitFor(() => {
      expect(api.patchSkill).toHaveBeenCalledWith("meta-harness-docs", {
        enabled: false,
        project_id: null,
      });
      expect(screen.getByRole("switch", { name: "Enable skill Meta-Harness Docs" }).getAttribute("aria-checked")).toBe("false");
    });

    cleanup();
    const skillCommands = new Map([["meta-harness-docs", "skill" as const]]);
    const { container } = render(
      <MentionEditor
        value="/meta-harness-docs @codex explain"
        validHandles={new Set(["codex"])}
        slashCommands={skillCommands}
        onChange={() => {}}
        onMentionQuery={() => {}}
      />
    );
    const pill = container.querySelector('[data-slash="meta-harness-docs"]');
    expect(pill?.getAttribute("data-slash-kind")).toBe("skill");
    const textbox = screen.getByRole("textbox", { name: "Message group chat" });
    expect(serializeMentionDom(textbox)).toBe("/meta-harness-docs @codex explain");
  });
});
