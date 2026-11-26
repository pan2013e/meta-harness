import { useEffect, useState } from "react";
import { KeyRound, Loader2, X } from "lucide-react";
import { ChatPane } from "./components/chat/ChatPane";
import { DesktopTitleBar } from "./components/common/DesktopTitleBar";
import { DiffReviewModal } from "./components/modals/DiffReviewModal";
import { FilePreviewModal } from "./components/modals/FilePreviewModal";
import { FolderPickerModal } from "./components/modals/FolderPickerModal";
import { MemoryModal } from "./components/modals/MemoryModal";
import { SettingsDialog } from "./components/modals/SettingsDialog";
import { Sidebar } from "./components/sidebar/Sidebar";
import { useAppStore, type StartupTask } from "./store/useAppStore";

const STARTUP_LABELS: Record<StartupTask, string> = {
  workspace: "Opening workspace",
  models: "Loading model choices",
  quota: "Checking subscription usage",
};

export function App() {
  return (
    <div className="flex h-dvh w-full flex-col overflow-hidden bg-app text-ink">
      <DesktopTitleBar />
      <AppContent />
    </div>
  );
}

function AppContent() {
  const {
    authRequired,
    authenticated,
    bootstrapping,
    startupPhase,
    startupTasks,
    errorBanner,
    folderModalOpen,
    settingsOpen,
    memoryModalOpen,
    diffModalOpen,
    filePreview,
    setSettingsOpen,
    setFolderModalOpen,
    setMemoryModalOpen,
    setDiffModalOpen,
    setFilePreview,
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
      <main className="grid min-h-0 flex-1 place-items-center bg-app px-4">
        <div role="status" className="min-w-60 rounded-2xl border border-line bg-panel px-7 py-6 text-center shadow-sm">
          <img src="/meta-harness.svg" alt="" className="mx-auto mb-3 h-11 w-11" />
          <h1 className="font-semibold text-ink">Meta-Harness</h1>
          <p className="mt-2 flex items-center justify-center gap-2 text-xs text-ink-muted">
            <Loader2 aria-hidden="true" className="h-3.5 w-3.5 animate-spin text-brand-ink motion-reduce:animate-none" />
            {startupPhase === "connecting" ? "Connecting to the server" : "Checking harnesses and projects"}
          </p>
        </div>
      </main>
    );
  }

  if (authRequired && !authenticated) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center bg-app p-4">
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
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
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

      {startupTasks.length > 0 && (
        <div role="status" className="flex shrink-0 items-center gap-2 border-b border-line bg-panel px-4 py-1.5 text-xs text-ink-muted">
          <Loader2 aria-hidden="true" className="h-3.5 w-3.5 shrink-0 animate-spin text-brand-ink motion-reduce:animate-none" />
          <span>{startupTasks.map((task) => STARTUP_LABELS[task]).join(" · ")}</span>
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
      {filePreview && (
        <FilePreviewModal key={JSON.stringify(filePreview)} {...filePreview} onClose={() => setFilePreview(null)} />
      )}
    </div>
  );
}
