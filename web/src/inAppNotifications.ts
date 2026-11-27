import { create } from "zustand";

type Severity = "info" | "success" | "warning" | "error";

export interface InAppNotification {
  id: number;
  message: string;
  severity: Severity;
  action?: { label: string; run: () => void };
}

let nextId = 0;
export const useInAppNotifications = create<{
  items: InAppNotification[];
  dismiss: (id: number) => void;
}>((set) => ({
  items: [],
  dismiss: (id) => set((state) => ({ items: state.items.filter((item) => item.id !== id) })),
}));

export function notifyInApp(
  message: string,
  severity: Severity = "info",
  action?: InAppNotification["action"],
) {
  useInAppNotifications.setState((state) => {
    if (state.items.some((item) => item.message === message && item.severity === severity)) return state;
    const item = { id: ++nextId, message, severity, action };
    return { items: [...state.items.slice(-3), item] };
  });
}

export function reportInAppError(error: unknown, fallback?: string) {
  notifyInApp(error instanceof Error ? error.message : fallback ?? String(error), "error");
}
