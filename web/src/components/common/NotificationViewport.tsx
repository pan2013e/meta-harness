import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { CircleAlert, CircleCheck, CircleX, Info, X } from "lucide-react";
import { useInAppNotifications, type InAppNotification } from "../../inAppNotifications";

const appearance = {
  info: { Icon: Info, color: "text-brand-ink" },
  success: { Icon: CircleCheck, color: "text-pos" },
  warning: { Icon: CircleAlert, color: "text-warn" },
  error: { Icon: CircleX, color: "text-neg" },
};

function NotificationToast({ item }: { item: InAppNotification }) {
  const dismiss = useInAppNotifications((state) => state.dismiss);
  const [hovering, setHovering] = useState(false);
  const [focused, setFocused] = useState(false);
  const [fading, setFading] = useState(false);
  const { Icon, color } = appearance[item.severity];

  useEffect(() => {
    if (hovering || focused) return;
    const timeout = window.setTimeout(() => setFading(true), item.action || item.severity === "error" ? 11000 : 6500);
    return () => window.clearTimeout(timeout);
  }, [hovering, focused, item]);

  useEffect(() => {
    if (!fading) return;
    const timeout = window.setTimeout(() => dismiss(item.id), 220);
    return () => window.clearTimeout(timeout);
  }, [dismiss, fading, item.id]);

  return (
    <div
      className={`pointer-events-auto flex items-start gap-3 rounded-lg border border-line-strong bg-elevated p-3 text-sm text-ink shadow-xl motion-safe:transition-opacity motion-safe:duration-200 ${fading ? "opacity-0" : ""}`}
      onMouseEnter={() => { setHovering(true); setFading(false); }}
      onMouseLeave={() => setHovering(false)}
      onFocus={() => { setFocused(true); setFading(false); }}
      onBlur={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) setFocused(false); }}
    >
      <Icon aria-hidden="true" className={`mt-0.5 h-4 w-4 shrink-0 ${color}`} />
      <div className="min-w-0 flex-1 space-y-2">
        <p role={item.severity === "error" ? "alert" : "status"} className="max-h-40 overflow-auto whitespace-pre-wrap break-words">
          {item.message}
        </p>
        {item.action && (
          <button type="button" className="rounded-md bg-brand-soft px-2.5 py-1 text-xs font-medium text-brand-ink hover:bg-hover"
            onClick={() => { item.action?.run(); dismiss(item.id); }}>
            {item.action.label}
          </button>
        )}
      </div>
      <button type="button" aria-label="Dismiss notification" onClick={() => dismiss(item.id)}
        className="shrink-0 rounded-md p-0.5 text-ink-muted hover:bg-hover hover:text-ink">
        <X aria-hidden="true" className="h-4 w-4" />
      </button>
    </div>
  );
}

export function NotificationViewport() {
  const items = useInAppNotifications((state) => state.items);
  const [host, setHost] = useState<HTMLElement>(document.body);

  useEffect(() => {
    const sync = () => setHost([...document.querySelectorAll<HTMLDialogElement>("dialog[open]")].at(-1) ?? document.body);
    const containsDialog = (node: Node) => node instanceof Element && (node.matches("dialog") || node.querySelector("dialog") !== null);
    const observer = new MutationObserver((records) => {
      if (records.some((record) => record.type === "attributes" ||
        [...record.addedNodes, ...record.removedNodes].some(containsDialog))) sync();
    });
    observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["open"] });
    sync();
    return () => observer.disconnect();
  }, []);

  if (items.length === 0) return null;
  return createPortal(
    <aside aria-label="In-app notifications"
      className={`${host === document.body ? "fixed" : "absolute"} pointer-events-none bottom-4 right-4 z-[100] flex w-[min(24rem,calc(100%-2rem))] flex-col gap-2`}>
      {items.map((item) => <NotificationToast key={item.id} item={item} />)}
    </aside>,
    host,
  );
}
