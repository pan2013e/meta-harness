const { app, BrowserWindow, Menu, Tray, dialog, ipcMain, nativeImage, shell } = require("electron");
const { execFile, spawn } = require("node:child_process");
const { randomBytes } = require("node:crypto");
const { homedir } = require("node:os");
const { delimiter, join } = require("node:path");
const { createInterface } = require("node:readline");
const { promisify } = require("node:util");

let window;
let tray;
let backend;
let backendUrl;
let quitting = false;
let killTimer;
let startupTimer;
let preparingUpdate = false;
let updateDownloader;
let updateInstaller;

const windows = process.platform === "win32";
const resources = app.isPackaged ? process.resourcesPath : __dirname;
const nativeBinary = join(resources, app.isPackaged ? "bin" : "staging", windows ? "meta-harness.exe" : "meta-harness");
const icon = nativeImage.createFromPath(app.isPackaged ? join(resources, "icon.png") : join(__dirname, "..", "assets", "icon.png"));

function openWindow() {
  if (!window) return;
  window.show();
  window.focus();
}

function showStartupError(error) {
  if (quitting) return;
  dialog.showErrorBox("Meta-Harness could not start", error.message);
  app.quit();
}

function openExternalHttp(url, localOrigin) {
  const target = new URL(url);
  if (["https:", "http:"].includes(target.protocol) && target.origin !== localOrigin) {
    void shell.openExternal(target.href).catch(() => undefined);
  }
}

function createWindow(url) {
  const origin = new URL(url).origin;
  window = new BrowserWindow({
    width: 1260,
    height: 840,
    minWidth: 420,
    minHeight: 540,
    icon,
    show: false,
    autoHideMenuBar: true,
    title: "Meta-Harness",
    webPreferences: {
      preload: join(__dirname, "preload.cjs"),
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false,
      webviewTag: false,
    },
  });
  window.webContents.setWindowOpenHandler(({ url }) => {
    openExternalHttp(url, origin);
    return { action: "deny" };
  });
  window.webContents.on("will-navigate", (event, target) => {
    if (new URL(target).origin !== origin) {
      event.preventDefault();
      openExternalHttp(target, origin);
    }
  });
  window.webContents.session.setPermissionRequestHandler((_contents, _permission, reply) => reply(false));
  window.once("ready-to-show", openWindow);
  window.on("close", (event) => {
    if (!quitting) {
      event.preventDefault();
      window.hide();
    }
  });
  void window.loadURL(url).catch(showStartupError);
}

async function prepareDesktopUpdate(event, version) {
  const frame = event.senderFrame;
  if (!app.isPackaged || !window || event.sender !== window.webContents ||
      !frame || frame.parent || !backendUrl || new URL(frame.url).origin !== new URL(backendUrl).origin) {
    throw new Error("Updates must be started from the installed Meta-Harness app.");
  }
  if (typeof version !== "string" || !/^\d+\.\d+\.\d+$/.test(version)) {
    throw new Error("Invalid release version.");
  }
  if (preparingUpdate || quitting) throw new Error("Meta-Harness is already preparing to close.");
  preparingUpdate = true;
  try {
    const download = promisify(execFile)(nativeBinary, ["stage-desktop-update", "--version", version], {
      windowsHide: true,
      timeout: 9 * 60 * 1000,
      maxBuffer: 64 * 1024,
    });
    updateDownloader = download.child;
    const { stdout } = await download;
    updateDownloader = null;
    if (quitting) return;
    const installer = JSON.parse(stdout.trim());
    if (typeof installer !== "string" || !installer) throw new Error("The update downloader returned no installer.");
    updateInstaller = installer;
    app.quit();
  } catch (error) {
    throw new Error(error.stderr?.trim() || error.message);
  } finally {
    updateDownloader = null;
    preparingUpdate = false;
  }
}

function startBackend() {
  const token = randomBytes(24).toString("hex");
  const env = { ...process.env, MH_DESKTOP_TOKEN: token };
  if (process.platform === "darwin") {
    env.PATH = [env.PATH, "/opt/homebrew/bin", "/usr/local/bin", join(homedir(), ".local", "bin"), join(homedir(), ".cargo", "bin")]
      .filter(Boolean).join(delimiter);
  }
  backend = spawn(nativeBinary, [
    "--desktop-child", "--no-tray", "--no-open", "--bind", "127.0.0.1:0",
  ], { stdio: ["pipe", "pipe", "pipe"], windowsHide: true, env });
  backend.stdin.on("error", () => undefined);
  backend.stderr.resume();
  backend.once("error", (error) => {
    clearTimeout(startupTimer);
    backend = null;
    showStartupError(error);
  });
  backend.once("exit", (code) => {
    backend = null;
    clearTimeout(startupTimer);
    clearTimeout(killTimer);
    if (quitting) app.quit();
    else showStartupError(new Error("The local backend exited (" + code + ")."));
  });

  startupTimer = setTimeout(() => showStartupError(new Error("The local backend did not become ready.")), 30000);
  const output = createInterface({ input: backend.stdout });
  output.on("line", (line) => {
    const match = /^MH_READY (http:\/\/127\.0\.0\.1:\d+)\/\?token=([a-f0-9]+)$/.exec(line.trim());
    if (!match || match[2] !== token || backendUrl || quitting) return;
    backendUrl = String(match[1]) + "/?token=" + token;
    clearTimeout(startupTimer);
    createWindow(backendUrl);
  });
}

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  ipcMain.handle("meta-harness:install-update", prepareDesktopUpdate);
  app.on("second-instance", openWindow);
  app.on("activate", openWindow);
  app.on("window-all-closed", () => undefined);
  app.on("before-quit", (event) => {
    quitting = true;
    updateDownloader?.kill();
    if (!backend) return;
    event.preventDefault();
    if (killTimer) return;
    backend.stdin.end();
    killTimer = setTimeout(() => backend?.kill(), 7000);
  });
  app.on("will-quit", (event) => {
    if (!updateInstaller) return;
    event.preventDefault();
    const installer = updateInstaller;
    updateInstaller = null;
    void shell.openPath(installer).then((error) => {
      if (error) dialog.showErrorBox("Could not open the update installer", error);
    }).catch((error) => {
      dialog.showErrorBox("Could not open the update installer", error.message);
    }).finally(() => app.quit());
  });
  void app.whenReady().then(() => {
    if (windows) app.setAppUserModelId("com.pan2013e.meta-harness");
    tray = new Tray(icon.resize({ width: process.platform === "darwin" ? 18 : 24 }));
    tray.setToolTip("Meta-Harness");
    tray.setContextMenu(Menu.buildFromTemplate([
      { label: "Open Meta-Harness", click: openWindow },
      { type: "separator" },
      { label: "Quit Meta-Harness", click: () => app.quit() },
    ]));
    tray.on("click", openWindow);
    startBackend();
  }).catch(showStartupError);
}
