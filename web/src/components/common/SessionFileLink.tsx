import type { ReactNode } from "react";
import { sessionFileUrl } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";

export function SessionFileLink({ sessionId, path, title, className, children }: {
  sessionId: string;
  path: string;
  title?: string;
  className?: string;
  children: ReactNode;
}) {
  return (
    <a
      href={sessionFileUrl(sessionId, path)}
      title={title ?? path}
      className={className}
      onClick={(event) => {
        if (event.altKey || event.metaKey || event.ctrlKey || event.shiftKey) return;
        event.preventDefault();
        useAppStore.getState().setFilePreview({ sessionId, path });
      }}
    >
      {children}
    </a>
  );
}
