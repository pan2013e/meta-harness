type TextMenuAction = { type: "quote" } | { type: "paste"; text: string } | { type: "paste_image"; data: string } | { type: "paste_error"; message: string } | null;

declare global {
  interface Window {
    metaHarnessDesktop?: {
      installUpdate: (version: string) => Promise<void>;
      textMenu: (kind: "message" | "composer" | "terminal", selection: string, canPaste: boolean) => Promise<TextMenuAction>;
      windowsTitleBar: boolean;
      setTitleBarColors: (color: string, symbolColor: string) => Promise<void>;
    };
  }
}

export function selectionRangeWithin(root: HTMLElement): Range | null {
  const selection = root.ownerDocument.getSelection();
  if (!selection?.rangeCount) return null;
  const range = selection.getRangeAt(0);
  return root.contains(range.startContainer) && root.contains(range.endContainer) ? range : null;
}
