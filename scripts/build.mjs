import { copyFileSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const windows = process.platform === "win32";
const options = new Set(process.argv.slice(2));
const store = options.has("--store");
if (options.has("--help")) {
  console.log("Usage: node scripts/build.mjs [--cli-only | --unpacked | --store]");
  process.exit(0);
}
for (const arg of options) {
  if (!["--cli-only", "--unpacked", "--store"].includes(arg)) throw new Error("Unknown build option: " + arg);
}
if (options.size > 1) {
  throw new Error("Choose one of --cli-only, --unpacked, or --store.");
}
if (store && !windows) throw new Error("Build the Microsoft Store package on Windows.");

const storeIdentity = store ? {
  identityName: process.env.MH_STORE_IDENTITY_NAME?.trim(),
  publisher: process.env.MH_STORE_PUBLISHER?.trim(),
  publisherDisplayName: process.env.MH_STORE_PUBLISHER_DISPLAY_NAME?.trim(),
} : null;
if (storeIdentity && (!/^[a-z0-9.-]{3,50}$/i.test(storeIdentity.identityName ?? "") ||
    !storeIdentity.publisher || !storeIdentity.publisherDisplayName)) {
  throw new Error("Set MH_STORE_IDENTITY_NAME, MH_STORE_PUBLISHER, and MH_STORE_PUBLISHER_DISPLAY_NAME from Partner Center's Product identity page.");
}

function run(program, args, cwd = root, capture = false, env = process.env) {
  const stdio = capture ? ["ignore", "pipe", "inherit"] : "inherit";
  const result = spawnSync(program, args, { cwd, env, encoding: "utf8", stdio, maxBuffer: 16 * 1024 * 1024, windowsHide: true });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(program + " exited with " + (result.signal ?? result.status));
  return result.stdout;
}

function npm(args, folder) {
  const cwd = join(root, folder);
  if (windows) run("cmd.exe", ["/d", "/c", "npm", ...args], cwd);
  else run("npm", args, cwd);
}

function prepareDesktop(backend, binary) {
  const desktop = join(root, "desktop");
  const staging = join(desktop, "staging");
  mkdirSync(staging, { recursive: true });
  copyFileSync(backend, join(staging, binary));
  npm(["ci"], "desktop");
  return desktop;
}

function notices(rustHost) {
  const sections = [];
  const thirdParty = (name, version, license, folder) => {
    sections.push(name + " " + version + " (" + (license ?? "see package") + ")");
    for (const item of readdirSync(folder, { withFileTypes: true })) {
      if (item.isFile() && /^(license|licence|copying|copyright|notice|third.?party.?notices)([._-]|$)/i.test(item.name)) {
        sections.push(item.name + "\n" + readFileSync(join(folder, item.name), "utf8").trim());
      }
    }
  };
  const cargo = JSON.parse(run("cargo", ["metadata", "--format-version", "1", "--locked", "--filter-platform", rustHost], root, true));
  for (const source of cargo.packages.filter((pkg) => pkg.source)) {
    thirdParty(source.name, source.version, source.license, dirname(source.manifest_path));
  }
  const lock = JSON.parse(readFileSync(join(root, "web", "package-lock.json"), "utf8"));
  for (const [path, pkg] of Object.entries(lock.packages)) {
    if (!path || pkg.dev) continue;
    const folder = join(root, "web", path);
    try {
      const meta = JSON.parse(readFileSync(join(folder, "package.json"), "utf8"));
      thirdParty(meta.name, meta.version, meta.license, folder);
    } catch (error) {
      if (error.code !== "ENOENT" || !pkg.optional) throw error;
    }
  }
  writeFileSync(join(root, "web", "public", "third-party-licenses.txt"),
    "Meta-Harness third-party notices\n\n" + sections.join("\n\n") + "\n");
}

run(process.execPath, ["scripts/version.mjs", "--check"]);
const rustHost = /^host: (.+)$/m.exec(run("rustc", ["-vV"], root, true))?.[1].trim();
if (!rustHost) throw new Error("Could not identify the Rust host target.");
const rustArch = { x86_64: "x64", aarch64: "arm64" }[rustHost.split("-")[0]];
const platform = { win32: "windows", darwin: "macos", linux: "linux" }[process.platform];
if (!rustArch || !platform || rustArch !== process.arch) {
  throw new Error("Use matching native Node.js and Rust installations for x64 or arm64 release builds.");
}
npm(["ci"], "web");
notices(rustHost);
npm(["run", "build"], "web");

const target = join(root, "target", "release-build");
const binary = windows ? "meta-harness.exe" : "meta-harness";
run("cargo", ["build", "--locked", "--release", "--bin", "meta-harness", "--target", rustHost, "--target-dir", target], root, false,
  { ...process.env, MH_RELEASE_BUILD: store ? "0" : "1" });
const backend = join(target, rustHost, "release", binary);
if (storeIdentity) {
  const desktop = prepareDesktop(backend, binary);
  const storeEnv = { ...process.env, CSC_IDENTITY_AUTO_DISCOVERY: "false" };
  delete storeEnv.CSC_LINK;
  delete storeEnv.WIN_CSC_LINK;
  run(process.execPath, [join(desktop, "node_modules", "electron-builder", "cli.js"),
    "--win", "appx", `--${rustArch}`, "--publish", "never", "--config.directories.output=../dist/store",
    ...Object.entries(storeIdentity).map(([key, value]) => `--config.appx.${key}=${value}`),
  ], desktop, false, storeEnv);
  console.log("Microsoft Store package written to " + join(root, "dist", "store"));
  process.exit(0);
}
const cli = join(root, "dist", "cli");
mkdirSync(cli, { recursive: true });
copyFileSync(backend, join(cli, binary));
copyFileSync(join(root, "LICENSE"), join(cli, "LICENSE.txt"));
copyFileSync(join(root, "web", "public", "third-party-licenses.txt"), join(cli, "third-party-licenses.txt"));
console.log("CLI release written to " + cli);

const releaseAssets = join(root, "dist", "release-assets");
mkdirSync(releaseAssets, { recursive: true });
const cliAsset = `meta-harness-cli-${platform}-${rustArch}${windows ? ".exe" : ""}`;
copyFileSync(join(cli, binary), join(releaseAssets, cliAsset));

if (!options.has("--cli-only")) {
  prepareDesktop(backend, binary);
  npm(["run", options.has("--unpacked") ? "pack" : "release"], "desktop");
  console.log("Desktop release written to " + join(root, "dist", "desktop"));
  if (!options.has("--unpacked") && process.platform === "darwin") {
    const installer = `meta-harness-desktop-${platform}-${rustArch}.dmg`;
    copyFileSync(join(root, "dist", "desktop", installer), join(releaseAssets, installer));
  }
}
