import { useEffect, useRef, useState } from "react";
import { ArrowLeft, Loader2 } from "lucide-react";
import { reportInAppError } from "../../inAppNotifications";

export function LicenseNotices({ onBack }: { onBack: () => void }) {
  const title = useRef<HTMLHeadingElement>(null);
  const [text, setText] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    title.current?.focus();
    const abort = new AbortController();
    void fetch("/third-party-licenses.txt", { signal: abort.signal }).then(async (response) => {
      if (!response.ok || !response.headers.get("content-type")?.startsWith("text/plain")) {
        throw new Error("Third-party license notices are unavailable in this build.");
      }
      const notices = await response.text();
      if (!abort.signal.aborted) setText(notices);
    }).catch((error: unknown) => {
      if (abort.signal.aborted) return;
      setFailed(true);
      reportInAppError(error);
    });
    return () => abort.abort();
  }, []);

  return (
    <div className="space-y-3">
      <button type="button" className="settings-button" onClick={onBack}>
        <ArrowLeft aria-hidden="true" size={14} /> Back to About
      </button>
      <h3 tabIndex={-1} ref={title} className="settings-heading">Third-party license notices</h3>
      {text !== null ? (
        <pre tabIndex={0} aria-label="License text"
          className="max-h-[38dvh] overflow-auto whitespace-pre-wrap break-words rounded-xl border border-line bg-inset p-4 font-mono text-xs leading-relaxed text-ink-soft">
          {text}
        </pre>
      ) : failed ? (
        <p className="text-sm text-ink-muted">The license viewer is unavailable.</p>
      ) : (
        <p role="status" className="flex items-center gap-2 text-sm text-ink-muted">
          <Loader2 aria-hidden="true" size={14} className="motion-safe:animate-spin" /> Loading notices…
        </p>
      )}
    </div>
  );
}
