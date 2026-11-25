import type { MemberStatus } from "../../types/api";

export function StatusDot({ status }: { status?: MemberStatus }) {
  const s = status ?? "hibernated";
  const color =
    s === "thinking" || s === "running_tool" || s === "spawning"
      ? "bg-amber-400 animate-pulse"
      : s === "consulting"
        ? "bg-violet-400 animate-pulse"
        : s === "waiting_permission"
          ? "bg-rose-400 animate-ping"
          : s === "idle"
            ? "bg-emerald-400"
            : s === "error"
              ? "bg-red-500"
              : "bg-zinc-600";

  const label =
    s === "hibernated"
      ? "Hibernated"
      : s === "running_tool"
        ? "Running Tool"
        : s === "waiting_permission"
          ? "Needs Approval"
          : s.charAt(0).toUpperCase() + s.slice(1);

  return (
    <span className="inline-flex items-center gap-1.5 text-[11px] text-zinc-400">
      <span className={`h-2 w-2 rounded-full ${color}`} />
      <span>{label}</span>
    </span>
  );
}

export function QuotaMeter({
  label,
  pct,
  rateLimited,
}: {
  label: string;
  pct: number | null;
  rateLimited?: boolean;
}) {
  const val = rateLimited ? 0 : Math.max(0, Math.min(100, pct ?? 100));
  const barColor = rateLimited
    ? "bg-red-500"
    : val > 50
      ? "bg-emerald-500"
      : val > 20
        ? "bg-amber-500"
        : "bg-rose-500";

  return (
    <div className="space-y-0.5">
      <div className="flex items-center justify-between text-[10px] text-zinc-400">
        <span>{label}</span>
        <span className={rateLimited ? "text-red-400 font-semibold" : "font-mono"}>
          {rateLimited ? "429 COOLDOWN" : pct === null ? "n/a" : `${Math.round(val)}%`}
        </span>
      </div>
      <div className="h-1 w-full rounded-full bg-zinc-800 overflow-hidden">
        <div
          className={`h-full transition-all duration-300 ${barColor}`}
          style={{ width: `${val}%` }}
        />
      </div>
    </div>
  );
}
