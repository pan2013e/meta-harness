import { useState } from "react";
import { api } from "../../api/client";
import type { QuotaWindow } from "../../types/api";
import { QuotaMeter } from "../common/Badges";

export function SharedAntigravityUsage({ adapterId }: { adapterId: string }) {
  const [windows, setWindows] = useState<QuotaWindow[]>([]);
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const check = async () => {
    setChecking(true);
    setError(null);
    try {
      setWindows((await api.sharedAntigravityUsage(adapterId)).windows);
    } catch (cause) {
      setWindows([]);
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setChecking(false);
    }
  };

  return (
    <div className="mt-2 space-y-2">
      <button type="button" disabled={checking} onClick={() => void check()}
        className="rounded-lg border border-line bg-elevated px-2.5 py-1.5 text-xs text-ink-soft hover:bg-hover disabled:opacity-50">
        {checking ? "Checking shared usage…" : "Check shared CLI usage"}
      </button>
      {error && <p className="text-xs text-neg" role="alert">{error}</p>}
      {windows.length > 0 && (
        <div className="space-y-2 rounded-lg border border-line bg-inset p-2.5">
          <h3 className="text-xs font-medium text-ink-soft">Shared CLI account</h3>
          {windows.map((window) => <QuotaMeter key={window.id}
            label={window.label} pct={window.remaining_pct} resetsAt={window.resets_at} />)}
        </div>
      )}
    </div>
  );
}
