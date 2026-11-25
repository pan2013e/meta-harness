import { useEffect, useState } from "react";
import { KeyRound, X } from "lucide-react";
import { ChatPane } from "./components/chat/ChatPane";
import { InspectorDrawer } from "./components/drawer/InspectorDrawer";
import { Sidebar } from "./components/sidebar/Sidebar";
import { useAppStore } from "./store/useAppStore";

export function App() {
  const {
    authRequired,
    authenticated,
    bootstrapping,
    errorBanner,
    clearError,
    init,
    authenticateWithToken,
  } = useAppStore();

  const [tokenInput, setTokenInput] = useState("");

  useEffect(() => {
    void init();
  }, [init]);

  if (bootstrapping) {
    return (
      <div className="h-screen w-screen bg-zinc-950 text-zinc-400 flex items-center justify-center text-sm">
        Connecting to Meta-Harness Switch...
      </div>
    );
  }

  if (authRequired && !authenticated) {
    return (
      <div className="h-screen w-screen bg-zinc-950 flex items-center justify-center p-4">
        <div className="w-full max-w-sm rounded-2xl border border-zinc-800 bg-zinc-900 p-6 space-y-4 shadow-2xl">
          <div className="flex items-center gap-2 text-indigo-400">
            <KeyRound className="w-5 h-5" />
            <h1 className="text-base font-semibold text-zinc-100">
              Remote Access Token
            </h1>
          </div>
          <p className="text-xs text-zinc-400">
            Enter the pairing bearer token printed in the Meta-Harness server terminal.
          </p>
          <input
            type="password"
            value={tokenInput}
            onChange={(e) => setTokenInput(e.target.value)}
            placeholder="Paste access token..."
            className="w-full rounded-lg bg-zinc-950 border border-zinc-800 px-3 py-2 text-sm text-zinc-100 font-mono"
          />
          <button
            type="button"
            onClick={() => void authenticateWithToken(tokenInput.trim())}
            className="w-full py-2 rounded-lg bg-indigo-600 hover:bg-indigo-500 text-sm font-medium text-white"
          >
            Connect
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="h-screen w-screen overflow-hidden flex flex-col bg-zinc-950 text-zinc-100">
      {errorBanner && (
        <div className="bg-rose-600/20 border-b border-rose-500/40 px-4 py-1.5 text-xs text-rose-200 flex items-center justify-between">
          <span>{errorBanner}</span>
          <button type="button" onClick={clearError} className="text-rose-300 hover:text-white">
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      )}

      <div className="flex-1 flex min-h-0">
        <Sidebar />
        <ChatPane />
        <InspectorDrawer />
      </div>
    </div>
  );
}
