import { useEffect, useMemo, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal as XTerm } from "@xterm/xterm";
import { Plus, Terminal, Trash2, X } from "lucide-react";
import { ptyWsUrl } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { ThemeId } from "../../types/api";

interface TerminalTab {
  id: string;
  label: string;
  target: string | null;
  status: "connecting" | "open" | "closed";
}

function getXtermTheme(theme: ThemeId) {
  if (theme === "daylight") {
    return {
      background: "#f8fafc",
      foreground: "#0f172a",
      cursor: "#4338ca",
      selectionBackground: "rgba(79, 70, 229, 0.22)",
    };
  }
  if (theme === "tokyo-night") {
    return {
      background: "#12131c",
      foreground: "#c8d3f5",
      cursor: "#7aa2f7",
      selectionBackground: "rgba(122, 162, 247, 0.26)",
    };
  }
  if (theme === "catppuccin") {
    return {
      background: "#11111b",
      foreground: "#cdd6f4",
      cursor: "#cba6f7",
      selectionBackground: "rgba(203, 166, 247, 0.24)",
    };
  }
  return {
    background: "#06080c",
    foreground: "#e2e8f0",
    cursor: "#818cf8",
    selectionBackground: "rgba(99, 102, 241, 0.28)",
  };
}

function TerminalTabPane({
  sessionId,
  tab,
  active,
  theme,
  heightPx,
  clearSeq,
  onStatusChange,
}: {
  sessionId: string;
  tab: TerminalTab;
  active: boolean;
  theme: ThemeId;
  heightPx: number;
  clearSeq: number;
  onStatusChange: (tabId: string, status: TerminalTab["status"]) => void;
}) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<XTerm | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const [fallbackMsg, setFallbackMsg] = useState<string | null>(null);

  const xtermTheme = useMemo(() => getXtermTheme(theme), [theme]);

  useEffect(() => {
    if (termRef.current) {
      termRef.current.options.theme = xtermTheme;
    }
  }, [xtermTheme]);

  useEffect(() => {
    if (clearSeq > 0) {
      termRef.current?.clear();
      termRef.current?.focus();
    }
  }, [clearSeq]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host || typeof window === "undefined") return;

    let disposed = false;
    let term: XTerm | null = null;
    let fitAddon: FitAddon | null = null;
    let ws: WebSocket | null = null;
    let observer: ResizeObserver | null = null;

    try {
      term = new XTerm({
        cursorBlink: true,
        fontSize: 12.5,
        lineHeight: 1.32,
        scrollback: 5000,
        convertEol: false,
        fontFamily:
          '"JetBrains Mono Variable", "Cascadia Mono", "Sarasa Mono SC", "Microsoft YaHei UI", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
        theme: xtermTheme,
      });
      fitAddon = new FitAddon();
      term.loadAddon(fitAddon);
      term.open(host);
      try {
        fitAddon.fit();
      } catch {
        // ignore layout measurements in minimal headless test DOM
      }
      termRef.current = term;
      fitRef.current = fitAddon;

      const cols = Math.max(20, term.cols || 80);
      const rows = Math.max(6, term.rows || 24);
      const pendingInput: string[] = [];

      if (typeof WebSocket !== "undefined") {
        ws = new WebSocket(ptyWsUrl(sessionId, cols, rows, tab.target));
        wsRef.current = ws;

        ws.onopen = () => {
          if (disposed) return;
          onStatusChange(tab.id, "open");
          while (pendingInput.length > 0 && ws?.readyState === WebSocket.OPEN) {
            const chunk = pendingInput.shift();
            if (chunk) {
              ws.send(JSON.stringify({ type: "input", data: chunk }));
            }
          }
          if (fitAddon && term) {
            try {
              fitAddon.fit();
              ws?.send(
                JSON.stringify({
                  type: "resize",
                  cols: Math.max(20, term.cols || 80),
                  rows: Math.max(6, term.rows || 24),
                })
              );
            } catch {
              // ignore
            }
          }
        };

        ws.onmessage = (ev) => {
          if (disposed || !term) return;
          try {
            const frame = JSON.parse(String(ev.data)) as {
              type: string;
              data?: string;
            };
            if (frame.type === "output" && typeof frame.data === "string") {
              term.write(frame.data);
            } else if (frame.type === "exit") {
              onStatusChange(tab.id, "closed");
              term.writeln("\r\n\x1b[90m[shell process exited]\x1b[0m");
            }
          } catch {
            // ignore malformed frame
          }
        };

        ws.onclose = () => {
          if (!disposed) onStatusChange(tab.id, "closed");
        };
      }

      const dataSub = term.onData((data) => {
        if (ws?.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: "input", data }));
        } else {
          pendingInput.push(data);
        }
      });

      const sendResize = () => {
        if (!fitAddon || !term || disposed) return;
        try {
          fitAddon.fit();
          if (ws?.readyState === WebSocket.OPEN) {
            ws.send(
              JSON.stringify({
                type: "resize",
                cols: Math.max(20, term.cols || 80),
                rows: Math.max(6, term.rows || 24),
              })
            );
          }
        } catch {
          // ignore
        }
      };

      if (typeof ResizeObserver !== "undefined") {
        observer = new ResizeObserver(() => sendResize());
        observer.observe(host);
      }

      return () => {
        disposed = true;
        observer?.disconnect();
        dataSub.dispose();
        ws?.close();
        term?.dispose();
        termRef.current = null;
        fitRef.current = null;
        wsRef.current = null;
      };
    } catch {
      setFallbackMsg("Interactive WebShell requires canvas support in a live browser.");
      return undefined;
    }
  }, [sessionId, tab.id, tab.target]);

  useEffect(() => {
    if (!active) return;
    const raf = window.requestAnimationFrame(() => {
      try {
        fitRef.current?.fit();
        if (
          wsRef.current?.readyState === WebSocket.OPEN &&
          termRef.current
        ) {
          wsRef.current.send(
            JSON.stringify({
              type: "resize",
              cols: Math.max(20, termRef.current.cols || 80),
              rows: Math.max(6, termRef.current.rows || 24),
            })
          );
        }
        termRef.current?.focus();
      } catch {
        // ignore
      }
    });
    return () => window.cancelAnimationFrame(raf);
  }, [active, heightPx]);

  if (fallbackMsg) {
    return (
      <div
        className={`${
          active ? "flex" : "hidden"
        } h-full items-center justify-center px-4 text-xs font-mono text-ink-muted`}
      >
        {fallbackMsg}
      </div>
    );
  }

  return (
    <div
      ref={hostRef}
      data-testid={`xterm-pane-${tab.id}`}
      className={`${active ? "block" : "hidden"} h-full w-full px-2 py-1.5`}
    />
  );
}

export function WebShellDrawer({
  sessionId,
  defaultCwd,
}: {
  sessionId: string;
  defaultCwd: string;
}) {
  const { members, probes, theme, setShellOpen } = useAppStore();
  const [heightPx, setHeightPx] = useState(260);
  const [newMenuOpen, setNewMenuOpen] = useState(false);
  const [clearCounter, setClearCounter] = useState<Record<string, number>>({});
  const seqRef = useRef(1);
  const [tabs, setTabs] = useState<TerminalTab[]>([
    { id: "tab_1", label: "integration", target: null, status: "connecting" },
  ]);
  const [activeTabId, setActiveTabId] = useState("tab_1");

  const installedIds = useMemo(
    () => new Set(probes.filter((p) => p.installed).map((p) => p.adapter_id)),
    [probes]
  );
  const worktreeMembers = useMemo(
    () => members.filter((m) => installedIds.has(m.adapter_id)),
    [members, installedIds]
  );

  useEffect(() => {
    const id = `tab_${++seqRef.current}`;
    setTabs([{ id, label: "integration", target: null, status: "connecting" }]);
    setActiveTabId(id);
  }, [sessionId]);

  const addTab = (target: string | null, label: string) => {
    const id = `tab_${++seqRef.current}`;
    setTabs((prev) => [...prev, { id, label, target, status: "connecting" }]);
    setActiveTabId(id);
    setNewMenuOpen(false);
  };

  const closeTab = (id: string) => {
    setTabs((prev) => {
      const next = prev.filter((t) => t.id !== id);
      if (next.length === 0) {
        setShellOpen(false);
        return prev;
      }
      if (activeTabId === id) {
        setActiveTabId(next[next.length - 1].id);
      }
      return next;
    });
  };

  const startResize = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    const startY = e.clientY;
    const startH = heightPx;
    const onMove = (ev: PointerEvent) => {
      const delta = startY - ev.clientY;
      setHeightPx(Math.max(180, Math.min(560, startH + delta)));
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  const activeTab = tabs.find((t) => t.id === activeTabId) ?? tabs[0];

  return (
    <section
      aria-label="In-web session shell"
      style={{ height: `${heightPx}px` }}
      className="relative border-t border-line bg-inset flex flex-col shrink-0"
    >
      {/* Top Drag Resize Handle */}
      <div
        role="separator"
        aria-orientation="horizontal"
        title="Drag to resize terminal"
        onPointerDown={startResize}
        className="h-1.5 w-full cursor-row-resize bg-transparent hover:bg-brand/40 transition-colors shrink-0"
      />

      <header className="px-3 py-1.5 border-b border-line bg-elevated/70 flex items-center justify-between gap-2 shrink-0">
        <div className="flex items-center gap-1.5 min-w-0 overflow-x-auto">
          <div className="flex items-center gap-1.5 px-1.5 text-xs font-semibold text-ink shrink-0">
            <Terminal className="w-3.5 h-3.5 text-brand-ink" />
            <span>Terminal</span>
          </div>

          <div className="flex items-center gap-1">
            {tabs.map((tab) => {
              const isActive = tab.id === activeTab?.id;
              return (
                <div
                  key={tab.id}
                  className={`inline-flex items-center gap-1 rounded-lg border px-2 py-0.5 text-[11px] font-mono transition-colors ${
                    isActive
                      ? "bg-brand-soft border-brand/35 text-brand-ink font-medium"
                      : "bg-panel border-line text-ink-muted hover:text-ink"
                  }`}
                >
                  <button
                    type="button"
                    onClick={() => setActiveTabId(tab.id)}
                    className="inline-flex items-center gap-1.5"
                  >
                    <span
                      className={`w-1.5 h-1.5 rounded-full ${
                        tab.status === "open"
                          ? "bg-pos"
                          : tab.status === "closed"
                            ? "bg-ink-muted"
                            : "bg-warn"
                      }`}
                    />
                    <span>{tab.label}</span>
                  </button>
                  <button
                    type="button"
                    aria-label={`Close terminal ${tab.label}`}
                    onClick={() => closeTab(tab.id)}
                    className="ml-0.5 opacity-65 hover:opacity-100 hover:text-neg"
                  >
                    <X className="w-2.5 h-2.5" />
                  </button>
                </div>
              );
            })}
          </div>

          {/* New Terminal Target Menu */}
          <div className="relative shrink-0">
            <button
              type="button"
              aria-label="New terminal tab"
              title="Open another worktree terminal"
              onClick={() => setNewMenuOpen((v) => !v)}
              className="p-1 rounded-md text-ink-muted hover:text-ink hover:bg-hover transition-colors"
            >
              <Plus className="w-3.5 h-3.5" />
            </button>
            {newMenuOpen && (
              <>
                <div
                  className="fixed inset-0 z-20"
                  onClick={() => setNewMenuOpen(false)}
                />
                <div className="absolute left-0 bottom-full mb-1.5 w-52 rounded-xl border border-line-strong bg-panel shadow-2xl p-1 z-30 space-y-0.5 text-xs">
                  <div className="px-2.5 py-1 text-[10px] uppercase tracking-wider font-semibold text-ink-muted">
                    Launch terminal in
                  </div>
                  <button
                    type="button"
                    onClick={() => addTab(null, "integration")}
                    className="w-full px-2.5 py-1.5 rounded-lg text-left hover:bg-hover text-ink font-mono text-[11px]"
                  >
                    integration
                  </button>
                  {worktreeMembers.map((m) => (
                    <button
                      key={m.handle}
                      type="button"
                      onClick={() => addTab(m.handle, `@${m.handle}`)}
                      className="w-full px-2.5 py-1.5 rounded-lg text-left hover:bg-hover text-ink-soft font-mono text-[11px]"
                    >
                      @{m.handle}
                    </button>
                  ))}
                </div>
              </>
            )}
          </div>
        </div>

        <div className="flex items-center gap-1.5 shrink-0">
          <span
            className="hidden lg:inline text-[10.5px] font-mono text-ink-muted truncate max-w-[260px]"
            title={defaultCwd}
          >
            {defaultCwd}
          </span>
          {activeTab && (
            <button
              type="button"
              title="Clear active terminal"
              onClick={() =>
                setClearCounter((prev) => ({
                  ...prev,
                  [activeTab.id]: (prev[activeTab.id] ?? 0) + 1,
                }))
              }
              className="p-1 rounded-md text-ink-muted hover:text-ink hover:bg-hover transition-colors"
            >
              <Trash2 className="w-3.5 h-3.5" />
            </button>
          )}
          <button
            type="button"
            aria-label="Close web shell"
            onClick={() => setShellOpen(false)}
            className="p-1 rounded-md text-ink-muted hover:text-ink hover:bg-hover transition-colors"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      </header>

      <div className="flex-1 min-h-0 relative">
        {tabs.map((tab) => (
          <TerminalTabPane
            key={tab.id}
            sessionId={sessionId}
            tab={tab}
            active={tab.id === activeTab?.id}
            theme={theme}
            heightPx={heightPx}
            clearSeq={clearCounter[tab.id] ?? 0}
            onStatusChange={(tabId, status) =>
              setTabs((prev) =>
                prev.map((t) => (t.id === tabId ? { ...t, status } : t))
              )
            }
          />
        ))}
      </div>
    </section>
  );
}
