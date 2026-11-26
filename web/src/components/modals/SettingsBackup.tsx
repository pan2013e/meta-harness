import { useState } from "react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";

const buttonClass = "rounded-lg border border-line bg-elevated px-3 py-1.5 text-ink-soft hover:bg-hover disabled:opacity-50";

export function SettingsBackup() {
  const theme = useAppStore((state) => state.theme);
  const importSettings = useAppStore((state) => state.importSettings);
  const [file, setFile] = useState<File | null>(null);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const run = async (action: () => Promise<void>) => {
    setWorking(true);
    setNotice(null);
    setError(null);
    try {
      await action();
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setWorking(false);
    }
  };

  const download = () => run(async () => {
    const settings = { ...(await api.exportSettings()), theme };
    const blob = new Blob([JSON.stringify(settings, null, 2) + "\n"], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = "meta-harness-settings.json";
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    setNotice("Settings exported.");
  });

  const upload = () => run(async () => {
    if (!file) return;
    if (file.size > 2 * 1024 * 1024) throw new Error("Choose a settings JSON file under 2 MB.");
    let document: unknown;
    try {
      document = JSON.parse(await file.text());
    } catch {
      throw new Error("This file is not valid JSON.");
    }
    await importSettings(document);
    setFile(null);
    setNotice("Settings imported.");
  });

  return (
    <section className="space-y-2 rounded-xl border border-line bg-inset p-3">
      <h2 className="font-semibold text-ink">Settings backup</h2>
      <p className="text-ink-muted">Transfer your theme, harness model defaults, quota preferences, and global personas. Import merges records by harness or persona handle.</p>
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" className={buttonClass} disabled={working} onClick={() => void download()}>
          Export settings
        </button>
        <label className={buttonClass + " relative cursor-pointer focus-within:outline-2 focus-within:outline-brand-ink"}>
          Choose JSON file
          <input aria-label="Settings JSON file" type="file" accept=".json,application/json" disabled={working}
            className="sr-only" onChange={(event) => {
              setFile(event.target.files?.[0] ?? null);
              event.currentTarget.value = "";
              setNotice(null);
              setError(null);
            }} />
        </label>
        {file && (
          <>
            <span className="max-w-56 truncate text-ink-muted" title={file.name}>{file.name}</span>
            <button type="button" className={buttonClass} disabled={working} onClick={() => void upload()}>
              Import settings
            </button>
          </>
        )}
      </div>
      {notice && <p role="status" className="text-pos">{notice}</p>}
      {error && <p role="alert" className="text-neg">{error}</p>}
    </section>
  );
}
