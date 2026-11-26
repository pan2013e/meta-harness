import { useState } from "react";
import { ArrowDownToLine, RefreshCw } from "lucide-react";
import { api } from "../../api/client";
import type { AppUpdateCheck } from "../../types/api";

declare global {
  interface Window {
    metaHarnessDesktop?: {
      installUpdate: (version: string) => Promise<void>;
    };
  }
}

function statusText(result: AppUpdateCheck) {
  switch (result.status) {
    case "current": return "Meta-Harness is up to date.";
    case "no_release": return "GitHub has no published release yet.";
    case "missing_asset": return `Version ${result.latest_version} has no verified installer or binary for this platform.`;
    case "unsupported": return "Updates are unavailable for this build.";
    case "available": return `Version ${result.latest_version} is available.`;
  }
}

export function AboutUpdates() {
  const desktop = window.metaHarnessDesktop;
  const [result, setResult] = useState<AppUpdateCheck | null>(null);
  const [busy, setBusy] = useState<"checking" | "updating" | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function check() {
    setError(null);
    setResult(null);
    setBusy("checking");
    try {
      setResult(await api.checkForUpdate(desktop ? "desktop" : "cli"));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not check for updates.");
    } finally {
      setBusy(null);
    }
  }

  async function install(version: string) {
    if (!desktop) return;
    setError(null);
    setBusy("updating");
    try {
      await desktop.installUpdate(version);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not prepare the update.");
      setBusy(null);
    }
  }

  const available = result?.status === "available" && result.latest_version;

  return (
    <div className="space-y-2 border-t border-line pt-4">
      <h3 className="font-medium">Updates</h3>
      <div className="flex flex-wrap gap-2">
        <button type="button" disabled={busy !== null} onClick={() => void check()}
          className="inline-flex items-center gap-2 rounded-lg border border-line px-3 py-1.5 text-xs hover:bg-hover disabled:opacity-50">
          <RefreshCw className={`h-3.5 w-3.5 ${busy === "checking" ? "animate-spin" : ""}`} />
          {busy === "checking" ? "Checking…" : "Check for updates"}
        </button>
        {desktop && available && (
          <button type="button" disabled={busy !== null} onClick={() => void install(available)}
            className="inline-flex items-center gap-2 rounded-lg bg-brand px-3 py-1.5 text-xs font-medium text-white hover:bg-brand-hover disabled:opacity-50">
            <ArrowDownToLine className="h-3.5 w-3.5" />
            {busy === "updating" ? "Downloading and verifying…" : "Update and quit"}
          </button>
        )}
      </div>
      {error && <p role="alert" className="text-xs text-neg">{error}</p>}
      {result && <p role="status" className="text-xs text-ink-muted">{statusText(result)}</p>}
      {available && !desktop && <p className="text-xs text-ink-muted">Run <code>meta-harness update</code> on the machine hosting this app.</p>}
      {available && desktop && <p className="text-xs text-ink-muted">Meta-Harness will close before opening the installer.</p>}
      {result && result.status !== "current" && (
        <a href={result.release_url} target="_blank" rel="noreferrer" className="text-xs text-brand-ink underline">View GitHub releases</a>
      )}
    </div>
  );
}
