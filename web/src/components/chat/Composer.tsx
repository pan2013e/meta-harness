import { useEffect, useId, useMemo, useRef, useState } from "react";
import {
  Check,
  ChevronDown,
  CircleGauge,
  CornerDownLeft,
  Cpu,
  Gauge,
  Send,
  Square,
} from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type {
  MemberContextBreakdown,
  ModelChoiceDetail,
  ModelSelection,
  PermissionPolicy,
  QuotaSnapshot,
  SessionMember,
} from "../../types/api";
import { HarnessAvatar, QuotaSnapshotSummary } from "../common/Badges";
import { APPROVAL_MODES } from "../common/approvalModes";
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

const EFFORT_LABELS: Record<string, string> = {
  none: "None",
  minimal: "Minimal",
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra High",
  max: "Max",
  ultra: "Ultra",
};

const EFFORT_ORDER = Object.keys(EFFORT_LABELS);
const COMPACT_COMMAND = /^\/compact(?:\s+([\s\S]+))?$/i;

const DEFAULT_MODEL_SETTINGS: ModelSelection = { model: null, effort: null };

function recipientKey(member: SessionMember): string {
  return member.adapter_id + "/" + member.handle;
}

function routingMentions(markdown: string, handles: ReadonlySet<string>) {
  // Match the router's mention boundaries and ignore fenced and inline code.
  let fence: string | undefined;
  const withoutFences = markdown.split("\n").map((line) => {
    if (fence) {
      const closing = line.match(/^[ \t]*(`{3,}|~{3,})\s*$/)?.[1];
      if (closing?.startsWith(fence)) fence = undefined;
      return "";
    }
    const opening = line.match(/^[ \t]*(`{3,}|~{3,})(.*)$/);
    if (opening && (opening[1][0] === "~" || !opening[2].includes("`"))) {
      fence = opening[1];
      return "";
    }
    return line;
  }).join("\n");
  const withoutCode = withoutFences.replace(/(?<!`)(`+)(?!`)[\s\S]*?(?<!`)\1(?!`)/g, " ");
  const token = /@([a-z0-9_-]+)(?![a-z0-9_/-]|\.[a-z0-9])(?:\[[^\s\]]+\])?/.source;
  const known = (handle: string) => handles.has(handle) || handle === "all" || handle === "orchestrator";
  let remaining = withoutCode.trimStart();
  const workflow = /^\/(?:plan|review|test|fix|compact)[,.;:!?]*(?:\s+|$)/i;
  while (workflow.test(remaining)) remaining = remaining.replace(workflow, "");

  const leading = new Set<string>();
  const address = new RegExp("^" + token, "i");
  for (let match = address.exec(remaining); match; match = address.exec(remaining)) {
    const handle = match[1].toLowerCase();
    if (known(handle)) leading.add(handle);
    const tail = remaining.slice(match[0].length);
    remaining = tail.replace(/^[\s,]+/, "");
    if (remaining === tail) break;
    remaining = remaining.replace(/^(?:and|&)(?:\s+|$)/i, "");
  }
  if (leading.size) return { mentions: leading, needsIntent: false };

  const mentions = new Set<string>();
  let previousEnd: number | undefined;
  let separated = false;
  for (const match of withoutCode.matchAll(new RegExp("(?:^|[\\s([{\\\"',:;>*])" + token, "gi"))) {
    const handle = match[1].toLowerCase();
    if (!known(handle) || mentions.has(handle)) continue;
    mentions.add(handle);
    if (handle === "all" || handle === "orchestrator") continue;
    const start = match.index + match[0].indexOf("@");
    if (previousEnd !== undefined) {
      const gap = withoutCode.slice(previousEnd, start).replace(/^[\s,()]+|[\s,()]+$/g, "");
      separated ||= !!gap && !/^(?:and|&)$/i.test(gap);
    }
    previousEnd = match.index + match[0].length;
  }
  return { mentions, needsIntent: separated && !mentions.has("all") && !mentions.has("orchestrator") };
}

function effortRank(value: string): number {
  const index = EFFORT_ORDER.indexOf(value);
  return index < 0 ? EFFORT_ORDER.length : index;
}

function formatCompactTokens(tokens: number): string {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1000) return `${Math.round(tokens / 1000)}k`;
  return `${tokens}`;
}

function modelContextWindow(model: string | null | undefined, fallback: number): number {
  return /^gpt-6(?:[.-]|$)/i.test(model ?? "") ? 1_000_000 : fallback;
}

function formatPercent(pct: number): string {
  return pct > 0 && pct < 1 ? "<1%" : Math.round(pct) + "%";
}

function quotaUsed(quota?: QuotaSnapshot): number | null {
  if (!quota) return null;
  if ((quota.rate_limited_until ?? 0) > Date.now()) return 100;
  const remaining = (quota.windows?.length
    ? quota.windows.map((window) => window.remaining_pct)
    : [quota.window_5h_remaining_pct, quota.weekly_remaining_pct]
  ).filter((value): value is number => value != null && Number.isFinite(value));
  return remaining.length ? Math.max(0, Math.min(100, 100 - Math.min(...remaining))) : null;
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
    globalPersonas,
    memberOptions,
    probes,
    quotas,
    memberStatuses,
    sessionContext,
    memoryJobs,
    memoryFacts,
    pendingPermissionPolicy,
    setPendingPermissionPolicy,
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
  const [modelMenu, setModelMenu] = useState<{
    route: string;
    tab: number;
    edits: Record<string, ModelSelection>;
  } | null>(null);
  const [modelOverrides, setModelOverrides] = useState<Record<string, ModelSelection>>({});
  const [permOpen, setPermOpen] = useState(false);
  const [contextOpen, setContextOpen] = useState(false);
  const [quotaOpen, setQuotaOpen] = useState(false);
  const [draftEstimate, setDraftEstimate] = useState<{ text: string; tokens: number } | null>(null);
  const [contextMemberHandle, setContextMemberHandle] = useState<string | null>(null);
  const editorRef = useRef<MentionEditorHandle | null>(null);
  const modelButtonRef = useRef<HTMLButtonElement | null>(null);
  const modelMenuId = useId();

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
  const availableMembers = useMemo<SessionMember[]>(() => {
    if (activeSession) {
      return members.filter((member) =>
        installedProbes.some((probe) => probe.adapter_id === member.adapter_id)
      );
    }
    const defaults: SessionMember[] = installedProbes.map((p) => ({
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
    for (const persona of globalPersonas) {
      if (!installedProbes.some((probe) => probe.adapter_id === persona.adapter_id) ||
          defaults.some((member) => member.handle.toLowerCase() === persona.handle.toLowerCase())) continue;
      defaults.push({
        session_id: "pending",
        role: "executor",
        worktree_branch: null,
        worktree_path: null,
        acp_session_id: null,
        last_seen_msg_seq: 0,
        ...persona,
      });
    }
    return defaults;
  }, [activeSession, members, installedProbes, adapterDefaults, globalPersonas]);

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

  const raw = text.trim();
  useEffect(() => {
    if (!raw) return;
    const controller = new AbortController();
    const timeout = window.setTimeout(() => {
      void api.estimateDraftTokens(raw, controller.signal).then(({ tokens }) => {
        if (!controller.signal.aborted) setDraftEstimate({ text: raw, tokens });
      }).catch(() => undefined);
    }, 160);
    return () => {
      controller.abort();
      window.clearTimeout(timeout);
    };
  }, [raw]);
  const compactMatch = raw.match(COMPACT_COMMAND);
  const routingText = compactMatch ? compactMatch[1]?.trim() ?? "" : raw;
  const roster = activeSession ? members : availableMembers;
  const routingHandles = useMemo(() => new Set(roster.map((member) => member.handle.toLowerCase())), [roster]);
  const { mentions: explicitMentions, needsIntent } = useMemo(
    () => routingMentions(routingText, routingHandles), [routingText, routingHandles]
  );
  const mentionedMembers = roster.filter((member) =>
    explicitMentions.has(member.handle.toLowerCase())
  );
  const toOrchestrator = explicitMentions.has("orchestrator");
  const automaticRecipient = orchestrator ?? availableMembers[0];
  const recipients = needsIntent ? [] : toOrchestrator
    ? orchestrator ? [orchestrator] : []
    : explicitMentions.has("all")
      ? availableMembers.filter((member) => member.handle !== activeSession?.orchestrator_member || member.role !== "orchestrator")
      : mentionedMembers.length
        ? mentionedMembers
        : automaticRecipient ? [automaticRecipient] : [];
  const route = JSON.stringify(recipients.map(recipientKey));
  const openMenu = modelMenu?.route === route ? modelMenu : null;
  const selectedTab = openMenu?.tab ?? 0;
  const recipient = recipients[selectedTab];
  const targetMember = availableMembers.find((member) => member.handle === recipient?.handle);
  const targetAdapterId = targetMember?.adapter_id;
  const targetCatalog = targetAdapterId ? adapterCatalogs[targetAdapterId] : undefined;
  const targetDefaults = targetAdapterId ? adapterDefaults[targetAdapterId] : undefined;
  const targetKey = targetMember ? recipientKey(targetMember) : undefined;
  const selectedSettings = (targetKey && modelOverrides[targetKey]) || DEFAULT_MODEL_SETTINGS;
  const openModelDraft = targetKey && openMenu ? openMenu.edits[targetKey] ?? selectedSettings : null;

  const liveOptions = targetMember ? memberOptions[targetMember.handle] : undefined;
  const liveEffort = liveOptions?.find(
    (option) =>
      option.category === "thought_level" ||
      option.category === "reasoning_effort" ||
      ["reasoning_effort", "thought_level", "effort"].includes(option.id)
  );
  const discoveredEfforts = liveEffort?.choices;
  const defaultEffortChoices = discoveredEfforts ?? targetCatalog?.efforts ?? [];

  const modelDetailsForTarget = useMemo(() => {
    if (!targetAdapterId) return [];
    const discovered = liveOptions?.find(
      (option) => option.category === "model" || option.id === "model"
    )?.choices ?? [];
    const catalogModels = targetCatalog?.model_details?.length
      ? targetCatalog.model_details
      : (targetCatalog?.models ?? []).map((model) => ({
          ...model,
          default_effort: null,
          supported_efforts: targetCatalog?.efforts ?? [],
        }));
    const merged = new Map<string, Pick<ModelChoiceDetail, "value" | "label" | "default_effort" | "supported_efforts">>(
      catalogModels.map((model) => [model.value, model])
    );
    for (const model of discovered) {
      merged.set(model.value, {
        ...model,
        default_effort: merged.get(model.value)?.default_effort ?? null,
        supported_efforts: merged.get(model.value)?.supported_efforts ?? discoveredEfforts ?? targetCatalog?.efforts ?? [],
      });
    }
    for (const model of [targetMember?.active_model, targetDefaults?.default_model]) {
      if (model && !merged.has(model)) {
        merged.set(model, {
          value: model,
          label: model,
          default_effort: null,
          supported_efforts: discoveredEfforts ?? targetCatalog?.efforts ?? [],
        });
      }
    }
    return [...merged.values()];
  }, [targetAdapterId, targetCatalog, liveOptions, discoveredEfforts, targetMember?.active_model, targetDefaults]);

  const inheritedModel = targetMember?.active_model ??
    targetDefaults?.default_model ?? targetCatalog?.model_details?.find((model) => model.is_default)?.value;
  const selectedModel = selectedSettings.model ?? inheritedModel;
  const modelLabel =
    modelDetailsForTarget.find((model) => model.value === selectedModel)?.label ??
    selectedModel ??
    "Model";
  const effortsForModel = (model: string | null) => {
    const effectiveModel = model ?? inheritedModel;
    return modelDetailsForTarget.find((detail) => detail.value === effectiveModel)?.supported_efforts ??
      defaultEffortChoices;
  };
  const effortChoices = openModelDraft
    ? [...effortsForModel(openModelDraft.model)].sort((left, right) => effortRank(left.value) - effortRank(right.value))
    : [];
  const draftModel = openModelDraft?.model ?? inheritedModel;
  const defaultEffortChoicesInOrder = [
    targetMember?.active_effort,
    targetDefaults?.default_effort,
    modelDetailsForTarget.find((model) => model.value === draftModel)?.default_effort,
    draftModel === inheritedModel ? liveEffort?.current_value : null,
  ];
  const inheritedEffort = defaultEffortChoicesInOrder.find((value) =>
    effortChoices.some((choice) => choice.value === value)
  );
  const effortIndex = effortChoices.findIndex((choice) => choice.value === (openModelDraft?.effort ?? inheritedEffort));
  const currentEffort = effortChoices[effortIndex];
  const effortsLabel = currentEffort ? EFFORT_LABELS[currentEffort.value] ?? currentEffort.label : null;
  const effortLabel = openModelDraft?.effort && effortsLabel
    ? effortsLabel
    : effortsLabel ? "Default · " + effortsLabel : "Harness default";
  const selectedEffortLabel = selectedSettings.effort
    ? EFFORT_LABELS[selectedSettings.effort] ?? selectedSettings.effort
    : null;

  const closeAllMenus = () => {
    setModelMenu(null);
    setPermOpen(false);
    setContextOpen(false);
    setQuotaOpen(false);
  };

  const editModelSettings = (settings: ModelSelection) => {
    if (targetKey && openMenu) {
      setModelMenu({ ...openMenu, edits: { ...openMenu.edits, [targetKey]: settings } });
    }
  };

  const applyModelSettings = () => {
    if (!openMenu) return;
    setModelOverrides((current) => ({ ...current, ...openMenu.edits }));
    setModelMenu(null);
    modelButtonRef.current?.focus();
  };

  const currentPolicy: PermissionPolicy =
    activeSession?.permission_policy ?? pendingPermissionPolicy;
  const approval = APPROVAL_MODES.find((mode) => mode.value === currentPolicy) ?? APPROVAL_MODES[0];

  // Compute displayed Context Breakdown + live Draft tokens
  const draftTokens = draftEstimate?.text === raw
    ? draftEstimate.tokens
    : Math.ceil(Array.from(raw).length / 4);

  const contextMembersList: MemberContextBreakdown[] = useMemo(() => {
    if (sessionContext?.members && sessionContext.members.length > 0) {
      return sessionContext.members;
    }
    return availableMembers.map((m) => ({
      handle: m.handle,
      adapter_id: m.adapter_id,
      model: m.active_model ?? adapterDefaults[m.adapter_id]?.default_model ??
        adapterCatalogs[m.adapter_id]?.model_details?.find((model) => model.is_default)?.value ?? null,
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
      targetMember?.handle ??
      orchestrator?.handle ??
      contextMembersList[0]?.handle;
    return (
      contextMembersList.find((m) => m.handle === preferredHandle) ??
      contextMembersList[0] ??
      null
    );
  }, [contextMemberHandle, targetMember, orchestrator, contextMembersList]);

  const contextTotalWithDraft = activeContextBreakdown
    ? activeContextBreakdown.used_tokens + draftTokens
    : draftTokens;
  const contextMember = availableMembers.find((member) => member.handle === activeContextBreakdown?.handle);
  const contextModel = (contextMember && modelOverrides[recipientKey(contextMember)]?.model) ??
    activeContextBreakdown?.model;
  const reportedContext = activeContextBreakdown?.context_window ?? 200_000;
  const sameModel = contextModel === activeContextBreakdown?.model;
  const modelFallback = contextMember ? adapterCatalogs[contextMember.adapter_id]?.default_context_window : undefined;
  const contextWindow = Math.max(activeContextBreakdown?.is_exact && sameModel
    ? reportedContext : modelContextWindow(contextModel, sameModel ? reportedContext : modelFallback ?? 200_000), 1);
  const contextPct = Math.min(100, (contextTotalWithDraft / contextWindow) * 100);
  const quotaIds = [...new Set(recipients.map((member) => member.adapter_id))];
  const reportedQuotaUse = quotaIds.map((id) => quotaUsed(quotas[id])).filter((value): value is number => value !== null);
  const usage = reportedQuotaUse.length ? Math.max(...reportedQuotaUse) : null;

  const handleSend = async () => {
    if (!raw || sending) return;

    const overrides = Object.fromEntries(recipients.flatMap((member) => {
      const selection = modelOverrides[recipientKey(member)];
      return selection?.model || selection?.effort ? [[member.handle, selection]] : [];
    }));
    const opts = Object.keys(overrides).length ? { recipient_overrides: overrides } : undefined;

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

    if (compactMatch) {
      if (!activeSession) return;
      setText("");
      setSending(true);
      try {
        await compactActiveSession();
        if (routingText) {
          await sendMessage(routingText, opts);
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
    setText("");
    setSending(true);
    try {
      await sendMessage(raw, opts);
    } catch (error) {
      setText((current) => (current ? `${raw}\n${current}` : raw));
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setSending(false);
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
      <div className="relative mx-auto max-w-[52rem] @container">
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
                ? "Message the group… Use @ to mention or / for commands"
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

          <div className="relative flex flex-wrap items-center gap-x-2 gap-y-1.5 border-t border-line pt-2">
            <div className="flex min-w-0 max-w-full flex-wrap items-center gap-1.5 whitespace-nowrap text-[11px]">
              {availableMembers.length > 0 && (
                <div className="@min-[48rem]:relative">
                  <button
                    ref={modelButtonRef}
                    type="button"
                    aria-label="Model and reasoning effort"
                    aria-expanded={Boolean(openMenu)}
                    aria-controls={openMenu ? modelMenuId : undefined}
                    title={
                      needsIntent
                        ? "The orchestrator will resolve the recipients on send"
                        : recipients.length > 1
                        ? "Choose a model and effort for each recipient"
                        : targetMember
                        ? [modelLabel, selectedEffortLabel, "@" + targetMember.handle].filter(Boolean).join(" · ")
                        : "The recipient's harness is unavailable"
                    }
                    disabled={recipients.length < 2 && !targetMember}
                    onClick={() => {
                      closeAllMenus();
                      if (!openMenu) {
                        setModelMenu({ route, tab: 0, edits: {} });
                      }
                    }}
                    className="inline-flex max-w-full items-center gap-1.5 rounded-lg border border-line bg-elevated px-2 py-1 text-ink-soft transition-colors hover:bg-hover hover:text-ink disabled:opacity-65"
                  >
                    <Cpu className="h-3.5 w-3.5 shrink-0 text-brand-ink" />
                    <span className="max-w-40 truncate font-medium">
                      {needsIntent ? "Auto route" : recipients.length > 1 ? `${recipients.length} recipients` : modelLabel}
                    </span>
                    {recipients.length === 1 && selectedEffortLabel && (
                      <span className="hidden shrink-0 text-ink-muted @[30rem]:inline">
                        · {selectedEffortLabel}
                      </span>
                    )}
                    <ChevronDown className="h-3 w-3 shrink-0 text-ink-muted" />
                  </button>

                  {openMenu && recipient && (
                    <>
                      <div className="fixed inset-0 z-20" onClick={closeAllMenus} />
                      <div
                        role="group"
                        id={modelMenuId}
                        aria-label={"Model and effort for @" + recipient.handle}
                        onKeyDown={(event) => {
                          if (event.key === "Escape") {
                            event.preventDefault();
                            closeAllMenus();
                            modelButtonRef.current?.focus();
                          }
                        }}
                        className="absolute bottom-full left-0 z-30 mb-2 w-72 max-w-full rounded-xl border border-line-strong bg-panel text-xs whitespace-normal shadow-2xl @min-[48rem]:max-w-none"
                      >
                        <div className="flex items-center justify-between gap-3 px-3 pt-3 pb-2">
                          <span className="font-semibold text-ink">Model</span>
                          {recipients.length === 1 && (
                            <span className="truncate font-mono text-[11px] text-ink-muted">
                              @{recipient.handle}
                            </span>
                          )}
                        </div>
                        {recipients.length > 1 && (
                          <div
                            role="tablist"
                            aria-label="Message recipients"
                            className="flex gap-1 overflow-x-auto border-b border-line px-2 pb-2"
                            onKeyDown={(event) => {
                              const last = recipients.length - 1;
                              const next = {
                                ArrowLeft: selectedTab === 0 ? last : selectedTab - 1,
                                ArrowRight: selectedTab === last ? 0 : selectedTab + 1,
                                Home: 0,
                                End: last,
                              }[event.key];
                              if (typeof next !== "number") return;
                              event.preventDefault();
                              setModelMenu({ ...openMenu, tab: next });
                              event.currentTarget.querySelectorAll("button")[next]?.focus();
                            }}
                          >
                            {recipients.map((member, index) => (
                              <button
                                key={member.handle}
                                type="button"
                                role="tab"
                                tabIndex={index === selectedTab ? 0 : -1}
                                aria-selected={index === selectedTab}
                                aria-controls={modelMenuId + "-panel"}
                                id={modelMenuId + "-tab-" + index}
                                onClick={() => setModelMenu({ ...openMenu, tab: index })}
                                className={`max-w-40 shrink-0 truncate rounded-md px-2 py-1.5 font-mono ${index === selectedTab
                                  ? "bg-brand-soft text-brand-ink"
                                  : "text-ink-soft hover:bg-hover"}`}
                              >
                                @{member.handle}
                              </button>
                            ))}
                          </div>
                        )}
                        <div
                          role={recipients.length > 1 ? "tabpanel" : undefined}
                          id={modelMenuId + "-panel"}
                          aria-labelledby={recipients.length > 1 ? modelMenuId + "-tab-" + selectedTab : undefined}
                        >
                          {!openModelDraft ? (
                            <p className="px-3 py-4 text-ink-muted">This recipient’s harness is unavailable.</p>
                          ) : (
                            <>
                              <div className="max-h-[min(12rem,28vh)] space-y-0.5 overflow-y-auto px-1.5 pb-1.5">
                                {[{ value: null, label: "Default" }, ...modelDetailsForTarget].map((option) => {
                                  const active = openModelDraft.model === option.value;
                                  return (
                                    <button
                                      key={option.value ?? ""}
                                      type="button"
                                      aria-pressed={active}
                                      onClick={() => {
                                        const effort = effortsForModel(option.value).some(
                                          (choice) => choice.value === openModelDraft.effort
                                        ) ? openModelDraft.effort : null;
                                        editModelSettings({ model: option.value, effort });
                                      }}
                                      className={"flex w-full items-center justify-between gap-2 rounded-lg px-2 py-1.5 text-left transition-colors " +
                                        (active ? "bg-brand-soft font-medium text-ink" : "text-ink-soft hover:bg-hover")}
                                    >
                                      <span className="truncate" title={option.label}>{option.label}</span>
                                      {active && <Check className="h-3.5 w-3.5 shrink-0 text-brand-ink" />}
                                    </button>
                                  );
                                })}
                              </div>
                              {effortChoices.length > 0 && (
                                <div className="border-t border-line px-3 pt-3 pb-1.5">
                                  <span className="flex items-center justify-between gap-2">
                                    <span className="font-medium text-ink-soft">Reasoning effort</span>
                                    <span className="text-brand-ink">{effortLabel}</span>
                                  </span>
                                  <input
                                    type="range"
                                    aria-label="Reasoning effort"
                                    aria-valuetext={effortLabel}
                                    min={0}
                                    max={effortChoices.length - 1}
                                    step={1}
                                    value={Math.max(0, effortIndex)}
                                    onChange={(event) => editModelSettings({
                                      ...openModelDraft,
                                      effort: effortChoices[event.currentTarget.valueAsNumber]?.value ?? null,
                                    })}
                                    className={`mt-1.5 block h-5 w-full rounded cursor-pointer accent-brand focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand-ink ${effortIndex < 0
                                      ? "[&::-webkit-slider-thumb]:opacity-0 [&::-moz-range-thumb]:opacity-0"
                                      : ""}`}
                                  />
                                  {openModelDraft.effort && (
                                    <button type="button" className="text-[10px] text-ink-muted hover:text-brand-ink"
                                      onClick={() => editModelSettings({ ...openModelDraft, effort: null })}>
                                      Reset to default
                                    </button>
                                  )}
                                </div>
                              )}
                            </>
                          )}
                        </div>
                        <div className="flex justify-end border-t border-line px-3 py-2">
                          <button
                            type="button"
                            onClick={applyModelSettings}
                            className="rounded-md bg-brand-soft px-3 py-1.5 font-medium text-brand-ink hover:bg-hover"
                          >
                            Done
                          </button>
                        </div>
                      </div>
                    </>
                  )}
                </div>
              )}

              {/* Permissions Selector Pill */}
              <div className="@min-[48rem]:relative">
                <button
                  type="button"
                  aria-label="Permission policy"
                  title={approval.label + " approval"}
                  onClick={() => {
                    const next = !permOpen;
                    closeAllMenus();
                    setPermOpen(next);
                  }}
                  className="inline-flex items-center gap-1 px-2 py-1 rounded-lg bg-elevated hover:bg-hover border border-line text-ink-soft hover:text-ink transition-colors"
                >
                  <approval.Icon aria-hidden="true" className={`w-3 h-3 shrink-0 ${approval.color}`} />
                  <span className="hidden sm:inline">{approval.label}</span>
                  <ChevronDown className="w-3 h-3 text-ink-muted shrink-0" />
                </button>

                {permOpen && (
                  <>
                    <div
                      className="fixed inset-0 z-20"
                      onClick={() => setPermOpen(false)}
                    />
                    <div className="absolute bottom-full left-0 mb-1.5 w-60 max-w-full rounded-xl border border-line-strong bg-panel shadow-2xl p-1 z-30 space-y-0.5 text-xs whitespace-normal @min-[48rem]:max-w-none">
                      <div className="px-2.5 py-1 text-[10px] uppercase tracking-wider font-semibold text-ink-muted">
                        Approval Mode
                      </div>
                      {APPROVAL_MODES.map(({ value, label, description, Icon, color }) => (
                        <button
                          key={value}
                          type="button"
                          onClick={() => void handlePolicyChange(value)}
                          className={`w-full px-2.5 py-1.5 rounded-lg text-left transition-colors ${
                            currentPolicy === value
                              ? "bg-brand-soft text-ink"
                              : "hover:bg-hover text-ink-soft"
                          }`}
                        >
                          <div className="flex items-center justify-between font-medium">
                            <span className="flex items-center gap-1.5">
                              <Icon aria-hidden="true" className={`h-3.5 w-3.5 ${color}`} />
                              {label}
                            </span>
                            {currentPolicy === value && (
                              <Check className="w-3 h-3 text-brand-ink" />
                            )}
                          </div>
                          <div className="text-[10.5px] text-ink-muted">{description}</div>
                        </button>
                      ))}
                    </div>
                  </>
                )}
              </div>

            </div>

            <div className="ml-auto flex max-w-full flex-wrap items-center justify-end gap-2 whitespace-nowrap">
              {quotaIds.length > 0 && (
                <div className="@min-[48rem]:relative">
                  <button
                    type="button"
                    aria-label="Subscription usage"
                    aria-expanded={quotaOpen}
                    title={usage === null ? "Subscription usage unavailable" : formatPercent(usage) + " of subscription used"}
                    onClick={() => {
                      const next = !quotaOpen;
                      closeAllMenus();
                      setQuotaOpen(next);
                    }}
                    className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-elevated px-2 py-1 font-mono text-[11px] text-ink-muted hover:bg-hover hover:text-ink"
                  >
                    {usage === null
                      ? <CircleGauge className="h-[15px] w-[15px]" />
                      : <ContextRing pct={usage} accent="var(--mh-pos)" />}
                    <span>{usage === null ? "—" : formatPercent(usage)}</span>
                  </button>
                  {quotaOpen && (
                    <>
                      <div className="fixed inset-0 z-20" onClick={closeAllMenus} />
                      <div className="absolute bottom-full right-0 z-30 mb-2 w-64 max-w-full max-h-64 space-y-3 overflow-y-auto rounded-xl border border-line-strong bg-panel p-3 text-xs whitespace-normal shadow-2xl @min-[48rem]:max-w-none">
                        <h3 className="font-semibold text-ink">Subscription usage</h3>
                        {quotaIds.map((id) => (
                          <div key={id} className="space-y-1.5">
                            <p className="font-medium text-ink-soft">{adapters[id]?.display_name ?? id}</p>
                            {quotas[id] ? <QuotaSnapshotSummary quota={quotas[id]} compact /> : (
                              <p className="text-[10px] text-ink-muted">Usage unavailable</p>
                            )}
                          </div>
                        ))}
                      </div>
                    </>
                  )}
                </div>
              )}
              {/* Context Usage Ring & Composition Popover (anchored inward right-0) */}
              {activeContextBreakdown && (
                <div className="@min-[48rem]:relative">
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
                      {formatPercent(contextPct)} · {formatCompactTokens(contextTotalWithDraft)}
                    </span>
                  </button>

                  {contextOpen && (
                    <>
                      <div
                        className="fixed inset-0 z-20"
                        onClick={() => setContextOpen(false)}
                      />
                      <div className="absolute bottom-full right-0 mb-2 w-80 max-w-full rounded-xl border border-line-strong bg-panel shadow-2xl p-3.5 z-30 space-y-3 text-xs whitespace-normal @min-[48rem]:max-w-none">
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

function ContextRing({ pct, accent = "var(--mh-brand)" }: { pct: number; accent?: string }) {
  const r = 5.5;
  const circ = 2 * Math.PI * r;
  const offset = circ - (Math.max(0, Math.min(100, pct)) / 100) * circ;
  const stroke = pct >= 85 ? "var(--mh-neg)" : pct >= 60 ? "var(--mh-warn)" : accent;
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
