const { contextBridge, ipcRenderer } = require("electron");

if (process.isMainFrame) {
  contextBridge.exposeInMainWorld("metaHarnessDesktop", {
    installUpdate: (version) => ipcRenderer.invoke("meta-harness:install-update", version),
    textMenu: (kind, selection, canPaste) => ipcRenderer.invoke("meta-harness:text-menu", kind, selection, canPaste),
    windowsTitleBar: process.platform === "win32",
    windowsStore: Boolean(process.windowsStore),
    setTitleBarColors: (color, symbolColor) => ipcRenderer.invoke("meta-harness:title-bar-colors", color, symbolColor),
    selectProjectFolder: () => ipcRenderer.invoke("meta-harness:project-folder"),
    notify: (body) => ipcRenderer.invoke("meta-harness:notify", body),
    saveProxy: (value) => ipcRenderer.invoke("meta-harness:save-proxy", value),
  });
}
