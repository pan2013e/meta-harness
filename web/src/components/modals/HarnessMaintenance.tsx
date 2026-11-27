import { notifyInApp, reportInAppError } from "../../inAppNotifications";
import { useEffect, useState } from "react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { HarnessCliVersion, HarnessOperation } from "../../types/api";

const actionClass = "settings-button";

function signInLinks(output: string): string[] {
  const trustedDomains = ["openai.com", "chatgpt.com", "google.com", "antigravity.google", "claude.com", "claude.ai", "anthropic.com"];
  return [...new Set(output.match(/https:\/\/[^\s<>"']+/g) ?? [])].flatMap((link) => {
    try {
      const url = new URL(link.replace(/[),;]+$/, ""));
      if (trustedDomains.some((domain) => url.hostname === domain || url.hostname.endsWith("."+domain))) {
        return [url.toString()];
      }
    } catch {
      // Partial terminal output may contain an unfinished URL.
    }
    return [];
  });
}

export function HarnessMaintenance({ adapterId, profile, hasSavedProfile, offersSignIn, offersDeviceCode }: {
  adapterId: string;
  profile?: string | null;
  hasSavedProfile: boolean;
  offersSignIn: boolean;
  offersDeviceCode: boolean;
}) {
  const [version, setVersion] = useState<HarnessCliVersion | null>(null);
  const [operation, setOperation] = useState<HarnessOperation | null>(null);
  const [working, setWorking] = useState(false);
  const [code, setCode] = useState("");
  const selectProfile = useAppStore((state) => state.selectHarnessProfile);
  const running = operation?.status === "running";

  useEffect(() => {
    let active = true;
    void api.harnessOperation(adapterId).then(({ operation }) => {
      if (active) setOperation(operation);
    }).catch(() => undefined);
    return () => { active = false; };
  }, [adapterId]);

  useEffect(() => {
    if (!running) return;
    let active = true;
    let pending = false;
    const timer = window.setInterval(async () => {
      if (pending) return;
      pending = true;
      try {
        const { operation: next } = await api.harnessOperation(adapterId);
        if (!active) return;
        setOperation(next);
        if (next?.status === "failed") {
          notifyInApp(next.kind === "sign_in"
            ? `@${adapterId} could not sign in. Check the profile status in Settings.`
            : `@${adapterId} could not update its CLI.`, "error");
        }
        if (next?.status === "completed") {
          notifyInApp(next.kind === "sign_in" ? `@${adapterId} profile signed in.` : `@${adapterId} CLI updated.`, "success");
          setCode("");
          setVersion(null);
          void useAppStore.getState().refreshQuotasLive();
        }
      } catch (cause) {
        if (active) reportInAppError(cause);
      } finally {
        pending = false;
      }
    }, 900);
    return () => { active = false; window.clearInterval(timer); };
  }, [adapterId, running]);

  const run = async (action: () => Promise<void>) => {
    setWorking(true);
    try {
      await action();
    } catch (cause) {
      reportInAppError(cause);
    } finally {
      setWorking(false);
    }
  };

  const start = (kind: HarnessOperation["kind"], loginMode?: "device_code") => run(async () => {
    const { operation } = await api.startHarnessOperation(adapterId, kind, loginMode);
    setCode("");
    setOperation(operation);
  });

  const output = (operation?.output ?? "").replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "");
  const authorization = operation?.authorization_url;
  const links = operation?.kind === "sign_in" && running
    ? authorization
      ? operation.accepts_input ? signInLinks(authorization) : []
      : signInLinks(output.slice(0, output.lastIndexOf("\n") + 1))
    : [];

  return (
    <div className="space-y-2 border-t border-line pt-2 text-xs">
      <div className="flex flex-wrap items-center gap-2">
        {version && (
          <span className="text-ink-muted">
            Installed {version.installed ?? "unknown"}{version.published && ` · Published ${version.published}`}
          </span>
        )}
        <button type="button" className={actionClass} disabled={working || running}
          onClick={() => void run(async () => { setVersion((await api.checkHarnessVersion(adapterId)).version); })}>
          Check CLI version
        </button>
        {version?.can_upgrade && (
          <button type="button" className={actionClass} disabled={working || running} onClick={() => void start("upgrade")}>
            Upgrade CLI
          </button>
        )}
        {(version?.can_sign_in ?? offersSignIn) && (
          <button type="button" className={actionClass} disabled={working || running} onClick={() => void start("sign_in")}>
            {profile || hasSavedProfile ? "Sign in to separate profile" : "Create profile & sign in"}
          </button>
        )}
        {hasSavedProfile && (
          <button type="button" className={actionClass} disabled={working || running}
            onClick={() => void run(async () => {
              await selectProfile(adapterId, profile ? "default" : "separate");
              setOperation(null);
            })}>
            {profile ? "Use default CLI profile" : "Use saved separate profile"}
          </button>
        )}
        {offersDeviceCode && (version?.can_sign_in ?? offersSignIn) && (
          <button type="button" className={actionClass} disabled={working || running}
            onClick={() => void start("sign_in", "device_code")}>
            Use device code
          </button>
        )}
      </div>
      {operation && (
        <div className="space-y-2 rounded-lg border border-line bg-inset px-2.5 py-2">
          <div className="flex items-center justify-between gap-2">
            <span role="status" className={operation.status === "failed" ? "text-neg" : "text-ink"}>
              {operation.kind === "sign_in" ? "Profile sign-in" : "CLI upgrade"} · {operation.status}
            </span>
            {running && <button type="button" className={actionClass} disabled={working}
              onClick={() => void run(async () => { await api.cancelHarnessOperation(adapterId); })}>Cancel</button>}
          </div>
          {links.map((link, index) => <a className="block text-brand-ink underline" key={link} href={link} target="_blank" rel="noreferrer noopener">
            {index === 0 ? "Open provider sign-in" : "Open alternate sign-in link"}
          </a>)}
          {authorization ? (
            <p className="text-ink-soft">{operation.accepts_input
              ? "Finish Google sign-in, then paste the authorization code or the full Antigravity callback URL."
              : "Verifying sign-in…"}</p>
          ) : output.trim() && <pre className="max-h-36 overflow-auto whitespace-pre-wrap break-all text-[11px] text-ink-soft">{output}</pre>}
          {running && operation.accepts_input && (
            <div className="flex gap-2">
              <input
                aria-label={authorization ? "Authorization code or callback URL" : "Provider authorization code"}
                type="password"
                autoComplete="off"
                value={code}
                onChange={(event) => setCode(event.target.value)}
                placeholder={authorization ? "Paste code or callback URL" : "Authorization code, if requested"}
                className="min-w-0 flex-1 rounded-lg border border-line bg-panel px-2 py-1.5"
              />
              <button type="button" className={actionClass} disabled={working || !code.trim()}
                onClick={() => void run(async () => {
                  await api.sendHarnessSignInInput(adapterId, code);
                  setCode("");
                  if (authorization) setOperation((current) => current ? { ...current, accepts_input: false } : current);
                })}>
                Submit code
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
