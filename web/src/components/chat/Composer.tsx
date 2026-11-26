import { useEffect, useMemo, useRef, useState } from "react";
import {
  Check,
  ChevronDown,
  CornerDownLeft,
  Cpu,
  Crown,
  Gauge,
  Send,
  Shield,
  Sparkles,
  Square,
} from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type {
  MemberContextBreakdown,
  PermissionPolicy,
} from "../../types/api";
import { HarnessAvatar } from "../common/Badges";
import {
  DEFAULT_SLASH_COMMANDS,
  MentionEditor,
  type MentionEditorHandle,
  type SlashCommandKind,
} from "./MentionEditor";

const SLASH_ITEMS: Array<{
  command: string;
  kind: SlashCommandKind;
  desc: string;
}> = [
  { command: "plan", kind: "workflow", desc: "Plan approach before editing" },
  { command: "review", kind: "workflow", desc: "Review changes & edge cases" },
  { command: "test", kind: "workflow", desc: "Add & run tests" },
  { command: "fix", kind: "workflow", desc: "Fix build or test errors" },
  { command: "compact", kind: "action", desc: "Compact chat history into summary" },
  { command: "diff", kind: "action", desc: "Open cumulative diff viewer" },
  { command: "shell", kind: "action", desc: "Toggle WebShell PTY terminal" },
  { command: "memory", kind: "action", desc: "Open project memory" },
  { command: "new", kind: "action", desc: "Start a new session" },
  { command: "archive", kind: "action", desc: "Archive current session" },
];

const PERMISSION_LABELS: Record<PermissionPolicy, string> = {
  worktree_smart: "Worktree-Smart",
  strict: "Strict",
  autonomous_sandbox: "Autonomous",
};

const EFFORT_LABELS: Record<string, string> = {
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra High",
  max: "Max",
  ultra: "Ultra",
};

function formatCompactTokens(tokens: number): string {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1000) return `${Math.round(tokens / 1000)}k`;
  return `${tokens}`;
}

export function Composer({
  draftPreset,
  onConsumePreset,
}: {
  draftPreset?: { seq: number; text: string } | null;
  onConsumePreset?: () => void;
}) {
  const {
    activeProjectId,
    activeSession,
    members,
    adapters,
    adapterCatalogs,
    adapterDefaults,
    memberOptions,
    probes,
    quotas,
    memberStatuses,
    sessionContext,
    memoryJobs,
    memoryFacts,
    selectedRecipient,
    setSelectedRecipient,
    pendingPermissionPolicy,
    setPendingPermissionPolicy,
    autoModelOverride,
    setAutoModelOverride,
    autoEffortOverride,
    setAutoEffortOverride,
    updateMemberConfig,
    createAndSelectSession,
    archiveSession,
    compactActiveSession,
    sendMessage,
    stopAll,
    selectSession,
    refreshSessionDiff,
    setDiffModalOpen,
    shellOpen,
    setShellOpen,
    setMemoryModalOpen,
    setError,
  } = useAppStore();

  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [mentionQuery, setMentionQuery] = useState<string | null>(null);
  const [slashQuery, setSlashQuery] = useState<string | null>(null);
  const [selectedIdx, setSelectedIdx] = useState(0);
  const [menuDismissed, setMenuDismissed] = useState(false);
  const [recipientOpen, setRecipientOpen] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);
  const [effortOpen, setEffortOpen] = useState(false);
  const [permOpen, setPermOpen] = useState(false);
  const [contextOpen, setContextOpen] = useState(false);
  const [contextMemberHandle, setContextMemberHandle] = useState<string | null>(null);
  const editorRef = useRef<MentionEditorHandle | null>(null);

  useEffect(() => {
    if (draftPreset?.text) {
      setText(draftPreset.text);
      onConsumePreset?.();
      editorRef.current?.focus();
    }
  }, [draftPreset?.seq, draftPreset?.text, onConsumePreset]);

  const installedProbes = useMemo(
    () => probes.filter((p) => p.installed),
    [probes]
  );

  // Use installed session members when in a session, otherwise synthesize virtual installed targets for the Welcome composer.
  const availableMembers = useMemo(() => {
    if (activeSession) {
      return members.filter((member) =>
        installedProbes.some((probe) => probe.adapter_id === member.adapter_id)
      );
    }
    return installedProbes.map((p) => ({
      session_id: "pending",
      handle: p.adapter_id,
      adapter_id: p.adapter_id,
      display_name: p.display_name,
      role: "both" as const,
      persona_prompt: "",
      worktree_branch: null,
      worktree_path: null,
      acp_session_id: null,
      active_model: adapterDefaults[p.adapter_id]?.default_model ?? null,
      active_effort: adapterDefaults[p.adapter_id]?.default_effort ?? null,
      last_seen_msg_seq: 0,
    }));
  }, [activeSession, members, installedProbes, adapterDefaults]);

  const orchestrator = useMemo(() => {
    if (activeSession) {
      return (
        availableMembers.find(
          (member) => member.handle === activeSession.orchestrator_member
        ) ?? null
      );
    }
    return (
      availableMembers.find((m) => m.adapter_id === "antigravity") ??
      availableMembers[0] ??
      null
    );
  }, [activeSession, availableMembers]);

  const validHandleSet = useMemo(() => {
    const s = new Set<string>(availableMembers.map((m) => m.handle));
    if (orchestrator) s.add("orchestrator");
    if (availableMembers.length > 0) s.add("all");
    return s;
  }, [availableMembers, orchestrator]);

  const anyActive = Object.values(memberStatuses).some(
    (s) =>
      s.status === "thinking" ||
      s.status === "running_tool" ||
      s.status === "spawning" ||
      s.status === "consulting" ||
      s.status === "waiting_permission"
  );

  useEffect(() => {
    setMenuDismissed(false);
    setSelectedIdx(0);
  }, [mentionQuery, slashQuery]);

  const memberQuotaLabel = (adapterId: string): string | null => {
    const q = quotas[adapterId];
    if (!q) return null;
    if (q.rate_limited_until !== null && q.rate_limited_until !== undefined) {
      return "429";
    }
    const minPct =
      q.window_5h_remaining_pct !== null && q.weekly_remaining_pct !== null
        ? Math.min(q.window_5h_remaining_pct, q.weekly_remaining_pct)
        : (q.window_5h_remaining_pct ?? q.weekly_remaining_pct ?? null);
    return minPct !== null ? `${Math.round(minPct)}%` : null;
  };

  const candidates = useMemo(() => {
    if (mentionQuery === null || menuDismissed) return [];
    const base = [
      ...availableMembers.map((m) => ({
        handle: m.handle,
        desc: m.display_name,
        accent: adapters[m.adapter_id]?.accent_color ?? "#6366f1",
        pctLabel: memberQuotaLabel(m.adapter_id),
      })),
      ...(orchestrator
        ? [
            {
              handle: "orchestrator",
              desc: "Session Orchestrator",
              accent: "#f59e0b",
              pctLabel: null,
            },
          ]
        : []),
      ...(availableMembers.length > 1
        ? [
            {
              handle: "all",
              desc: "All available executors",
              accent: "#6366f1",
              pctLabel: null,
            },
          ]
        : []),
    ];
    return base.filter((c) => c.handle.toLowerCase().startsWith(mentionQuery));
  }, [mentionQuery, menuDismissed, availableMembers, orchestrator, adapters, quotas]);

  const slashCandidates = useMemo(() => {
    if (slashQuery === null || menuDismissed) return [];
    return SLASH_ITEMS.filter((item) => item.command.startsWith(slashQuery));
  }, [slashQuery, menuDismissed]);

  const explicitMentions = useMemo(
    () =>
      Array.from(
        text
          .replace(/```[\s\S]*?```|`[^`]+`/g, " ")
          .matchAll(/(?:^|\s)@([a-zA-Z0-9_-]+)/g)
      ).map((m) => m[1]),
    [text]
  );

  const routingLabel = useMemo(() => {
    if (explicitMentions.length > 0) {
      return `Direct · ${[...new Set(explicitMentions)].map((h) => `@${h}`).join(", ")}`;
    }
    if (selectedRecipient) {
      return selectedRecipient === "all"
        ? "Broadcast · @all"
        : `Direct · @${selectedRecipient}`;
    }
    if (orchestrator) return `Auto · @${orchestrator.handle}`;
    return availableMembers.length
      ? `To @${availableMembers[0].handle}`
      : "No harness";
  }, [explicitMentions, selectedRecipient, orchestrator, availableMembers]);

  // Determine target member when controlling per-target vs Auto model & effort
  const targetedMember = useMemo(() => {
    const firstExplicit = explicitMentions.find(
      (h) => h.toLowerCase() !== "all" && h.toLowerCase() !== "orchestrator"
    );
    if (firstExplicit) {
      return (
        availableMembers.find(
          (m) => m.handle.toLowerCase() === firstExplicit.toLowerCase()
        ) ?? null
      );
    }
    if (selectedRecipient && selectedRecipient !== "all") {
      return (
        availableMembers.find((m) => m.handle === selectedRecipient) ?? null
      );
    }
    return null;
  }, [explicitMentions, selectedRecipient, availableMembers]);

  const targetAdapterId =
    targetedMember?.adapter_id ??
    orchestrator?.adapter_id ??
    availableMembers[0]?.adapter_id;
  const targetCatalog = targetAdapterId ? adapterCatalogs[targetAdapterId] : undefined;

  const modelDetailsForTarget = useMemo(() => {
    if (!targetAdapterId) return [];
    const discovered = targetedMember
      ? (memberOptions[targetedMember.handle] ?? []).find(
          (o) => o.category === "model" || o.id === "model"
        )?.choices ?? []
      : [];
    const baseDetails =
      targetCatalog?.model_details && targetCatalog.model_details.length > 0
        ? targetCatalog.model_details
        : (targetCatalog?.models ?? []).map((m) => ({
            value: m.value,
            label: m.label,
            description: "",
            is_default: false,
            default_effort: null,
            supported_efforts: targetCatalog?.efforts ?? [],
          }));
    const seen = new Set<string>();
    const merged = [
      ...discovered.map((c) => ({
        value: c.value,
        label: c.label,
        description: "",
        is_default: false,
        default_effort: null,
        supported_efforts:
          baseDetails.find((d) => d.value === c.value)?.supported_efforts ??
          targetCatalog?.efforts ??
          [],
      })),
      ...baseDetails,
    ].filter((item) => {
      if (seen.has(item.value)) return false;
      seen.add(item.value);
      return true;
    });
    const extra =
      targetedMember?.active_model ?? adapterDefaults[targetAdapterId]?.default_model;
    if (extra && !seen.has(extra)) {
      merged.unshift({
        value: extra,
        label: extra,
        description: "",
        is_default: false,
        default_effort: null,
        supported_efforts: targetCatalog?.efforts ?? [],
      });
    }
    return merged;
  }, [targetAdapterId, targetCatalog, targetedMember, memberOptions, adapterDefaults]);

  const selectedModelValue = targetedMember
    ? targetedMember.active_model ?? ""
    : autoModelOverride ?? "";
  const selectedEffortValue = targetedMember
    ? targetedMember.active_effort ?? ""
    : autoEffortOverride ?? "";

  const selectedModelDetail = useMemo(
    () => modelDetailsForTarget.find((m) => m.value === selectedModelValue) ?? null,
    [modelDetailsForTarget, selectedModelValue]
  );

  const effortChoicesForTarget = useMemo(() => {
    if (!targetAdapterId) return [];
    if (selectedModelDetail && selectedModelDetail.supported_efforts.length > 0) {
      return selectedModelDetail.supported_efforts;
    }
    const discovered = targetedMember
      ? (memberOptions[targetedMember.handle] ?? []).find(
          (o) =>
            o.category === "thought_level" ||
            o.category === "reasoning_effort" ||
            o.id === "reasoning_effort"
        )?.choices ?? []
      : [];
    return discovered.length > 0 ? discovered : targetCatalog?.efforts ?? [];
  }, [targetAdapterId, selectedModelDetail, targetedMember, memberOptions, targetCatalog]);

  const closeAllMenus = () => {
    setRecipientOpen(false);
    setModelOpen(false);
    setEffortOpen(false);
    setPermOpen(false);
    setContextOpen(false);
  };

  const handleModelSelect = (nextModel: string | null) => {
    setModelOpen(false);
    const nextDetail = modelDetailsForTarget.find((m) => m.value === nextModel);
    const allowed =
      nextDetail && nextDetail.supported_efforts.length > 0
        ? nextDetail.supported_efforts
        : targetCatalog?.efforts ?? [];
    const curEffort = selectedEffortValue || null;
    const effortStillValid =
      curEffort === null || allowed.some((e) => e.value === curEffort);
    const nextEffort = effortStillValid ? undefined : null;

    if (targetedMember && activeSession) {
      void updateMemberConfig(targetedMember.handle, {
        active_model: nextModel,
        ...(nextEffort !== undefined ? { active_effort: nextEffort } : {}),
      });
    } else {
      setAutoModelOverride(nextModel);
      if (nextEffort !== undefined) setAutoEffortOverride(null);
    }
  };

  const handleEffortSelect = (nextEffort: string | null) => {
    setEffortOpen(false);
    if (targetedMember && activeSession) {
      void updateMemberConfig(targetedMember.handle, {
        active_effort: nextEffort,
      });
    } else {
      setAutoEffortOverride(nextEffort);
    }
  };

  const currentPolicy: PermissionPolicy =
    activeSession?.permission_policy ?? pendingPermissionPolicy;

  // Compute displayed Context Breakdown + live Draft tokens
  const draftTokens = useMemo(
    () => (text.trim().length > 0 ? Math.max(1, Math.ceil(text.trim().length / 4)) : 0),
    [text]
  );

  const contextMembersList: MemberContextBreakdown[] = useMemo(() => {
    if (sessionContext?.members && sessionContext.members.length > 0) {
      return sessionContext.members;
    }
    return availableMembers.map((m) => ({
      handle: m.handle,
      adapter_id: m.adapter_id,
      model: m.active_model ?? adapterDefaults[m.adapter_id]?.default_model ?? null,
      used_tokens: 280 + memoryJobs.length * 45 + memoryFacts.length * 25 + 160,
      context_window: adapterCatalogs[m.adapter_id]?.default_context_window ?? 200_000,
      is_exact: false,
      system_and_persona_tokens: 280,
      memory_and_telemetry_tokens: memoryJobs.length * 45 + memoryFacts.length * 25 + 160,
      chat_history_tokens: 0,
      agent_and_tool_tokens: 0,
    }));
  }, [
    sessionContext,
    availableMembers,
    adapterDefaults,
    adapterCatalogs,
    memoryJobs.length,
    memoryFacts.length,
  ]);

  const activeContextBreakdown = useMemo(() => {
    const preferredHandle =
      contextMemberHandle ??
      targetedMember?.handle ??
      orchestrator?.handle ??
      contextMembersList[0]?.handle;
    return (
      contextMembersList.find((m) => m.handle === preferredHandle) ??
      contextMembersList[0] ??
      null
    );
  }, [contextMemberHandle, targetedMember, orchestrator, contextMembersList]);

  const contextTotalWithDraft = activeContextBreakdown
    ? activeContextBreakdown.used_tokens + draftTokens
    : draftTokens;
  const contextWindow = Math.max(activeContextBreakdown?.context_window ?? 200_000, 1);
  const contextPct = Math.min(100, Math.round((contextTotalWithDraft / contextWindow) * 100));

  const usingOrchestrator =
    Boolean(orchestrator) &&
    selectedRecipient === null &&
    explicitMentions.length === 0;

  const handleSend = async () => {
    const raw = text.trim();
    if (!raw || sending) return;

    const lowerRaw = raw.toLowerCase();
    if (lowerRaw === "/diff") {
      if (activeSession) {
        setText("");
        void refreshSessionDiff();
        setDiffModalOpen(true);
      }
      return;
    }
    if (lowerRaw === "/shell") {
      if (activeSession) {
        setText("");
        setShellOpen(!shellOpen);
      }
      return;
    }
    if (lowerRaw === "/memory") {
      if (activeProjectId) {
        setText("");
        setMemoryModalOpen(true);
      }
      return;
    }
    if (lowerRaw === "/new") {
      setText("");
      setSending(true);
      try {
        await createAndSelectSession("New session");
      } catch (error) {
        setError(error instanceof Error ? error.message : String(error));
      } finally {
        setSending(false);
      }
      return;
    }
    if (lowerRaw === "/archive") {
      if (activeSession) {
        setText("");
        setSending(true);
        try {
          await archiveSession(activeSession.id, true);
        } catch (error) {
          setError(error instanceof Error ? error.message : String(error));
        } finally {
          setSending(false);
        }
      }
      return;
    }

    const compactMatch = raw.match(/^\/compact(?:\s+([\s\S]+))?$/i);
    if (compactMatch) {
      if (!activeSession) return;
      const remainder = compactMatch[1]?.trim() ?? "";
      setText("");
      setSending(true);
      try {
        await compactActiveSession();
        if (remainder) {
          const payload =
            selectedRecipient && explicitMentions.length === 0
              ? `@${selectedRecipient} ${remainder}`
              : remainder;
          const opts = targetedMember
            ? {
                model: targetedMember.active_model ?? undefined,
                effort: targetedMember.active_effort ?? undefined,
              }
            : {
                model: autoModelOverride ?? undefined,
                effort: autoEffortOverride ?? undefined,
              };
          await sendMessage(payload, opts);
        }
      } catch (error) {
        setText((current) => (current ? `${raw}\n${current}` : raw));
        setError(error instanceof Error ? error.message : String(error));
      } finally {
        setSending(false);
      }
      return;
    }

    if (!availableMembers.length) return;
    const payload =
      selectedRecipient && explicitMentions.length === 0
        ? `@${selectedRecipient} ${raw}`
        : raw;
    const opts = targetedMember
      ? {
          model: targetedMember.active_model ?? undefined,
          effort: targetedMember.active_effort ?? undefined,
        }
      : {
          model: autoModelOverride ?? undefined,
          effort: autoEffortOverride ?? undefined,
        };
    setText("");
    setSending(true);
    try {
      await sendMessage(payload, opts);
    } catch (error) {
      setText((current) => (current ? `${raw}\n${current}` : raw));
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setSending(false);
    }
  };

  const toggleConfirmFirst = async () => {
    if (!activeSession) return;
    try {
      await api.patchSession(activeSession.id, {
        confirm_before_dispatch: !activeSession.confirm_before_dispatch,
      });
      if (useAppStore.getState().activeSession?.id === activeSession.id) {
        await selectSession(activeSession.id);
      }
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    }
  };

  const handlePolicyChange = async (policy: PermissionPolicy) => {
    setPermOpen(false);
    if (!activeSession) {
      setPendingPermissionPolicy(policy);
      return;
    }
    try {
      await api.patchSession(activeSession.id, { permission_policy: policy });
      if (useAppStore.getState().activeSession?.id === activeSession.id) {
        await selectSession(activeSession.id);
      }
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    }
  };

  const activeAutocompleteCount =
    slashCandidates.length > 0 ? slashCandidates.length : candidates.length;

  return (
    <div className="relative px-4 pb-4 pt-2 sm:px-6">
      <div className="relative mx-auto max-w-[52rem]">
        {/* Autocomplete Dropdown (@mentions or /slash commands) */}
        {candidates.length > 0 && (
          <div className="absolute bottom-full left-0 mb-2 w-80 max-w-full rounded-xl border border-line-strong bg-panel shadow-2xl overflow-hidden z-30 p-1">
            <div className="px-2.5 py-1 text-[10px] uppercase tracking-wider font-semibold text-ink-muted">
              Route target (Tab / Enter)
            </div>
            {candidates.map((c, idx) => (
              <button
                key={c.handle}
                type="button"
                onClick={() => editorRef.current?.insertMention(c.handle)}
                className={`w-full px-2.5 py-1.5 rounded-lg text-left text-xs flex items-center justify-between gap-2 transition-colors ${
                  idx === selectedIdx
                    ? "bg-brand-soft text-ink"
                    : "hover:bg-hover text-ink-soft"
                }`}
              >
                <span className="flex items-center gap-2 min-w-0">
                  <HarnessAvatar handle={c.handle} accentColor={c.accent} size="sm" />
                  <span className="font-mono font-semibold text-brand-ink">
                    @{c.handle}
                  </span>
                </span>
                <span className="flex items-center gap-1.5 text-[11px] text-ink-muted shrink-0">
                  <span className="truncate max-w-[100px]">{c.desc}</span>
                  {c.pctLabel && (
                    <span className="px-1.5 py-0.2 rounded bg-inset border border-line font-mono text-[10px]">
                      {c.pctLabel}
                    </span>
                  )}
                </span>
              </button>
            ))}
          </div>
        )}

        {slashCandidates.length > 0 && (
          <div className="absolute bottom-full left-0 mb-2 w-80 max-w-full rounded-xl border border-line-strong bg-panel shadow-2xl overflow-hidden z-30 p-1">
            <div className="px-2.5 py-1 text-[10px] uppercase tracking-wider font-semibold text-ink-muted">
              Slash command (Tab / Enter)
            </div>
            {slashCandidates.map((item, idx) => (
              <button
                key={item.command}
                type="button"
                onClick={() => editorRef.current?.insertSlash(item.command)}
                className={`w-full px-2.5 py-1.5 rounded-lg text-left text-xs flex items-center justify-between gap-2 transition-colors ${
                  idx === selectedIdx
                    ? "bg-brand-soft text-ink"
                    : "hover:bg-hover text-ink-soft"
                }`}
              >
                <span
                  className={`font-mono font-semibold px-1.5 py-0.2 rounded ${
                    item.kind === "workflow"
                      ? "bg-pos-soft text-pos"
                      : "bg-warn-soft text-warn"
                  }`}
                >
                  /{item.command}
                </span>
                <span className="text-[11px] text-ink-muted truncate">{item.desc}</span>
              </button>
            ))}
          </div>
        )}

        <div className="space-y-2 rounded-2xl border border-line-strong bg-panel p-3 shadow-sm transition-colors focus-within:border-brand">
          <MentionEditor
            ref={editorRef}
            value={text}
            validHandles={validHandleSet}
            slashCommands={DEFAULT_SLASH_COMMANDS}
            disabled={!availableMembers.length}
            placeholder={
              availableMembers.length
                ? "Message the group… Type @ for harnesses or / for commands"
                : "No CLI harnesses detected yet — open Settings to check requirements"
            }
            onChange={setText}
            onMentionQuery={setMentionQuery}
            onSlashQuery={setSlashQuery}
            onKeyDown={(e) => {
              if (activeAutocompleteCount > 0) {
                if (e.key === "ArrowDown") {
                  e.preventDefault();
                  setSelectedIdx((i) => (i + 1) % activeAutocompleteCount);
                  return;
                }
                if (e.key === "ArrowUp") {
                  e.preventDefault();
                  setSelectedIdx(
                    (i) => (i - 1 + activeAutocompleteCount) % activeAutocompleteCount
                  );
                  return;
                }
                if (e.key === "Tab" || e.key === "Enter") {
                  e.preventDefault();
                  if (slashCandidates.length > 0) {
                    const chosen = slashCandidates[selectedIdx] ?? slashCandidates[0];
                    if (chosen) editorRef.current?.insertSlash(chosen.command);
                  } else {
                    const chosen = candidates[selectedIdx] ?? candidates[0];
                    if (chosen) editorRef.current?.insertMention(chosen.handle);
                  }
                  return;
                }
                if (e.key === "Escape") {
                  e.preventDefault();
                  setMenuDismissed(true);
                  return;
                }
              }

              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                e.preventDefault();
                void handleSend();
                return;
              }
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void handleSend();
              }
            }}
          />

          <div className="flex items-center justify-between gap-2 pt-2 border-t border-line whitespace-nowrap">
            <div className="flex items-center gap-1.5 text-[11px] relative min-w-0">
              {/* Recipient Pill */}
              <div className="relative">
                <button
                  type="button"
                  onClick={() => {
                    const next = !recipientOpen;
                    closeAllMenus();
                    setRecipientOpen(next);
                  }}
                  disabled={explicitMentions.length > 0}
                  className="inline-flex items-center gap-1 px-2 py-1 rounded-lg bg-elevated hover:bg-hover disabled:opacity-70 border border-line text-ink-soft hover:text-ink transition-colors"
                >
                  {usingOrchestrator && <Crown className="w-3 h-3 text-warn shrink-0" />}
                  <span className="font-mono truncate max-w-[125px]">{routingLabel}</span>
                  {explicitMentions.length === 0 && (
                    <ChevronDown className="w-3 h-3 text-ink-muted shrink-0" />
                  )}
                </button>

                {recipientOpen && explicitMentions.length === 0 && (
                  <>
                    <div
                      className="fixed inset-0 z-20"
                      onClick={() => setRecipientOpen(false)}
                    />
                    <div className="absolute bottom-full left-0 mb-1.5 w-64 rounded-xl border border-line-strong bg-panel shadow-2xl p-1 z-30 space-y-0.5">
                      <div className="px-2.5 py-1 text-[10px] uppercase tracking-wider font-semibold text-ink-muted">
                        Send messages to
                      </div>
                      {orchestrator && (
                        <button
                          type="button"
                          onClick={() => {
                            setSelectedRecipient(null);
                            setRecipientOpen(false);
                          }}
                          className={`w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg text-left text-xs ${
                            selectedRecipient === null
                              ? "bg-brand-soft text-ink"
                              : "hover:bg-hover text-ink-soft"
                          }`}
                        >
                          <span className="flex items-center gap-2">
                            <Crown className="w-3.5 h-3.5 text-warn" />
                            <span>Auto (@{orchestrator.handle})</span>
                          </span>
                          {selectedRecipient === null && (
                            <Check className="w-3 h-3 text-brand-ink" />
                          )}
                        </button>
                      )}

                      {availableMembers.map((m) => {
                        const pct = memberQuotaLabel(m.adapter_id);
                        const accent = adapters[m.adapter_id]?.accent_color ?? "#6366f1";
                        const active = selectedRecipient === m.handle;
                        return (
                          <button
                            key={m.handle}
                            type="button"
                            onClick={() => {
                              setSelectedRecipient(m.handle);
                              setRecipientOpen(false);
                            }}
                            className={`w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg text-left text-xs ${
                              active
                                ? "bg-brand-soft text-ink"
                                : "hover:bg-hover text-ink-soft"
                            }`}
                          >
                            <span className="flex items-center gap-2 min-w-0">
                              <HarnessAvatar
                                handle={m.handle}
                                accentColor={accent}
                                status={memberStatuses[m.handle]?.status}
                                size="sm"
                              />
                              <span className="font-mono text-ink truncate">
                                @{m.handle}
                              </span>
                            </span>
                            <span className="flex items-center gap-1.5 shrink-0">
                              {pct && (
                                <span className="text-[10px] font-mono px-1.5 py-0.2 rounded bg-inset border border-line text-ink-muted">
                                  {pct}
                                </span>
                              )}
                              {active && <Check className="w-3 h-3 text-brand-ink" />}
                            </span>
                          </button>
                        );
                      })}

                      {availableMembers.length > 1 && (
                        <button
                          type="button"
                          onClick={() => {
                            setSelectedRecipient("all");
                            setRecipientOpen(false);
                          }}
                          className={`w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg text-left text-xs ${
                            selectedRecipient === "all"
                              ? "bg-brand-soft text-ink"
                              : "hover:bg-hover text-ink-soft"
                          }`}
                        >
                          <span className="font-mono">@all (All executors)</span>
                          {selectedRecipient === "all" && (
                            <Check className="w-3 h-3 text-brand-ink" />
                          )}
                        </button>
                      )}
                    </div>
                  </>
                )}
              </div>

              {/* Custom Themed Target Model Popover */}
              {availableMembers.length > 0 && (
                <div className="relative">
                  <button
                    type="button"
                    aria-label="Target model"
                    onClick={() => {
                      const next = !modelOpen;
                      closeAllMenus();
                      setModelOpen(next);
                    }}
                    className="inline-flex items-center gap-1 px-2 py-1 rounded-lg bg-elevated hover:bg-hover border border-line text-ink-soft hover:text-ink transition-colors"
                  >
                    <Cpu className="w-3 h-3 text-brand-ink shrink-0" />
                    <span className="font-mono truncate max-w-[110px]">
                      {selectedModelDetail?.label ??
                        (selectedModelValue || (targetedMember ? "Default" : "Auto"))}
                    </span>
                    <ChevronDown className="w-3 h-3 text-ink-muted shrink-0" />
                  </button>

                  {modelOpen && (
                    <>
                      <div
                        className="fixed inset-0 z-20"
                        onClick={() => setModelOpen(false)}
                      />
                      <div className="absolute bottom-full left-0 mb-1.5 w-72 max-h-72 overflow-y-auto rounded-xl border border-line-strong bg-panel shadow-2xl p-1 z-30 space-y-0.5 text-xs whitespace-normal">
                        <div className="px-2.5 py-1 text-[10px] uppercase tracking-wider font-semibold text-ink-muted">
                          Model
                        </div>
                        <button
                          type="button"
                          onClick={() => handleModelSelect(null)}
                          className={`w-full px-2.5 py-1.5 rounded-lg text-left flex items-center justify-between gap-2 transition-colors ${
                            !selectedModelValue
                              ? "bg-brand-soft text-ink font-medium"
                              : "hover:bg-hover text-ink-soft"
                          }`}
                        >
                          <span className="font-mono">
                            {targetedMember ? "Model: Default" : "Model: Auto"}
                          </span>
                          {!selectedModelValue && (
                            <Check className="w-3 h-3 text-brand-ink shrink-0" />
                          )}
                        </button>

                        {modelDetailsForTarget.map((m) => {
                          const active = selectedModelValue === m.value;
                          return (
                            <button
                              key={m.value}
                              type="button"
                              onClick={() => handleModelSelect(m.value)}
                              className={`w-full px-2.5 py-1.5 rounded-lg text-left transition-colors ${
                                active
                                  ? "bg-brand-soft text-ink"
                                  : "hover:bg-hover text-ink-soft"
                              }`}
                            >
                              <div className="flex items-center justify-between gap-2">
                                <span className="flex items-center gap-1.5 font-mono font-medium text-ink">
                                  <span>{m.label}</span>
                                  {m.is_default && (
                                    <span className="px-1.5 py-0.2 rounded bg-inset border border-line text-[9.5px] text-ink-muted">
                                      default
                                    </span>
                                  )}
                                </span>
                                {active && (
                                  <Check className="w-3 h-3 text-brand-ink shrink-0" />
                                )}
                              </div>
                              {m.description && (
                                <div className="text-[10.5px] text-ink-muted line-clamp-2 mt-0.5">
                                  {m.description}
                                </div>
                              )}
                            </button>
                          );
                        })}
                      </div>
                    </>
                  )}
                </div>
              )}

              {/* Custom Themed Reasoning Effort Popover */}
              {availableMembers.length > 0 && (
                <div className="relative">
                  <button
                    type="button"
                    aria-label="Reasoning effort"
                    onClick={() => {
                      const next = !effortOpen;
                      closeAllMenus();
                      setEffortOpen(next);
                    }}
                    className="inline-flex items-center gap-1 px-2 py-1 rounded-lg bg-elevated hover:bg-hover border border-line text-ink-soft hover:text-ink transition-colors"
                  >
                    <Sparkles className="w-3 h-3 text-warn shrink-0" />
                    <span>
                      {selectedEffortValue
                        ? EFFORT_LABELS[selectedEffortValue] ?? selectedEffortValue
                        : targetedMember
                          ? "Effort: Default"
                          : "Auto"}
                    </span>
                    <ChevronDown className="w-3 h-3 text-ink-muted shrink-0" />
                  </button>

                  {effortOpen && (
                    <>
                      <div
                        className="fixed inset-0 z-20"
                        onClick={() => setEffortOpen(false)}
                      />
                      <div className="absolute bottom-full left-0 mb-1.5 w-48 max-h-64 overflow-y-auto rounded-xl border border-line-strong bg-panel shadow-2xl p-1 z-30 space-y-0.5 text-xs">
                        <div className="px-2.5 py-1 text-[10px] uppercase tracking-wider font-semibold text-ink-muted">
                          Reasoning Effort
                        </div>
                        <button
                          type="button"
                          onClick={() => handleEffortSelect(null)}
                          className={`w-full px-2.5 py-1.5 rounded-lg text-left flex items-center justify-between gap-2 transition-colors ${
                            !selectedEffortValue
                              ? "bg-brand-soft text-ink font-medium"
                              : "hover:bg-hover text-ink-soft"
                          }`}
                        >
                          <span>{targetedMember ? "Effort: Default" : "Effort: Auto"}</span>
                          {!selectedEffortValue && (
                            <Check className="w-3 h-3 text-brand-ink shrink-0" />
                          )}
                        </button>
                        {effortChoicesForTarget.map((c) => {
                          const active = selectedEffortValue === c.value;
                          return (
                            <button
                              key={c.value}
                              type="button"
                              onClick={() => handleEffortSelect(c.value)}
                              className={`w-full px-2.5 py-1.5 rounded-lg text-left flex items-center justify-between gap-2 transition-colors ${
                                active
                                  ? "bg-brand-soft text-ink font-medium"
                                  : "hover:bg-hover text-ink-soft"
                              }`}
                            >
                              <span>{EFFORT_LABELS[c.value] ?? c.label}</span>
                              {active && (
                                <Check className="w-3 h-3 text-brand-ink shrink-0" />
                              )}
                            </button>
                          );
                        })}
                      </div>
                    </>
                  )}
                </div>
              )}

              {/* Permissions Selector Pill */}
              <div className="relative">
                <button
                  type="button"
                  aria-label="Permission policy"
                  onClick={() => {
                    const next = !permOpen;
                    closeAllMenus();
                    setPermOpen(next);
                  }}
                  className="inline-flex items-center gap-1 px-2 py-1 rounded-lg bg-elevated hover:bg-hover border border-line text-ink-soft hover:text-ink transition-colors"
                >
                  <Shield className="w-3 h-3 text-pos shrink-0" />
                  <span className="hidden sm:inline">{PERMISSION_LABELS[currentPolicy]}</span>
                  <ChevronDown className="w-3 h-3 text-ink-muted shrink-0" />
                </button>

                {permOpen && (
                  <>
                    <div
                      className="fixed inset-0 z-20"
                      onClick={() => setPermOpen(false)}
                    />
                    <div className="absolute bottom-full left-0 mb-1.5 w-60 rounded-xl border border-line-strong bg-panel shadow-2xl p-1 z-30 space-y-0.5 text-xs whitespace-normal">
                      <div className="px-2.5 py-1 text-[10px] uppercase tracking-wider font-semibold text-ink-muted">
                        Approval Mode
                      </div>
                      {(
                        [
                          [
                            "worktree_smart",
                            "Worktree-Smart",
                            "Auto-allow worktree edits; prompt shell",
                          ],
                          ["strict", "Strict", "Prompt all file edits & shell"],
                          [
                            "autonomous_sandbox",
                            "Autonomous",
                            "Auto-allow inside isolated worktree",
                          ],
                        ] as const
                      ).map(([val, label, desc]) => (
                        <button
                          key={val}
                          type="button"
                          onClick={() => void handlePolicyChange(val)}
                          className={`w-full px-2.5 py-1.5 rounded-lg text-left transition-colors ${
                            currentPolicy === val
                              ? "bg-brand-soft text-ink"
                              : "hover:bg-hover text-ink-soft"
                          }`}
                        >
                          <div className="flex items-center justify-between font-medium">
                            <span>{label}</span>
                            {currentPolicy === val && (
                              <Check className="w-3 h-3 text-brand-ink" />
                            )}
                          </div>
                          <div className="text-[10.5px] text-ink-muted">{desc}</div>
                        </button>
                      ))}
                    </div>
                  </>
                )}
              </div>

              {usingOrchestrator && activeSession && (
                <button
                  type="button"
                  aria-pressed={activeSession.confirm_before_dispatch}
                  onClick={() => void toggleConfirmFirst()}
                  className={`hidden md:inline-flex items-center gap-1 px-2 py-1 rounded-lg border transition-colors ${
                    activeSession.confirm_before_dispatch
                      ? "bg-warn-soft border-warn/40 text-warn font-medium"
                      : "bg-elevated border-line text-ink-muted hover:text-ink"
                  }`}
                >
                  {activeSession.confirm_before_dispatch && (
                    <Check className="w-3 h-3" />
                  )}
                  <span>Plan first</span>
                </button>
              )}
            </div>

            <div className="flex items-center gap-2 relative shrink-0">
              {/* Context Usage Ring & Composition Popover (anchored inward right-0) */}
              {activeContextBreakdown && (
                <div className="relative">
                  <button
                    type="button"
                    aria-label="Context usage"
                    onClick={() => {
                      const next = !contextOpen;
                      closeAllMenus();
                      setContextOpen(next);
                    }}
                    className="inline-flex items-center gap-1.5 px-2 py-1 rounded-lg bg-elevated hover:bg-hover border border-line text-[11px] font-mono text-ink-muted hover:text-ink transition-colors"
                  >
                    <ContextRing pct={contextPct} />
                    <span>
                      {contextPct}% · {formatCompactTokens(contextTotalWithDraft)}
                    </span>
                  </button>

                  {contextOpen && (
                    <>
                      <div
                        className="fixed inset-0 z-20"
                        onClick={() => setContextOpen(false)}
                      />
                      <div className="absolute bottom-full right-0 mb-2 w-80 rounded-xl border border-line-strong bg-panel shadow-2xl p-3.5 z-30 space-y-3 text-xs whitespace-normal">
                        <div className="flex items-center justify-between gap-2">
                          <div className="flex items-center gap-1.5 font-semibold text-ink">
                            <Gauge className="w-3.5 h-3.5 text-brand-ink" />
                            <span>Context Composition</span>
                          </div>
                          <span className="font-mono text-[10.5px] text-ink-muted">
                            {formatCompactTokens(contextTotalWithDraft)} /{" "}
                            {formatCompactTokens(contextWindow)}
                            {activeContextBreakdown.is_exact ? " · ACP" : " · est"}
                          </span>
                        </div>

                        {contextMembersList.length > 1 && (
                          <div className="flex flex-wrap gap-1">
                            {contextMembersList.map((m) => (
                              <button
                                key={m.handle}
                                type="button"
                                onClick={() => setContextMemberHandle(m.handle)}
                                className={`px-2 py-0.5 rounded-md font-mono text-[10.5px] border ${
                                  activeContextBreakdown.handle === m.handle
                                    ? "bg-brand-soft border-brand/40 text-brand-ink"
                                    : "bg-inset border-line text-ink-muted hover:text-ink"
                                }`}
                              >
                                @{m.handle}
                              </button>
                            ))}
                          </div>
                        )}

                        <ContextBreakdownBar
                          breakdown={activeContextBreakdown}
                          draftTokens={draftTokens}
                          contextWindow={contextWindow}
                        />
                      </div>
                    </>
                  )}
                </div>
              )}

              {anyActive && (
                <button
                  type="button"
                  onClick={() => void stopAll()}
                  className="inline-flex items-center gap-1.5 rounded-lg bg-neg-soft px-3 py-1.5 text-xs font-medium text-neg transition-colors hover:bg-neg/20"
                >
                  <Square className="w-3 h-3 fill-current" />
                  <span>Stop</span>
                </button>
              )}
              <button
                type="button"
                title="Send (Enter or Ctrl+Enter)"
                disabled={
                  !text.trim() ||
                  sending ||
                  (!availableMembers.length && !/^\/(diff|shell|memory|new|archive|compact)\b/i.test(text.trim()))
                }
                onClick={() => void handleSend()}
                className="inline-flex items-center gap-1.5 px-3.5 py-1.5 rounded-xl bg-brand hover:bg-brand-hover disabled:opacity-40 text-xs font-medium text-white transition-colors"
              >
                <Send className="w-3.5 h-3.5" />
                <span>Send</span>
                <kbd className="hidden sm:inline-flex items-center gap-0.5 text-[10px] opacity-80 font-mono">
                  <CornerDownLeft className="w-2.5 h-2.5" />
                </kbd>
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function ContextRing({ pct }: { pct: number }) {
  const r = 5.5;
  const circ = 2 * Math.PI * r;
  const offset = circ - (Math.max(0, Math.min(100, pct)) / 100) * circ;
  const stroke = pct >= 85 ? "#ef4444" : pct >= 60 ? "#f59e0b" : "#6366f1";
  return (
    <svg width="15" height="15" viewBox="0 0 16 16" aria-hidden="true">
      <circle
        cx="8"
        cy="8"
        r={r}
        fill="none"
        stroke="currentColor"
        strokeOpacity="0.22"
        strokeWidth="2"
      />
      <circle
        cx="8"
        cy="8"
        r={r}
        fill="none"
        stroke={stroke}
        strokeWidth="2"
        strokeDasharray={circ}
        strokeDashoffset={offset}
        strokeLinecap="round"
        transform="rotate(-90 8 8)"
      />
    </svg>
  );
}

function ContextBreakdownBar({
  breakdown,
  draftTokens,
  contextWindow,
}: {
  breakdown: MemberContextBreakdown;
  draftTokens: number;
  contextWindow: number;
}) {
  const segments = [
    {
      label: "System & Persona",
      tokens: breakdown.system_and_persona_tokens,
      color: "bg-indigo-500",
    },
    {
      label: "Memory & Telemetry",
      tokens: breakdown.memory_and_telemetry_tokens,
      color: "bg-amber-500",
    },
    {
      label: "Chat History",
      tokens: breakdown.chat_history_tokens,
      color: "bg-emerald-500",
    },
    {
      label: "Tool & Turn Output",
      tokens: breakdown.agent_and_tool_tokens,
      color: "bg-sky-500",
    },
    {
      label: "Current Draft",
      tokens: draftTokens,
      color: "bg-fuchsia-500",
    },
  ];
  const denom = Math.max(
    contextWindow,
    segments.reduce((sum, s) => sum + s.tokens, 0),
    1
  );

  return (
    <div className="space-y-2.5">
      <div className="h-2 w-full rounded-full bg-inset overflow-hidden border border-line flex">
        {segments.map((s) => {
          const widthPct = (s.tokens / denom) * 100;
          if (widthPct <= 0) return null;
          return (
            <div
              key={s.label}
              className={`h-full ${s.color}`}
              style={{ width: `${Math.max(widthPct, 1)}%` }}
              title={`${s.label}: ${s.tokens} tokens`}
            />
          );
        })}
      </div>

      <div className="space-y-1 text-[11px]">
        {segments.map((s) => (
          <div key={s.label} className="flex items-center justify-between gap-2">
            <span className="flex items-center gap-2 text-ink-soft">
              <span className={`w-2 h-2 rounded-xs ${s.color}`} />
              <span>{s.label}</span>
            </span>
            <span className="font-mono text-ink-muted">
              {formatCompactTokens(s.tokens)}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
