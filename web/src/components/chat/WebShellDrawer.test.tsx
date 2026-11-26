import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { WebShellDrawer } from "./WebShellDrawer";

const terminalClipboard = vi.hoisted(() => ({
  getSelection: vi.fn(() => "selected terminal output"),
  paste: vi.fn(),
}));

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    parser = { registerOscHandler: vi.fn() };
    getSelection = terminalClipboard.getSelection;
    paste = terminalClipboard.paste;
    loadAddon = vi.fn();
    open = vi.fn();
    focus = vi.fn();
    write = vi.fn();
    dispose = vi.fn();
    onData = () => ({ dispose: vi.fn() });

    constructor(public options: { disableStdin: boolean }) {}
  },
}));

vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

it("uses the terminal selection and xterm's paste handling for the desktop menu", async () => {
  let socket: { onopen?: () => void } | undefined;
  vi.stubGlobal("WebSocket", class {
    static OPEN = 1;
    readyState = 1;
    onopen?: () => void;
    send = vi.fn();
    close = vi.fn();

    constructor() { socket = this; }
  });
  const textMenu = vi.fn().mockResolvedValue({ type: "paste", text: "pasted terminal text" });
  vi.stubGlobal("metaHarnessDesktop", { textMenu });

  render(<WebShellDrawer sessionId="s_terminal" shared={false} visible defaultCwd="/test" />);
  const pane = screen.getByTestId("xterm-pane-tab_1");
  act(() => socket?.onopen?.());
  fireEvent.contextMenu(pane);

  await waitFor(() => expect(terminalClipboard.paste).toHaveBeenCalledWith("pasted terminal text"));
  expect(textMenu).toHaveBeenCalledWith("terminal", "selected terminal output", true);
});
