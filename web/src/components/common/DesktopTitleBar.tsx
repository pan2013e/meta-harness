import { useEffect } from "react";
import { useAppStore } from "../../store/useAppStore";

export function DesktopTitleBar() {
  const theme = useAppStore((state) => state.theme);
  const desktop = window.metaHarnessDesktop;

  useEffect(() => {
    if (!desktop?.windowsTitleBar) return;
    const frame = requestAnimationFrame(() => {
      const colors = getComputedStyle(document.documentElement);
      void desktop.setTitleBarColors(
        colors.getPropertyValue("--mh-panel").trim(),
        colors.getPropertyValue("--mh-ink").trim(),
      ).catch(() => undefined);
    });
    return () => cancelAnimationFrame(frame);
  }, [desktop, theme]);

  if (!desktop?.windowsTitleBar) return null;
  return (
    <header aria-label="Window title bar" className="desktop-title-bar h-9 shrink-0 select-none border-b border-line bg-panel">
      <div className="flex h-full items-center gap-2 px-3" style={{ maxWidth: "env(titlebar-area-width, 100%)" }}>
        <img src="/meta-harness.svg" className="h-[18px] w-[18px] shrink-0" alt="" draggable={false} />
        <span className="truncate text-xs font-semibold text-ink">Meta-Harness</span>
      </div>
    </header>
  );
}
