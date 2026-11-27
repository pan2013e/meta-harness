const { app, BrowserWindow, Menu, Tray, clipboard, dialog, ipcMain, nativeImage, shell } = require("electron");
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
const titleBarHeight = 36;
const startupWindowColor = "#0e1219";
const privilegeRefusalCode = 77; // Shared with the native launcher.
const privilegeRefusal = "Run Meta-Harness as a standard user, without root or administrator privileges.";
const resources = app.isPackaged ? process.resourcesPath : __dirname;
const nativeBinary = join(resources, app.isPackaged ? "bin" : "staging", windows ? "meta-harness.exe" : "meta-harness");
const icon = nativeImage.createFromPath(app.isPackaged ? join(resources, "icon.png") : join(__dirname, "..", "assets", "icon.png"));

function openWindow() {
  if (!window) return;
  window.show();
  window.focus();
}

function startTray() {
  tray = new Tray(icon.resize({ width: process.platform === "darwin" ? 18 : 24 }));
  tray.setToolTip("Meta-Harness");
  tray.setContextMenu(Menu.buildFromTemplate([
    { label: "Open Meta-Harness", click: openWindow },
    { type: "separator" },
    { label: "Quit Meta-Harness", click: () => app.quit() },
  ]));
  tray.on("click", openWindow);
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
    backgroundColor: startupWindowColor,
    ...(windows && {
      titleBarStyle: "hidden",
      titleBarOverlay: { color: startupWindowColor, symbolColor: "#f1f5f9", height: titleBarHeight },
    }),
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

function fromMainWindow(event) {
  const frame = event.senderFrame;
  return window && !window.isDestroyed() && event.sender === window.webContents &&
    frame && !frame.parent && backendUrl && new URL(frame.url).origin === new URL(backendUrl).origin;
}

function showTextMenu(event, kind, selection, canPaste) {
  if (!fromMainWindow(event) || typeof selection !== "string" || !["message", "composer", "terminal"].includes(kind)) return null;
  const isMessage = kind === "message";
  if (isMessage && !selection.trim()) return null;

  return new Promise((resolve) => {
    const copy = {
      label: "Copy",
      enabled: selection.length > 0,
      click: () => {
        clipboard.writeText(selection);
        resolve(null);
      },
    };
    const action = isMessage ? {
      label: "Quote in composer",
      click: () => resolve({ type: "quote" }),
    } : {
      label: "Paste",
      enabled: canPaste === true,
      click: () => {
        if (kind === "composer") {
          const image = clipboard.readImage();
          if (!image.isEmpty()) {
            const png = image.toPNG();
            resolve(png.length > 5 * 1024 * 1024
              ? { type: "paste_error", message: "Each image must be at most 5 MiB." }
              : { type: "paste_image", data: png.toString("base64") });
            return;
          }
        }
        resolve({ type: "paste", text: clipboard.readText() });
      },
    };
    Menu.buildFromTemplate([copy, action]).popup({
      window,
      // A dismissed menu has no action. Let a menu item's click settle first.
      callback: () => setImmediate(() => resolve(null)),
    });
  });
}

function setTitleBarColors(event, color, symbolColor) {
  const hexColor = (value) => {
    if (typeof value !== "string" || !/^#(?:[\da-f]{3}|[\da-f]{6})$/i.test(value)) return null;
    return value.length === 4 ? "#" + value.slice(1).replace(/./g, (digit) => digit.repeat(2)) : value;
  };
  const background = hexColor(color);
  const foreground = hexColor(symbolColor);
  if (windows && fromMainWindow(event) && background && foreground) {
    window.setTitleBarOverlay({ color: background, symbolColor: foreground, height: titleBarHeight });
  }
}

async function prepareDesktopUpdate(event, version) {
  if (!app.isPackaged || !fromMainWindow(event)) {
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
    else showStartupError(new Error(code === privilegeRefusalCode ? privilegeRefusal : "The local backend exited (" + code + ")."));
  });

  startupTimer = setTimeout(() => showStartupError(new Error("The local backend did not become ready.")), 30000);
  const output = createInterface({ input: backend.stdout });
  output.on("line", (line) => {
    const match = /^MH_READY (http:\/\/127\.0\.0\.1:\d+)\/#token=([a-f0-9]+)$/.exec(line.trim());
    if (!match || match[2] !== token || backendUrl || quitting) return;
    backendUrl = String(match[1]) + "/#token=" + token;
    clearTimeout(startupTimer);
    try {
      startTray();
      createWindow(backendUrl);
    } catch (error) {
      showStartupError(error);
    }
  });
}

if (process.getuid?.() === 0 || process.geteuid?.() === 0) {
  dialog.showErrorBox("Meta-Harness could not start", privilegeRefusal);
  app.exit(privilegeRefusalCode);
} else if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  ipcMain.handle("meta-harness:install-update", prepareDesktopUpdate);
  ipcMain.handle("meta-harness:text-menu", showTextMenu);
  ipcMain.handle("meta-harness:title-bar-colors", setTitleBarColors);
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
    startBackend();
  }).catch(showStartupError);
}
