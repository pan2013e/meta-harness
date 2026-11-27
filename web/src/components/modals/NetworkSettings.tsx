import { useEffect, useState } from "react";

export function NetworkSettings() {
  const desktop = window.metaHarnessDesktop;
  const [configured, setConfigured] = useState<string | null>(null);
  const [proxyNotUsed, setProxyNotUsed] = useState(false);
  const [value, setValue] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!desktop) return;
    let active = true;
    void desktop.proxySettings().then(({ host, proxyNotUsed }) => {
      if (active) { setConfigured(host); setProxyNotUsed(proxyNotUsed); }
    }).catch(() => {
      if (active) setError("Could not read the desktop proxy setting.");
    }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [desktop]);

  const save = async (proxy: string | null) => {
    if (!desktop || saving) return;
    setSaving(true);
    setError(null);
    try {
      await desktop.saveProxy(proxy);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setSaving(false);
    }
  };

  return (
    <form className="space-y-4 text-sm" onSubmit={(event) => { event.preventDefault(); if (value.trim()) void save(value.trim()); }}>
      <div>
        <h2 className="font-semibold">Outbound proxy</h2>
        <p className="mt-1 text-xs text-ink-muted">Applies to Meta-Harness and its agent CLIs after restart.</p>
      </div>
      {!loading && <p className="flex flex-wrap items-center gap-2 text-xs text-ink-soft">
        {configured ? `Configured proxy · ${configured}` : "Using startup environment or system proxy settings"}
        {proxyNotUsed && <span className="rounded-md bg-warn-soft px-1.5 py-0.5 text-warn" title="The configured proxy could not be reached. Connections are direct.">Proxy not used</span>}
      </p>}
      <label className="block space-y-1 text-xs text-ink-soft">
        <span>Proxy URL</span>
        <input type="password" autoComplete="off" value={value} maxLength={2048}
          onChange={(event) => setValue(event.target.value)} placeholder="http://proxy.example:8080"
          className="w-full rounded-lg border border-line bg-inset px-3 py-2 text-sm text-ink" />
      </label>
      <div className="flex flex-wrap gap-2 text-xs">
        <button type="submit" disabled={loading || saving || !value.trim()}
          className="rounded-lg bg-brand px-3 py-1.5 font-medium text-white hover:bg-brand-hover disabled:opacity-50">Save and restart</button>
        {configured && <button type="button" disabled={saving} onClick={() => void save(null)}
          className="rounded-lg border border-line bg-elevated px-3 py-1.5 hover:bg-hover disabled:opacity-50">Use automatic proxy and restart</button>}
      </div>
      {error && <p role="alert" className="text-xs text-neg">{error}</p>}
    </form>
  );
}
