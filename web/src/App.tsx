import { useEffect, useState } from "react";
import { KeyRound, Loader2, X } from "lucide-react";
import { ChatPane } from "./components/chat/ChatPane";
import { DiffReviewModal } from "./components/modals/DiffReviewModal";
import { FolderPickerModal } from "./components/modals/FolderPickerModal";
import { MemoryModal } from "./components/modals/MemoryModal";
import { SettingsDialog } from "./components/modals/SettingsDialog";
import { Sidebar } from "./components/sidebar/Sidebar";
import { useAppStore } from "./store/useAppStore";

export function App() {
  const {
    authRequired,
    authenticated,
    bootstrapping,
    errorBanner,
    folderModalOpen,
    settingsOpen,
    memoryModalOpen,
    diffModalOpen,
    setSettingsOpen,
    setFolderModalOpen,
    setMemoryModalOpen,
    setDiffModalOpen,
    createAndSelectProject,
    toggleSidebar,
    clearError,
    init,
    authenticateWithToken,
  } = useAppStore();

  const [tokenInput, setTokenInput] = useState("");

  useEffect(() => {
    void init();
  }, [init]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "b") {
        e.preventDefault();
        toggleSidebar();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [toggleSidebar]);

  if (bootstrapping) {
    return (
      <div className="h-screen w-screen bg-app text-ink-muted flex items-center justify-center gap-2.5 text-sm">
        <Loader2 className="w-4 h-4 animate-spin text-brand-ink" />
        <span>Connecting to Meta-Harness Switch...</span>
      </div>
    );
  }

  if (authRequired && !authenticated) {
    return (
      <div className="h-screen w-screen bg-app flex items-center justify-center p-4">
        <div className="w-full max-w-sm rounded-2xl border border-line-strong bg-panel p-6 space-y-4 shadow-2xl">
          <div className="flex items-center gap-2 text-brand-ink">
            <KeyRound className="w-5 h-5" />
            <h1 className="text-base font-semibold text-ink">Remote Access Token</h1>
          </div>
          <p className="text-xs text-ink-muted leading-relaxed">
            Enter the pairing bearer token printed in the Meta-Harness server terminal.
          </p>
          {errorBanner && (
            <p role="alert" className="text-xs text-neg">
              {errorBanner}
            </p>
          )}
          <input
            type="password"
            value={tokenInput}
            onChange={(e) => setTokenInput(e.target.value)}
            placeholder="Paste access token..."
            className="w-full rounded-xl bg-inset border border-line focus:border-brand px-3 py-2 text-sm text-ink font-mono focus:outline-none"
          />
          <button
            type="button"
            onClick={() => void authenticateWithToken(tokenInput.trim())}
            className="w-full py-2.5 rounded-xl bg-brand hover:bg-brand-hover text-sm font-medium text-white transition-colors"
          >
            Connect
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-dvh w-full flex-col overflow-hidden bg-app text-ink">
      {errorBanner && (
        <div className="bg-neg-soft border-b border-neg/35 px-4 py-1.5 text-xs text-neg flex items-center justify-between">
          <span>{errorBanner}</span>
          <button
            type="button"
            onClick={clearError}
            className="text-neg hover:opacity-75 p-0.5"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      )}

      <div className="flex-1 flex min-h-0">
        <Sidebar />
        <ChatPane />
      </div>

      {folderModalOpen && (
        <FolderPickerModal
          onClose={() => setFolderModalOpen(false)}
          onSelect={async (path) => {
            await createAndSelectProject(path);
          }}
        />
      )}
      {settingsOpen && <SettingsDialog onClose={() => setSettingsOpen(false)} />}
      {memoryModalOpen && <MemoryModal onClose={() => setMemoryModalOpen(false)} />}
      {diffModalOpen && <DiffReviewModal onClose={() => setDiffModalOpen(false)} />}
    </div>
  );
}
