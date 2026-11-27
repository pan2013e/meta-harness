import { useState } from "react";
import { browserNotificationPermission, showNotification } from "../../notifications";
import { useAppStore } from "../../store/useAppStore";

const options = [
  ["approval_requests", "Approval requests"],
  ["turn_completions", "Completed and failed turns"],
] as const;

export function NotificationSettings() {
  const prefs = useAppStore((state) => state.notificationPreferences);
  const save = useAppStore((state) => state.saveNotificationPreferences);
  const desktop = Boolean(window.metaHarnessDesktop?.notify);
  const [permission, setPermission] = useState(browserNotificationPermission);
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const allowed = desktop || permission === "granted";

  return (
    <section className="space-y-4 text-sm">
      <div>
        <h2 className="font-semibold">Notifications</h2>
        <p className="mt-1 text-xs text-ink-muted">Get alerts while you’re away from the relevant session.</p>
      </div>
      <fieldset disabled={saving} className="space-y-2">
        {options.map(([key, label]) => (
          <label key={key} className="flex items-center justify-between gap-4 rounded-xl border border-line bg-inset px-3 py-2.5 text-sm">
            {label}
            <input type="checkbox" checked={prefs[key]} className="accent-brand" onChange={(event) => {
              setSaving(true);
              setStatus(null);
              void save({ ...prefs, [key]: event.target.checked }).catch((error: unknown) => {
                setStatus(error instanceof Error ? error.message : String(error));
              }).finally(() => setSaving(false));
            }} />
          </label>
        ))}
      </fieldset>
      {!desktop && permission === "default" && (
        <button type="button" className="rounded-lg border border-line bg-elevated px-3 py-1.5 text-xs hover:bg-hover"
          onClick={() => void Notification.requestPermission().then(setPermission).catch(() => setStatus("Could not request browser notification permission."))}>
          Enable browser notifications
        </button>
      )}
      {!desktop && permission === "denied" && <p className="text-xs text-ink-muted">Notifications are blocked for this site in your browser settings.</p>}
      {!desktop && permission === "unavailable" && <p className="text-xs text-ink-muted">Browser notifications are unavailable on this page.</p>}
      {allowed && (
        <button type="button" className="rounded-lg border border-line bg-elevated px-3 py-1.5 text-xs hover:bg-hover"
          onClick={() => void showNotification("This is a test notification.").then((shown) => {
            setStatus(shown ? null : "Notifications are unavailable on this device.");
          }).catch(() => setStatus("Could not send a notification."))}>
          Send test notification
        </button>
      )}
      {status && <p role="status" className="text-xs text-ink-muted">{status}</p>}
    </section>
  );
}
