const { contextBridge, ipcRenderer } = require("electron");

if (process.isMainFrame) {
  contextBridge.exposeInMainWorld("metaHarnessDesktop", {
    installUpdate: (version) => ipcRenderer.invoke("meta-harness:install-update", version),
    textMenu: (kind, selection, canPaste) => ipcRenderer.invoke("meta-harness:text-menu", kind, selection, canPaste),
    windowsTitleBar: process.platform === "win32",
    setTitleBarColors: (color, symbolColor) => ipcRenderer.invoke("meta-harness:title-bar-colors", color, symbolColor),
  });
}
