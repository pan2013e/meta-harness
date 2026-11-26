import type { TrajectoryEvent } from "../../types/api";

const OUTPUT_LIMIT = 24_000;

export interface ToolEntry {
  type: "tool";
  key: string;
  time: number;
  title?: string;
  name?: string;
  category?: string;
  status?: string;
  cwd?: string;
  exitCode?: number;
  input?: unknown;
  result?: unknown;
  output: string;
  outputTruncated: boolean;
  content: string[];
  edits: Array<{ path: string; diff?: string; before?: string; after?: string }>;
}

export type TrajectoryEntry = ToolEntry | { type: "event"; key: string; event: TrajectoryEvent };

const switchActions: Record<string, [action: string, completed: string]> = {
  chat_list_peers: ["list group members", "listed group members"],
  chat_read_messages: ["read group chat messages", "read group chat messages"],
  chat_send_message: ["send a group message", "sent a group message"],
  peer_consult: ["consult another agent", "consulted another agent"],
  trajectory_list: ["list agent activity", "listed agent activity"],
  trajectory_read: ["review an agent's activity", "reviewed an agent's activity"],
  memory_recall: ["look up project memory", "looked up project memory"],
  memory_record_fact: ["save a project note", "saved a project note"],
  quota_get_status: ["check subscription usage", "checked subscription usage"],
  models_list_catalog: ["check available models", "checked available models"],
  orchestrator_dispatch: ["request a task handoff", "requested a task handoff"],
  memory_curate_job: ["save a job summary", "saved a job summary"],
};

export function switchToolLabel(entry: ToolEntry, actor?: string): string | undefined {
  const switchName = /^(?:Run\s+)?(?:mcp_meta-harness-switch_|mcp__meta-harness-switch__|mcp\.meta-harness-switch\.|meta-harness-switch\/)([a-z][a-z_]+)$/i;
  const name = (switchName.exec(entry.name ?? "")?.[1] ?? switchName.exec(entry.title ?? "")?.[1])?.toLowerCase();
  if (!name) return;
  const [action, completed] = Object.hasOwn(switchActions, name) ? switchActions[name] : ["use group tools", "used group tools"];
  if (entry.status === "completed") return actor ? `${actor} ${completed}` : capitalize(completed);
  if (entry.status === "failed") return `Couldn’t ${action}`;
  return capitalize(action);
}

function capitalize(text: string) {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

export const asRecord = (value: unknown): Record<string, unknown> =>
  value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};

const asText = (value: unknown) => typeof value === "string" ? value : undefined;

export function cleanWorktreePath(raw: string): string {
  const slash = raw.replace(/\\/g, "/");
  const idx = slash.toLowerCase().indexOf(".meta-harness/worktrees/");
  if (idx !== -1) {
    const rest = slash.slice(idx + ".meta-harness/worktrees/".length);
    const parts = rest.split("/");
    if (parts.length >= 3) {
      return parts.slice(2).join("/");
    }
  }
  return slash;
}

/** Groups ACP tool patches, including Codex's terminal metadata in previously saved turns. */
export function groupTrajectoryEvents(events: TrajectoryEvent[]): TrajectoryEntry[] {
  const entries: TrajectoryEntry[] = [];
  const tools = new Map<string, ToolEntry>();
  const terminalLinks = new Map<string, string>();
  for (const { payload } of events) {
    const toolId = asText(payload.toolCallId);
    if (!toolId || !Array.isArray(payload.content)) continue;
    for (const item of payload.content) {
      const content = asRecord(item);
      if (content.type === "terminal" && typeof content.terminalId === "string") {
        terminalLinks.set(content.terminalId, toolId);
      }
    }
  }

  for (const event of events) {
    const { payload, event_kind: kind } = event;
    const toolId = asText(payload.toolCallId) ?? asText(payload.tool_call_id);
    const terminalId = asText(payload.terminal_id);
    const linkedId = toolId ?? (terminalId ? terminalLinks.get(terminalId) : undefined);
    if (kind !== "tool_call" && kind !== "terminal" && !(kind === "diff" && linkedId)) {
      entries.push({ type: "event", key: `event:${event.id}`, event });
      continue;
    }

    const key = linkedId ? `tool:${linkedId}` : terminalId ? `terminal:${terminalId}` : `event:${event.id}`;
    let tool = tools.get(key);
    if (!tool) {
      tool = { type: "tool", key, time: event.created_at, output: "", outputTruncated: false, content: [], edits: [] };
      tools.set(key, tool);
      entries.push(tool);
    }

    const meta = asRecord(payload._meta);
    const legacyOutput = asRecord(meta.terminal_output_delta ?? meta.terminal_output);
    const terminalInfo = asRecord(meta.terminal_info);
    const terminalExit = asRecord(meta.terminal_exit);
    const input = asRecord(payload.rawInput);
    const result = asRecord(payload.rawOutput);
    const exitCode = payload.exit_code ?? terminalExit.exit_code ?? result.exit_code;
    if (typeof exitCode === "number") tool.exitCode = exitCode;
    tool.title = asText(payload.title) ?? asText(payload.command) ?? tool.title;
    tool.name = asText(payload.name) ?? tool.name;
    tool.category = asText(payload.kind) ?? tool.category;
    tool.cwd = asText(payload.cwd ?? terminalInfo.cwd ?? input.cwd) ?? tool.cwd;
    tool.status = asText(payload.status) ?? tool.status;
    if (payload.rawInput != null) tool.input = payload.rawInput;
    if (payload.rawOutput != null) tool.result = payload.rawOutput;

    const delta = asText(payload.output_delta ?? legacyOutput.data ?? payload.output_tail);
    if (delta) {
      const output = tool.output + delta;
      tool.outputTruncated ||= output.length > OUTPUT_LIMIT;
      tool.output = output.slice(-OUTPUT_LIMIT);
    }

    if (kind === "diff" && typeof payload.diff === "string") {
      tool.edits.push({
        path: cleanWorktreePath(asText(payload.path) ?? "File edit"),
        diff: payload.diff,
      });
    }
    if (Array.isArray(payload.content)) {
      const content: string[] = [];
      for (const item of payload.content) {
        const block = asRecord(item);
        const text = asText(asRecord(block.content).text ?? block.text);
        if (text) content.push(text);
        if (block.type === "diff" && typeof block.path === "string") {
          tool.edits.push({
            path: cleanWorktreePath(block.path),
            before: asText(block.oldText),
            after: asText(block.newText),
          });
        }
      }
      tool.content = content;
    }
  }
  return entries;
}
