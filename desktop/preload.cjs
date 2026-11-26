const { contextBridge, ipcRenderer } = require("electron");

if (process.isMainFrame) {
  contextBridge.exposeInMainWorld("metaHarnessDesktop", {
    installUpdate: (version) => ipcRenderer.invoke("meta-harness:install-update", version),
  });
}
