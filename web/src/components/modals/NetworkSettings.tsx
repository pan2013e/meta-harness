import { notifyInApp, reportInAppError } from "../../inAppNotifications";
import { useEffect, useState } from "react";
import { api, type OutboundProxyStatus } from "../../api/client";

export function NetworkSettings() {
  const desktop = window.metaHarnessDesktop;
  const [status, setStatus] = useState<OutboundProxyStatus | null>(null);
  const [value, setValue] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [checking, setChecking] = useState(false);
  const [unreachable, setUnreachable] = useState(false);

  useEffect(() => {
    const abort = new AbortController();
    let pending = false;
    const readStatus = async (initial = false) => {
      if (pending) return;
      pending = true;
      try {
        const next = await api.proxyStatus(abort.signal);
        if (!abort.signal.aborted) {
          setStatus((previous) => previous && (
            previous.generation > next.generation ||
            (previous.generation === next.generation && (previous.checked_at ?? 0) > (next.checked_at ?? 0))
          ) ? previous : next);
        }
      } catch {
        if (initial && !abort.signal.aborted) notifyInApp("Could not read the server proxy status.", "error");
      } finally {
        pending = false;
        if (initial && !abort.signal.aborted) setLoading(false);
      }
    };
    void readStatus(true);
    const timer = window.setInterval(() => void readStatus(), 5000);
    return () => { abort.abort(); window.clearInterval(timer); };
  }, []);

  const check = async () => {
    if (checking) return;
    setChecking(true);
    try {
      setStatus(await (desktop?.checkProxy?.() ?? api.checkProxy()));
    } catch (cause) {
      reportInAppError(cause);
    } finally {
      setChecking(false);
    }
  };

  const save = async (proxy: string | null, allowUnreachable = false) => {
    if (!desktop?.saveProxy || saving) return;
    setSaving(true);
    try {
      const result = await desktop.saveProxy(proxy, allowUnreachable);
      setUnreachable(!result.saved);
      if (result.saved) {
        setStatus(result.status);
        setValue("");
        notifyInApp(proxy ? "Proxy settings saved." : "Automatic proxy settings restored.", "success");
      }
    } catch (cause) {
      reportInAppError(cause);
    } finally {
      setSaving(false);
    }
  };

  const route = status?.proxy_not_used
    ? "a direct connection"
    : status?.host ? `the proxy at ${status.host}` : "automatic proxy settings";

  return (
    <form className="settings-card space-y-4 text-sm" onSubmit={(event) => { event.preventDefault(); if (value.trim()) void save(value.trim()); }}>
      <div>
        <h2 className="settings-heading">Outbound proxy</h2>
        {!desktop?.saveProxy && <p className="mt-1 text-xs text-ink-muted">
          Set an explicit server proxy with <code>meta-harness --proxy &lt;URL&gt;</code> to enable health checks.
        </p>}
      </div>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="space-y-1 text-xs text-ink-soft">
          <p role="status" className="flex flex-wrap items-center gap-2">
            {loading ? "Reading proxy status…" : status ? `Meta-Harness is currently using ${route}.` : "Proxy status unavailable."}
            {status?.host && (status.proxy_not_used ? (
              <span className="rounded-md bg-warn-soft px-1.5 py-0.5 text-warn" title="The connectivity check failed. New connections use the direct route.">Proxy not used</span>
            ) : <span className="rounded-md bg-pos-soft px-1.5 py-0.5 text-pos">Proxy active</span>)}
          </p>
          {status?.checked_at && (
            <p className="text-ink-muted">Last checked {new Date(status.checked_at * 1000).toLocaleTimeString()}</p>
          )}
        </div>
        {status?.host && <button type="button" className="settings-button" disabled={checking || saving} onClick={() => void check()}>
          {checking ? "Checking…" : "Check now"}
        </button>}
      </div>
      {desktop?.saveProxy && <label className="block space-y-1 text-xs text-ink-soft">
        <span>Proxy URL</span>
        <input type="text" autoComplete="off" autoCapitalize="none" spellCheck={false} value={value} maxLength={2048} disabled={saving}
          onChange={(event) => { setValue(event.target.value); setUnreachable(false); }} placeholder="http://proxy.example:8080"
          className="w-full rounded-lg border border-line bg-inset px-3 py-2 text-sm text-ink" />
      </label>}
      {desktop?.saveProxy && <div className="flex flex-wrap gap-2 text-xs">
        <button type="submit" disabled={loading || saving || checking || !value.trim()}
          className="rounded-lg bg-brand px-3 py-1.5 font-medium text-white hover:bg-brand-hover disabled:opacity-50">
          {saving ? "Checking proxy…" : "Save proxy"}
        </button>
        {status?.desktop_override && <button type="button" className="settings-button" disabled={saving || checking}
          onClick={() => void save(null)}>Restore automatic settings</button>}
      </div>}
      {unreachable && <div className="flex flex-wrap items-center gap-2 rounded-lg bg-warn-soft px-3 py-2 text-xs text-warn">
        <p role="status" className="flex-1">This proxy failed the internet check. If saved, Meta-Harness will connect directly until the proxy recovers.</p>
        <button type="button" className="settings-button" disabled={saving || checking}
          onClick={() => void save(value.trim(), true)}>Save anyway</button>
      </div>}
    </form>
  );
}
