import { notifyInApp, reportInAppError } from "../../inAppNotifications";
import { useState } from "react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";

const buttonClass = "settings-button";

export function SettingsBackup() {
  const theme = useAppStore((state) => state.theme);
  const importSettings = useAppStore((state) => state.importSettings);
  const [file, setFile] = useState<File | null>(null);
  const [working, setWorking] = useState(false);

  const run = async (action: () => Promise<void>) => {
    setWorking(true);
    try {
      await action();
    } catch (error) {
      reportInAppError(error);
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
    notifyInApp("Settings exported.", "success");
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
    notifyInApp("Settings imported.", "success");
  });

  return (
    <section className="settings-card space-y-3">
      <h2 className="settings-heading">Settings backup</h2>
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
    </section>
  );
}
