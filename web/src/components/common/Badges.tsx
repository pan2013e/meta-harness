import { User } from "lucide-react";
import type { MemberStatus, QuotaSnapshot, ThemeId } from "../../types/api";

export const THEMES: Array<{
  id: ThemeId;
  label: string;
  desc: string;
  swatch: [string, string, string];
}> = [
  {
    id: "obsidian",
    label: "Obsidian Studio",
    desc: "Deep slate & indigo",
    swatch: ["#090b10", "#151b26", "#6366f1"],
  },
  {
    id: "tokyo-night",
    label: "Tokyo Night",
    desc: "Storm midnight blue",
    swatch: ["#161722", "#222639", "#7aa2f7"],
  },
  {
    id: "catppuccin",
    label: "Catppuccin Mocha",
    desc: "Warm espresso & mauve",
    swatch: ["#181825", "#28283c", "#cba6f7"],
  },
  {
    id: "daylight",
    label: "Paper Daylight",
    desc: "Crisp high-contrast light",
    swatch: ["#f4f6f9", "#ffffff", "#4f46e5"],
  },
];

export function HarnessAvatar({
  handle,
  accentColor,
  status,
  isUser = false,
  size = "md",
}: {
  handle: string;
  accentColor?: string;
  status?: MemberStatus;
  isUser?: boolean;
  size?: "sm" | "md";
}) {
  const dim = size === "sm" ? "w-6 h-6 text-[10px]" : "w-8 h-8 text-xs";
  const ring = accentColor ?? "#6366f1";
  const mono = isUser
    ? null
    : handle.replace(/^@/, "").slice(0, 2).toUpperCase();

  const dotColor =
    status === "thinking" || status === "running_tool" || status === "spawning"
      ? "bg-amber-400 animate-pulse"
      : status === "consulting"
        ? "bg-violet-400 animate-pulse"
        : status === "waiting_permission"
          ? "bg-rose-500"
          : status === "idle"
            ? "bg-emerald-400"
            : status === "error"
              ? "bg-red-500"
              : null;

  return (
    <div className="relative shrink-0">
      <div
        className={`${dim} rounded-lg flex items-center justify-center font-mono font-bold border`}
        style={
          isUser
            ? {
                backgroundColor: "var(--mh-brand-soft)",
                borderColor: "var(--mh-brand)",
                color: "var(--mh-brand-ink)",
              }
            : {
                backgroundColor: `${ring}1e`,
                borderColor: `${ring}55`,
                color: ring,
              }
        }
      >
        {isUser ? <User className="w-4 h-4" /> : mono}
      </div>
      {dotColor && (
        <span
          className={`absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full ring-2 ring-panel ${dotColor}`}
        />
      )}
    </div>
  );
}

export function StatusDot({ status }: { status?: MemberStatus }) {
  const s = status ?? "hibernated";
  const color =
    s === "thinking" || s === "running_tool" || s === "spawning"
      ? "bg-amber-400 animate-pulse"
      : s === "consulting"
        ? "bg-violet-400 animate-pulse"
        : s === "waiting_permission"
          ? "bg-rose-500 animate-ping"
          : s === "idle"
            ? "bg-emerald-400"
            : s === "error"
              ? "bg-red-500"
              : "bg-ink-muted/60";

  const label =
    s === "hibernated"
      ? "Standby"
      : s === "running_tool"
        ? "Running tool"
        : s === "waiting_permission"
          ? "Needs approval"
          : s.charAt(0).toUpperCase() + s.slice(1);

  return (
    <span className="inline-flex items-center gap-1.5 text-[11px] text-ink-muted">
      <span className={`h-1.5 w-1.5 rounded-full ${color}`} />
      <span>{label}</span>
    </span>
  );
}

export function formatResetCountdown(resetsAt?: number | null): string | null {
  if (!resetsAt) return null;
  const diffMs = resetsAt - Date.now();
  if (diffMs <= 0) return "Resets soon";
  const mins = Math.floor(diffMs / 60000);
  if (mins < 60) return `${Math.max(1, mins)}m`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) {
    const remM = mins % 60;
    return remM > 0 ? `${hrs}h ${remM}m` : `${hrs}h`;
  }
  const days = Math.floor(hrs / 24);
  const remH = hrs % 24;
  return remH > 0 ? `${days}d ${remH}h` : `${days}d`;
}

export function QuotaMeter({
  label,
  pct,
  rateLimited,
  resetsAt,
}: {
  label: string;
  pct: number | null;
  rateLimited?: boolean;
  resetsAt?: number | null;
}) {
  const unavailable = !rateLimited && pct === null;
  const val = rateLimited ? 0 : unavailable ? 0 : Math.max(0, Math.min(100, pct ?? 0));
  const barColor = rateLimited
    ? "bg-neg"
    : val > 50
      ? "bg-pos"
      : val > 15
        ? "bg-warn"
        : "bg-neg";

  const countdown = formatResetCountdown(resetsAt);

  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between text-[10px]">
        <span className="text-ink-soft font-medium">{label}</span>
        <div className="flex items-center gap-1.5">
          {countdown && pct !== null && (
            <span className="text-ink-muted font-mono">
              ↻ {countdown}
            </span>
          )}
          <span
            className={
              rateLimited || (pct !== null && val <= 0.5)
                ? "text-neg font-semibold font-mono"
                : unavailable
                  ? "text-ink-muted font-mono"
                  : "text-ink font-mono font-medium"
            }
          >
            {rateLimited
              ? "429 COOLDOWN"
              : unavailable
                ? "n/a"
                : `${Math.round(val)}%`}
          </span>
        </div>
      </div>
      <div className="h-1.5 w-full rounded-full bg-inset overflow-hidden border border-line/60">
        {!unavailable && (
          <div
            className={`h-full rounded-full transition-all duration-300 ${barColor}`}
            style={{ width: `${val}%` }}
          />
        )}
      </div>
    </div>
  );
}

export function QuotaSnapshotSummary({
  quota,
  compact = false,
}: {
  quota: QuotaSnapshot;
  compact?: boolean;
}) {
  const windows =
    quota.windows && quota.windows.length > 0
      ? quota.windows
      : [
          ...(quota.window_5h_remaining_pct !== null
            ? [
                {
                  id: "5h",
                  label: "5h",
                  remaining_pct: quota.window_5h_remaining_pct,
                  resets_at: quota.window_5h_resets_at,
                  window_mins: 300,
                },
              ]
            : []),
          ...(quota.weekly_remaining_pct !== null
            ? [
                {
                  id: "7d",
                  label: "Weekly",
                  remaining_pct: quota.weekly_remaining_pct,
                  resets_at: quota.weekly_resets_at,
                  window_mins: 10080,
                },
              ]
            : []),
        ];

  const isCooldown = quota.rate_limited_until !== null || quota.source === "cooldown429";

  return (
    <div className="space-y-1.5">
      {isCooldown && windows.length === 0 && (
        <QuotaMeter label="Limit" pct={0} rateLimited resetsAt={quota.rate_limited_until} />
      )}

      {windows.map((w) => (
        <QuotaMeter
          key={w.id}
          label={compact && w.label === "Weekly" ? "7d" : w.label}
          pct={w.remaining_pct}
          rateLimited={isCooldown}
          resetsAt={w.resets_at}
        />
      ))}

      {windows.length === 0 && !isCooldown && (
        <div className="text-[10px] text-ink-muted flex items-center justify-between gap-2 py-0.5">
          <span className="truncate">
            {quota.status_note ?? "Quota unavailable"}
          </span>
        </div>
      )}

      {!compact && (quota.status_note || quota.credits_label) && windows.length > 0 && (
        <div className="flex items-center justify-between gap-2 text-[10px] text-ink-muted pt-0.5">
          {quota.status_note && <span className="truncate">{quota.status_note}</span>}
          {quota.credits_label && (
            <span className="font-mono text-ink-soft shrink-0">{quota.credits_label}</span>
          )}
        </div>
      )}
    </div>
  );
}
